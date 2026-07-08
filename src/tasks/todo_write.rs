//! Tasks Module — agent self-tracking types (s03).
//!
//! `TodoState` / `TodoItem` are session-scoped checklist types used by the
//! daemon `/todos` endpoint. The former `TodoWriteTool` has been removed;
//! task tracking is now unified under the `update_plan` tool.

use serde::{Deserialize, Serialize};

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
