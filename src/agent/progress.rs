//! Subagent progress types for real-time execution visibility.
//!
//! These types are standalone — they do NOT depend on AppEvent or TUI types.
//! The subagent loop emits `SubagentProgress` events through an optional
//! `ProgressCallback`. The daemon stores them in a shared store; the TUI polls
//! the store and converts updates into `AppEvent::SubagentUpdate` for rendering.

use crate::api::ChatMessage;
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
    /// Full text stored here — TUI layer truncates for display.
    Thought { text: String },
    /// The model called a tool.
    Action {
        tool_name: String,
        params_summary: String,
    },
    /// A tool execution result.
    ToolResult {
        tool_name: String,
        success: bool,
        summary: String,
    },
    /// An error occurred.
    Error {
        message: String,
        error_type: ErrorType,
    },
    /// Subagent completed.
    Completion {
        status: String,
        summary: Option<String>,
    },
}

/// Categorized error types for subagent execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ErrorType {
    Timeout,
    BudgetExceeded {
        limit_k: u64,
        used: u64,
    },
    Stuck {
        reason: String,
    },
    ToolError {
        tool: String,
        message: String,
    },
    ParseError {
        message: String,
    },
    /// The subagent was cancelled via its execution context's cancellation token.
    Cancelled,
    Unknown,
}

/// Detailed error information for a failed subagent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorInfo {
    pub error_type: ErrorType,
    pub message: String,
    pub last_tool: Option<String>,
    pub last_params: Option<String>,
    pub round: u32,
    pub retryable: bool,
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
    /// Execution event timeline (earliest → latest), no truncation.
    pub action_log: Vec<SubagentEvent>,
    /// Last assistant text response (full text, TUI truncates for display).
    pub text_snapshot: Option<String>,
    /// Unix epoch timestamp in milliseconds when this subagent started.
    pub started_at: i64,
    pub elapsed_ms: u64,
    pub metadata: Option<SubagentMetadata>,

    // === New fields ===
    /// Incremental progress delta for the last round (0.0-1.0).
    pub progress_delta: Option<f32>,
    /// Token budget in thousands (0 = unlimited).
    pub token_budget_k: Option<u64>,
    /// Cumulative tokens used so far.
    pub cumulative_tokens: u64,
    /// Error details when status is Failed or Cancelled.
    pub error_details: Option<ErrorInfo>,
    /// Full event stream (replaces old action_log for new code, kept for compat).
    pub events: Vec<SubagentEvent>,
    /// Full conversation messages from the subagent's loop,
    /// for rendering the focus view as a chat history.
    pub messages: Vec<ChatMessage>,
}

/// Serializable lifecycle state reported by a subagent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SubagentStatus {
    /// Created but not yet executing.
    Pending,
    /// Actively executing model or tool work.
    Running,
    /// Waiting for child subagents to finish.
    WaitingForChildren,
    /// Producing the final result.
    Finalizing,
    /// Cancellation is in progress.
    Cancelling,
    /// Finished successfully.
    Completed,
    /// Finished with an error.
    Failed,
    /// Finished due to cancellation.
    Cancelled,
}

impl SubagentStatus {
    /// Returns whether the subagent has finished and requires no further work.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentMetadata {
    pub token_count: Option<usize>,
    pub error: Option<String>,
    pub depends_on: Vec<String>,
}

pub type ProgressCallback = Arc<dyn Fn(SubagentProgress) + Send + Sync>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_subagent_event_type_thought_serialize() {
        let event = SubagentEvent {
            event_type: SubagentEventType::Thought {
                text: "hello".to_string(),
            },
            elapsed_ms: 100,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("Thought"));
        assert!(json.contains("hello"));
        let deserialized: SubagentEvent = serde_json::from_str(&json).unwrap();
        match deserialized.event_type {
            SubagentEventType::Thought { text } => assert_eq!(text, "hello"),
            _ => panic!("expected Thought"),
        }
        assert_eq!(deserialized.elapsed_ms, 100);
    }

    #[test]
    fn test_subagent_event_type_action_serialize() {
        let event = SubagentEvent {
            event_type: SubagentEventType::Action {
                tool_name: "read_file".to_string(),
                params_summary: "src/main.rs".to_string(),
            },
            elapsed_ms: 200,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("Action"));
        assert!(json.contains("read_file"));
        let deserialized: SubagentEvent = serde_json::from_str(&json).unwrap();
        match deserialized.event_type {
            SubagentEventType::Action {
                tool_name,
                params_summary,
            } => {
                assert_eq!(tool_name, "read_file");
                assert_eq!(params_summary, "src/main.rs");
            }
            _ => panic!("expected Action"),
        }
    }

    #[test]
    fn test_subagent_event_type_tool_result_serialize() {
        let event = SubagentEvent {
            event_type: SubagentEventType::ToolResult {
                tool_name: "read_file".to_string(),
                success: true,
                summary: "file read successfully".to_string(),
            },
            elapsed_ms: 300,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("ToolResult"));
        assert!(json.contains("read_file"));
        assert!(json.contains("true"));
        let deserialized: SubagentEvent = serde_json::from_str(&json).unwrap();
        match deserialized.event_type {
            SubagentEventType::ToolResult {
                tool_name,
                success,
                summary,
            } => {
                assert_eq!(tool_name, "read_file");
                assert!(success);
                assert_eq!(summary, "file read successfully");
            }
            _ => panic!("expected ToolResult"),
        }
    }

    #[test]
    fn test_subagent_event_type_error_serialize() {
        let event = SubagentEvent {
            event_type: SubagentEventType::Error {
                message: "timeout occurred".to_string(),
                error_type: ErrorType::Timeout,
            },
            elapsed_ms: 400,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("Error"));
        assert!(json.contains("Timeout"));
        let deserialized: SubagentEvent = serde_json::from_str(&json).unwrap();
        match deserialized.event_type {
            SubagentEventType::Error {
                message,
                error_type,
            } => {
                assert_eq!(message, "timeout occurred");
                assert!(matches!(error_type, ErrorType::Timeout));
            }
            _ => panic!("expected Error"),
        }
    }

    #[test]
    fn test_subagent_event_type_error_budget_exceeded() {
        let event = SubagentEvent {
            event_type: SubagentEventType::Error {
                message: "budget exceeded".to_string(),
                error_type: ErrorType::BudgetExceeded {
                    limit_k: 10,
                    used: 15,
                },
            },
            elapsed_ms: 500,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("BudgetExceeded"));
        assert!(json.contains("10"));
        assert!(json.contains("15"));
        let deserialized: SubagentEvent = serde_json::from_str(&json).unwrap();
        match deserialized.event_type {
            SubagentEventType::Error {
                error_type: ErrorType::BudgetExceeded { limit_k, used },
                ..
            } => {
                assert_eq!(limit_k, 10);
                assert_eq!(used, 15);
            }
            _ => panic!("expected BudgetExceeded"),
        }
    }

    #[test]
    fn test_subagent_event_type_completion_serialize() {
        let event = SubagentEvent {
            event_type: SubagentEventType::Completion {
                status: "completed".to_string(),
                summary: Some("all done".to_string()),
            },
            elapsed_ms: 600,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("Completion"));
        assert!(json.contains("completed"));
        assert!(json.contains("all done"));
        let deserialized: SubagentEvent = serde_json::from_str(&json).unwrap();
        match deserialized.event_type {
            SubagentEventType::Completion { status, summary } => {
                assert_eq!(status, "completed");
                assert_eq!(summary, Some("all done".to_string()));
            }
            _ => panic!("expected Completion"),
        }
    }

    #[test]
    fn test_subagent_event_type_completion_no_summary() {
        let event = SubagentEvent {
            event_type: SubagentEventType::Completion {
                status: "failed".to_string(),
                summary: None,
            },
            elapsed_ms: 700,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("failed"));
        // summary should be null
        assert!(json.contains("\"summary\":null") || !json.contains("all done"));
        let deserialized: SubagentEvent = serde_json::from_str(&json).unwrap();
        match deserialized.event_type {
            SubagentEventType::Completion { status, summary } => {
                assert_eq!(status, "failed");
                assert_eq!(summary, None);
            }
            _ => panic!("expected Completion"),
        }
    }

    #[test]
    fn test_error_type_variants() {
        assert!(matches!(ErrorType::Timeout, ErrorType::Timeout));
        assert!(matches!(ErrorType::Unknown, ErrorType::Unknown));
        assert!(matches!(
            ErrorType::BudgetExceeded {
                limit_k: 5,
                used: 10
            },
            ErrorType::BudgetExceeded { .. }
        ));
        assert!(matches!(
            ErrorType::Stuck {
                reason: String::new()
            },
            ErrorType::Stuck { .. }
        ));
        assert!(matches!(
            ErrorType::ToolError {
                tool: String::new(),
                message: String::new()
            },
            ErrorType::ToolError { .. }
        ));
        assert!(matches!(
            ErrorType::ParseError {
                message: String::new()
            },
            ErrorType::ParseError { .. }
        ));
    }

    #[test]
    fn test_error_type_serialization() {
        // Test Timeout
        let json = serde_json::to_string(&ErrorType::Timeout).unwrap();
        assert_eq!(json, "\"Timeout\"");

        // Test Unknown
        let json = serde_json::to_string(&ErrorType::Unknown).unwrap();
        assert_eq!(json, "\"Unknown\"");

        // Test BudgetExceeded
        let be = ErrorType::BudgetExceeded {
            limit_k: 10,
            used: 15,
        };
        let json = serde_json::to_string(&be).unwrap();
        assert!(json.contains("BudgetExceeded"));
        let deserialized: ErrorType = serde_json::from_str(&json).unwrap();
        match deserialized {
            ErrorType::BudgetExceeded { limit_k, used } => {
                assert_eq!(limit_k, 10);
                assert_eq!(used, 15);
            }
            _ => panic!("expected BudgetExceeded"),
        }
    }

    #[test]
    fn test_error_info_struct() {
        let info = ErrorInfo {
            error_type: ErrorType::Timeout,
            message: "timed out after 30s".to_string(),
            last_tool: Some("read_file".to_string()),
            last_params: Some("src/main.rs".to_string()),
            round: 5,
            retryable: true,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("Timeout"));
        assert!(json.contains("timed out after 30s"));
        assert!(json.contains("read_file"));
        assert!(json.contains("5"));
        assert!(json.contains("true"));

        let deserialized: ErrorInfo = serde_json::from_str(&json).unwrap();
        assert!(matches!(deserialized.error_type, ErrorType::Timeout));
        assert_eq!(deserialized.message, "timed out after 30s");
        assert_eq!(deserialized.last_tool, Some("read_file".to_string()));
        assert_eq!(deserialized.last_params, Some("src/main.rs".to_string()));
        assert_eq!(deserialized.round, 5);
        assert!(deserialized.retryable);
    }

    #[test]
    fn test_error_info_minimal() {
        let info = ErrorInfo {
            error_type: ErrorType::Unknown,
            message: "something went wrong".to_string(),
            last_tool: None,
            last_params: None,
            round: 0,
            retryable: false,
        };
        let json = serde_json::to_string(&info).unwrap();
        let deserialized: ErrorInfo = serde_json::from_str(&json).unwrap();
        assert!(matches!(deserialized.error_type, ErrorType::Unknown));
        assert_eq!(deserialized.round, 0);
        assert!(!deserialized.retryable);
        assert!(deserialized.last_tool.is_none());
        assert!(deserialized.last_params.is_none());
    }

    #[test]
    fn test_subagent_progress_new_fields() {
        let progress = SubagentProgress {
            node_id: "node1".to_string(),
            parent_id: None,
            label: "test".to_string(),
            status: SubagentStatus::Running,
            round: Some(1),
            max_rounds: Some(10),
            current_tool: Some("read_file".to_string()),
            current_params: Some("src/main.rs".to_string()),
            action_log: vec![],
            text_snapshot: Some("working".to_string()),
            started_at: 1000,
            elapsed_ms: 500,
            metadata: None,
            progress_delta: Some(0.5),
            token_budget_k: Some(10),
            cumulative_tokens: 5000,
            error_details: None,
            events: vec![],
            messages: vec![],
        };
        let json = serde_json::to_string(&progress).unwrap();
        assert!(json.contains("progress_delta"));
        assert!(json.contains("token_budget_k"));
        assert!(json.contains("cumulative_tokens"));
        assert!(json.contains("error_details"));
        assert!(json.contains("events"));
        assert!(json.contains("0.5"));
        assert!(json.contains("10"));
        assert!(json.contains("5000"));

        let deserialized: SubagentProgress = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.progress_delta, Some(0.5));
        assert_eq!(deserialized.token_budget_k, Some(10));
        assert_eq!(deserialized.cumulative_tokens, 5000);
        assert!(deserialized.error_details.is_none());
        assert!(deserialized.events.is_empty());
    }

    #[test]
    fn test_subagent_progress_new_fields_defaults() {
        let progress = SubagentProgress {
            node_id: "node1".to_string(),
            parent_id: None,
            label: "test".to_string(),
            status: SubagentStatus::Running,
            round: Some(1),
            max_rounds: Some(10),
            current_tool: None,
            current_params: None,
            action_log: vec![],
            text_snapshot: None,
            started_at: 1000,
            elapsed_ms: 500,
            metadata: None,
            progress_delta: None,
            token_budget_k: None,
            cumulative_tokens: 0,
            error_details: None,
            events: vec![],
            messages: vec![],
        };
        let json = serde_json::to_string(&progress).unwrap();
        let deserialized: SubagentProgress = serde_json::from_str(&json).unwrap();
        assert!(deserialized.progress_delta.is_none());
        assert!(deserialized.token_budget_k.is_none());
        assert_eq!(deserialized.cumulative_tokens, 0);
        assert!(deserialized.error_details.is_none());
        assert!(deserialized.events.is_empty());
    }

    #[test]
    fn test_subagent_status_variants() {
        assert_eq!(SubagentStatus::Pending, SubagentStatus::Pending);
        assert_eq!(SubagentStatus::Running, SubagentStatus::Running);
        assert_eq!(SubagentStatus::Completed, SubagentStatus::Completed);
        assert_eq!(SubagentStatus::Failed, SubagentStatus::Failed);
        assert_eq!(SubagentStatus::Cancelled, SubagentStatus::Cancelled);
        assert_ne!(SubagentStatus::Pending, SubagentStatus::Running);
    }

    #[test]
    fn test_subagent_status_terminal_semantics() {
        for status in [
            SubagentStatus::Pending,
            SubagentStatus::Running,
            SubagentStatus::WaitingForChildren,
            SubagentStatus::Finalizing,
            SubagentStatus::Cancelling,
        ] {
            assert!(!status.is_terminal(), "{status:?} must remain active");
        }

        for status in [
            SubagentStatus::Completed,
            SubagentStatus::Failed,
            SubagentStatus::Cancelled,
        ] {
            assert!(status.is_terminal(), "{status:?} must be terminal");
        }
    }

    // ── contract tests for error_details population ──

    /// Verify the ErrorInfo construction formula used in subagent_loop and task.rs/pipeline.rs.
    #[test]
    fn test_error_details_construction_formula() {
        let error_msg = Some("Token budget exceeded: limit 10k, used 15k".to_string());
        let current_tool = Some("read_file".to_string());
        let current_params = Some("src/main.rs".to_string());
        let round = 5usize;

        let details = error_msg.as_ref().map(|msg| ErrorInfo {
            error_type: ErrorType::Unknown,
            message: msg.clone(),
            last_tool: current_tool.clone(),
            last_params: current_params.clone(),
            round: round as u32,
            retryable: true,
        });

        assert!(
            details.is_some(),
            "error_details should be Some when error_msg is Some"
        );
        let d = details.unwrap();
        assert!(matches!(d.error_type, ErrorType::Unknown));
        assert_eq!(d.message, "Token budget exceeded: limit 10k, used 15k");
        assert_eq!(d.last_tool, Some("read_file".to_string()));
        assert_eq!(d.last_params, Some("src/main.rs".to_string()));
        assert_eq!(d.round, 5);
        assert!(d.retryable);
    }

    #[test]
    fn test_error_details_none_when_no_error() {
        let error_msg: Option<String> = None;
        let current_tool: Option<String> = None;
        let current_params: Option<String> = None;

        let details = error_msg.as_ref().map(|msg| ErrorInfo {
            error_type: ErrorType::Unknown,
            message: msg.clone(),
            last_tool: current_tool.clone(),
            last_params: current_params.clone(),
            round: 0,
            retryable: true,
        });

        assert!(
            details.is_none(),
            "error_details should be None when error_msg is None"
        );
    }

    // ── progress_delta construction formula test ──

    #[test]
    fn test_progress_delta_computed_during_running_emit() {
        // Simulate the delta computation pattern from subagent_loop.rs:
        // tool_types_used tracks all tool types seen so far.
        // After a round with tools, the delta is computed and should be >0 if new types seen.
        let mut tool_types_used: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        let round_tool_names = vec!["read_file".to_string(), "grep".to_string()];
        let round_tool_types: std::collections::HashSet<String> =
            round_tool_names.into_iter().collect();
        let new_types: Vec<&String> = round_tool_types.difference(&tool_types_used).collect();
        let delta = if tool_types_used.is_empty() {
            1.0f32
        } else {
            new_types.len() as f32 / tool_types_used.len() as f32
        };
        tool_types_used.extend(round_tool_types);

        // The delta should be 1.0 for the first round (new types / empty = 1.0)
        assert_eq!(delta, 1.0f32, "First round should have delta=1.0");

        // After the second round with the same tool types, delta should be 0.0
        let round_tool_names2 = vec!["read_file".to_string(), "grep".to_string()];
        let round_tool_types2: std::collections::HashSet<String> =
            round_tool_names2.into_iter().collect();
        let new_types2: Vec<&String> = round_tool_types2.difference(&tool_types_used).collect();
        let delta2 = if tool_types_used.is_empty() {
            1.0f32
        } else {
            new_types2.len() as f32 / tool_types_used.len() as f32
        };
        assert_eq!(delta2, 0.0f32, "Repeated tool types should yield delta=0.0");
    }
}
