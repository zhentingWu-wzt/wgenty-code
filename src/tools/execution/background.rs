//! Background task management — run shell commands in the background
//! and receive their results via a notification queue that gets injected
//! into the agent loop on the next iteration.

use crate::sandbox::shell_command_captured;
use crate::tools::{Tool, ToolError, ToolOutput};
use async_trait::async_trait;
use serde::Serialize;
use std::sync::Arc;
use tokio::sync::Mutex;

/// A completed background task result
#[derive(Debug, Clone, Serialize)]
pub struct BackgroundResult {
    pub task_id: String,
    pub result_type: String,
    pub command: String,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub success: bool,
}

/// Manages background command execution and notification delivery.
pub struct BackgroundManager {
    results: Arc<Mutex<Vec<BackgroundResult>>>,
    next_id: Arc<Mutex<u64>>,
}

impl BackgroundManager {
    pub fn new() -> Self {
        Self {
            results: Arc::new(Mutex::new(Vec::new())),
            next_id: Arc::new(Mutex::new(1)),
        }
    }

    /// Spawn a command in the background. Returns the task_id immediately.
    pub async fn spawn(&self, command: &str, timeout_secs: u64) -> String {
        let mut id_lock = self.next_id.lock().await;
        let task_id = format!("bg_{}", *id_lock);
        *id_lock += 1;
        drop(id_lock);

        let task_id_clone = task_id.clone();
        let command_clone = command.to_string();
        let results = self.results.clone();

        tokio::spawn(async move {
            let timeout = std::time::Duration::from_secs(timeout_secs.max(300));
            // Clone before the inner async block so command_clone remains available
            // for the fallback after the move into the timeout closure.
            let cmd_inner = command_clone.clone();
            let result = tokio::time::timeout(timeout, async {
                // Platform shell + captured stdio (CREATE_NO_WINDOW on Windows).
                let output = shell_command_captured(&cmd_inner).output().await;

                match output {
                    Ok(out) => BackgroundResult {
                        task_id: task_id_clone.clone(),
                        result_type: "command".to_string(),
                        command: cmd_inner.clone(),
                        stdout: String::from_utf8_lossy(&out.stdout).to_string(),
                        stderr: String::from_utf8_lossy(&out.stderr).to_string(),
                        exit_code: out.status.code(),
                        success: out.status.success(),
                    },
                    Err(e) => BackgroundResult {
                        task_id: task_id_clone.clone(),
                        result_type: "command".to_string(),
                        command: cmd_inner,
                        stdout: String::new(),
                        stderr: format!("Failed to execute: {}", e),
                        exit_code: None,
                        success: false,
                    },
                }
            })
            .await;

            let final_result = result.unwrap_or(BackgroundResult {
                task_id: task_id_clone,
                result_type: "command".to_string(),
                command: command_clone,
                stdout: String::new(),
                stderr: "Command timed out".to_string(),
                exit_code: None,
                success: false,
            });

            let mut results = results.lock().await;
            results.push(final_result);
        });

        task_id
    }

    /// Push a subagent result into the completed queue (for background subagents).
    pub async fn push_subagent_result(&self, description: &str, stdout: &str, success: bool) {
        let mut id_lock = self.next_id.lock().await;
        let task_id = format!("subagent_{}", *id_lock);
        *id_lock += 1;
        drop(id_lock);

        let mut results = self.results.lock().await;
        results.push(BackgroundResult {
            task_id,
            result_type: "subagent".to_string(),
            command: description.to_string(),
            stdout: stdout.to_string(),
            stderr: String::new(),
            exit_code: if success { Some(0) } else { Some(1) },
            success,
        });
    }

    /// Drain all completed results from the queue.
    pub async fn drain_results(&self) -> Vec<BackgroundResult> {
        let mut results = self.results.lock().await;
        std::mem::take(&mut *results)
    }

    /// Check if any results are ready.
    pub async fn has_results(&self) -> bool {
        let results = self.results.lock().await;
        !results.is_empty()
    }

    /// Format results as a notification message for injection into the agent loop.
    pub fn format_notification(results: &[BackgroundResult]) -> Option<String> {
        if results.is_empty() {
            return None;
        }

        let mut lines = vec!["## Background Task Results\n".to_string()];
        for r in results {
            let status = if r.success { "SUCCESS" } else { "FAILED" };
            let output = if r.success { &r.stdout } else { &r.stderr };
            // Truncate to 2000 chars
            lines.push(format!(
                "### {} [{}] (exit: {:?})\n```\n{}\n```",
                r.task_id, status, r.exit_code, output
            ));
        }
        Some(lines.join("\n"))
    }

    /// Store a subagent result for later retrieval.
    pub async fn store_subagent_result(
        &self,
        task_id: impl Into<String>,
        result: impl Into<String>,
    ) {
        let mut results = self.results.lock().await;
        results.push(BackgroundResult {
            task_id: task_id.into(),
            result_type: "subagent".to_string(),
            command: String::new(),
            stdout: result.into(),
            stderr: String::new(),
            exit_code: Some(0),
            success: true,
        });
    }

    /// Generate a unique task ID for a background subagent.
    pub async fn next_task_id(&self) -> String {
        let mut id_lock = self.next_id.lock().await;
        let task_id = format!("bg_{}", *id_lock);
        *id_lock += 1;
        task_id
    }
}

impl Default for BackgroundManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Tool for running shell commands in the background.
pub struct BackgroundTool {
    manager: Arc<BackgroundManager>,
}

impl BackgroundTool {
    pub fn new(manager: Arc<BackgroundManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for BackgroundTool {
    fn name(&self) -> &str {
        "background"
    }

    fn description(&self) -> &str {
        "Run a shell command in the background. Returns a task_id immediately. \
         Results will be delivered when the command completes. \
         Use for long-running commands where you don't need the result right away."
    }

    fn is_read_only(&self) -> bool {
        false
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The shell command to run in the background"
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Max execution time in seconds (default 300)",
                    "default": 300
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let command = input["command"].as_str().unwrap_or("");
        let timeout = input["timeout_secs"].as_u64().unwrap_or(300);

        let task_id = self.manager.spawn(command, timeout).await;

        Ok(ToolOutput {
            output_type: "json".to_string(),
            content: serde_json::json!({
                "success": true,
                "task_id": task_id,
                "message": format!(
                    "Background task '{}' started. Results will be delivered when ready.",
                    task_id
                )
            })
            .to_string(),
            metadata: std::collections::HashMap::new(),
        })
    }
}
