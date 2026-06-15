//! Subagent transcript persistence module.
//!
//! Stores subagent execution transcripts in SQLite for later review,
//! debugging, and rollback scenarios.

pub mod store;

pub use store::SubagentTranscriptStore;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentTranscript {
    pub id: String,                         // UUID v4
    pub session_id: String,
    pub parent_id: Option<String>,
    pub label: String,
    pub status: TranscriptStatus,
    pub system_prompt: Option<String>,
    pub user_prompt: String,
    pub started_at: i64,                    // Unix ms
    pub finished_at: Option<i64>,
    pub total_tokens: u64,
    pub max_rounds: Option<u32>,
    pub actual_rounds: u32,
    pub token_budget_k: Option<u64>,
    pub error_message: Option<String>,
    pub summary: Option<String>,
    pub events: Vec<SubagentEventRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TranscriptStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl std::fmt::Display for TranscriptStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Running => write!(f, "running"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentEventRecord {
    pub round: u32,
    pub event_type: String,            // thought | action | tool_result | error | completion
    pub tool_name: Option<String>,
    pub tool_params: Option<serde_json::Value>,
    pub data: String,
    pub elapsed_ms: u64,
    pub token_count: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentTranscriptHeader {
    pub id: String,
    pub session_id: String,
    pub parent_id: Option<String>,
    pub label: String,
    pub status: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub total_tokens: u64,
    pub actual_rounds: u32,
    pub error_message: Option<String>,
    pub summary: Option<String>,
}

#[derive(Debug, Clone)]
pub enum TranscriptError {
    Database(String),
    NotFound(String),
    Io(String),
}

impl std::fmt::Display for TranscriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Database(msg) => write!(f, "Database error: {}", msg),
            Self::NotFound(id) => write!(f, "Transcript not found: {}", id),
            Self::Io(msg) => write!(f, "IO error: {}", msg),
        }
    }
}

impl std::error::Error for TranscriptError {}

impl From<rusqlite::Error> for TranscriptError {
    fn from(e: rusqlite::Error) -> Self {
        TranscriptError::Database(e.to_string())
    }
}
