//! Subagent progress types for real-time execution visibility.
//!
//! These types are standalone — they do NOT depend on AppEvent or TUI types.
//! The subagent loop emits `SubagentProgress` events through an optional
//! `ProgressCallback`. The daemon stores them in a shared store; the TUI polls
//! the store and converts updates into `AppEvent::SubagentUpdate` for rendering.

use serde::{Deserialize, Serialize};
use std::sync::Arc;

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
