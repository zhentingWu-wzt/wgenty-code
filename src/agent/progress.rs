//! Subagent progress types for real-time execution visibility.
//!
//! These types are standalone — they do NOT depend on AppEvent or TUI types.
//! The subagent loop emits `SubagentProgress` events through an optional
//! `ProgressCallback`. The daemon stores them in a shared store; the TUI polls
//! the store and converts updates into `AppEvent::SubagentUpdate` for rendering.

use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// An event in a subagent's execution timeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentEvent {
    pub event_type: SubagentEventType,
    /// Milliseconds elapsed since subagent started when this event occurred.
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SubagentEventType {
    /// The model output text (analysis, planning, conclusion).
    /// Text is truncated to 200 chars before storage.
    Thought { text: String },
    /// The model called a tool.
    Action {
        tool_name: String,
        params_summary: String,
    },
}

/// A progress update emitted by a subagent at key lifecycle points.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentProgress {
    pub node_id: String,
    pub parent_id: Option<String>,
    pub label: String,
    pub status: SubagentStatus,
    pub round: Option<usize>,
    pub max_rounds: Option<usize>,
    pub current_tool: Option<String>,
    /// Human-readable summary of the current tool's key parameters.
    /// e.g., `"src/auth.rs"` when `current_tool` is `"file_read"`.
    pub current_params: Option<String>,
    /// Execution event timeline (earliest → latest), max 50 entries.
    pub action_log: Vec<SubagentEvent>,
    /// Last assistant text response (truncated to last ~200 chars).
    /// Captures what the model "said/thought" between tool calls.
    pub text_snapshot: Option<String>,
    /// Unix epoch timestamp in milliseconds when this subagent started.
    pub started_at: i64,
    pub elapsed_ms: u64,
    pub metadata: Option<SubagentMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SubagentStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentMetadata {
    pub token_count: Option<usize>,
    pub error: Option<String>,
    pub depends_on: Vec<String>,
}

pub type ProgressCallback = Arc<dyn Fn(SubagentProgress) + Send + Sync>;
