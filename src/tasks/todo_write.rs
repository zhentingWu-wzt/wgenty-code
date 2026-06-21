//! TodoWrite Tool — session-scoped agent self-tracking checklist.
//!
//! Follows the s03 design philosophy: the agent tracks its own progress via
//! a simple checklist it updates as a batch. The harness injects a nag reminder
//! when the agent forgets to update.
//!
//! Key constraints:
//! - Max 20 items (prevents list bloat)
//! - Only 1 in_progress at a time (enforces focus)
//! - Batch replace: each call replaces the entire list
//! - Session-scoped: lives in memory, not persisted to disk

use crate::tools::{Tool, ToolError, ToolOutput};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

// ── Todo item model ──────────────────────────────────────────────────────────

/// Metadata for tasks that originate from subagent executions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentTodoMeta {
    pub subagent_type: String, // "explore" | "plan" | "general-purpose" | ...
    pub token_usage: u64,
    pub rounds: u32,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItem {
    pub content: String,
    pub status: String, // "pending" | "in_progress" | "completed"
    #[serde(default)]
    #[serde(rename = "activeForm")]
    pub active_form: String, // shown when in_progress, e.g. "Fixing auth bug"
    #[serde(default)]
    pub subagent: Option<SubagentTodoMeta>,
}

// ── Shared todo state ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct TodoState {
    pub items: Vec<TodoItem>,
}

impl TodoState {
    pub fn has_open_items(&self) -> bool {
        self.items.iter().any(|t| t.status != "completed")
    }

    pub fn render(&self) -> String {
        if self.items.is_empty() {
            return "No todos.".to_string();
        }
        let mut lines: Vec<String> = Vec::new();
        for item in &self.items {
            let marker = match item.status.as_str() {
                "completed" => "[x]",
                "in_progress" => "[>]",
                _ => "[ ]",
            };
            let suffix = if item.status == "in_progress" && !item.active_form.is_empty() {
                format!(" <- {}", item.active_form)
            } else {
                String::new()
            };
            lines.push(format!("{} {}{}", marker, item.content, suffix));
        }
        let done = self
            .items
            .iter()
            .filter(|t| t.status == "completed")
            .count();
        lines.push(format!("\n({}/{})", done, self.items.len()));
        lines.join("\n")
    }
}

// ── TodoWrite tool ───────────────────────────────────────────────────────────

pub struct TodoWriteTool {
    state: Arc<RwLock<TodoState>>,
}

impl Default for TodoWriteTool {
    fn default() -> Self {
        Self::new()
    }
}

impl TodoWriteTool {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(TodoState::default())),
        }
    }

    /// Create a TodoWriteTool that shares the same todo state.
    pub fn from_arc(state: Arc<RwLock<TodoState>>) -> Self {
        Self { state }
    }

    /// Return the underlying state so it can be shared (e.g. with the API endpoint).
    pub fn todo_state(&self) -> Arc<RwLock<TodoState>> {
        self.state.clone()
    }
}

#[async_trait]
impl Tool for TodoWriteTool {
    fn name(&self) -> &str {
        "TodoWrite"
    }

    fn description(&self) -> &str {
        "Update the agent's task tracking checklist. Replaces the entire list on each call. \
         Mark the current task in_progress before starting, completed when done. \
         Only one in_progress at a time. Max 20 items."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "items": {
                    "type": "array",
                    "description": "The complete todo list (replaces existing). Max 20 items.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "content": {
                                "type": "string",
                                "description": "Task description"
                            },
                            "status": {
                                "type": "string",
                                "enum": ["pending", "in_progress", "completed"],
                                "description": "Task status"
                            },
                            "activeForm": {
                                "type": "string",
                                "description": "REQUIRED when status is in_progress. Present continuous verb form (e.g. 'Fixing auth bug', 'Writing tests'). Omit or leave empty for pending/completed items."
                            },
                            "subagent": {
                                "type": "object",
                                "description": "Metadata for tasks originating from subagent executions. Omit for regular tasks.",
                                "properties": {
                                    "subagent_type": {
                                        "type": "string",
                                        "description": "Type of subagent: explore, plan, general-purpose"
                                    },
                                    "token_usage": {
                                        "type": "integer",
                                        "description": "Total tokens consumed by the subagent"
                                    },
                                    "rounds": {
                                        "type": "integer",
                                        "description": "Number of rounds completed"
                                    },
                                    "duration_ms": {
                                        "type": "integer",
                                        "description": "Wall-clock duration in milliseconds"
                                    }
                                }
                            }
                        },
                        "required": ["content", "status"]
                    }
                }
            },
            "required": ["items"]
        })
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let raw_items = input["items"].as_array().ok_or_else(|| ToolError {
            message: "'items' array is required".to_string(),
            code: Some("missing_items".to_string()),
        })?;

        // Validate and convert
        let mut validated: Vec<TodoItem> = Vec::new();
        let mut in_progress_count = 0usize;

        for (i, item) in raw_items.iter().enumerate() {
            let content = item["content"]
                .as_str()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| ToolError {
                    message: format!("Item {}: content is required", i),
                    code: Some("missing_content".to_string()),
                })?;

            let status = item["status"]
                .as_str()
                .map(|s| s.to_lowercase())
                .ok_or_else(|| ToolError {
                    message: format!("Item {}: status is required", i),
                    code: Some("missing_status".to_string()),
                })?;

            if !["pending", "in_progress", "completed"].contains(&status.as_str()) {
                return Err(ToolError {
                    message: format!(
                        "Item {}: invalid status '{}'. Must be pending, in_progress, or completed",
                        i, status
                    ),
                    code: Some("invalid_status".to_string()),
                });
            }

            let active_form = item["activeForm"]
                .as_str()
                .map(|s| s.trim().to_string())
                .unwrap_or_default();

            if status == "in_progress" {
                in_progress_count += 1;
            }

            if status == "in_progress" && active_form.is_empty() {
                return Err(ToolError {
                    message: format!(
                        "Item {}: activeForm is required when status is in_progress",
                        i
                    ),
                    code: Some("missing_active_form".to_string()),
                });
            }

            // Parse optional subagent metadata from input
            let subagent = item.get("subagent").and_then(|s| {
                if s.is_null() {
                    return None;
                }
                Some(SubagentTodoMeta {
                    subagent_type: s["subagent_type"]
                        .as_str()
                        .unwrap_or("general-purpose")
                        .to_string(),
                    token_usage: s["token_usage"].as_u64().unwrap_or(0),
                    rounds: s["rounds"].as_u64().unwrap_or(0) as u32,
                    duration_ms: s["duration_ms"].as_u64().unwrap_or(0),
                })
            });

            validated.push(TodoItem {
                content,
                status,
                active_form: active_form.clone(),
                subagent,
            });
        }

        if validated.len() > 20 {
            return Err(ToolError {
                message: format!("Max 20 items allowed, got {}", validated.len()),
                code: Some("too_many_items".to_string()),
            });
        }

        if in_progress_count > 1 {
            return Err(ToolError {
                message: format!(
                    "Only one task can be in_progress at a time, got {}",
                    in_progress_count
                ),
                code: Some("multiple_in_progress".to_string()),
            });
        }

        // Update state
        let mut state = self.state.write().await;
        state.items = validated;
        let rendered = state.render();
        drop(state);

        Ok(ToolOutput {
            output_type: "text".to_string(),
            content: rendered,
            metadata: std::collections::HashMap::new(),
        })
    }
}
