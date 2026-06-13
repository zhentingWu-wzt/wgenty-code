//! Checkpoint/Undo tools — git stash-based rollback.
//! Creates a git stash checkpoint before risky operations; undo restores.

use crate::tools::{Tool, ToolError, ToolOutput};
use async_trait::async_trait;

pub struct CheckpointManager {
    count: std::sync::atomic::AtomicU32,
}

impl Default for CheckpointManager {
    fn default() -> Self {
        Self {
            count: std::sync::atomic::AtomicU32::new(0),
        }
    }
}

impl CheckpointManager {
    pub fn new() -> Self {
        Self {
            count: std::sync::atomic::AtomicU32::new(0),
        }
    }

    /// Create a git stash checkpoint. Uses tokio::process::Command to
    /// avoid blocking the async runtime during the git operation.
    pub async fn create(&self, description: &str) -> Result<String, String> {
        let n = self.count.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        let msg = format!("wgenty-checkpoint-{}: {}", n, description);
        let output = tokio::process::Command::new("git")
            .args(["stash", "push", "--include-untracked", "-m", &msg])
            .output()
            .await
            .map_err(|e| format!("git stash failed: {}", e))?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).to_string());
        }
        Ok(msg)
    }

    /// Undo the most recent checkpoint via git stash pop.
    pub async fn undo(&self) -> Result<String, String> {
        let output = tokio::process::Command::new("git")
            .args(["stash", "pop"])
            .output()
            .await
            .map_err(|e| format!("git stash pop failed: {}", e))?;
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr).to_string();
            if err.contains("No stash entries found") {
                return Err("No checkpoints to undo".to_string());
            }
            return Err(err);
        }
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// List all wgenty checkpoints.
    pub async fn list(&self) -> Result<String, String> {
        let output = tokio::process::Command::new("git")
            .args(["stash", "list", "--grep=wgenty-checkpoint"])
            .output()
            .await
            .map_err(|e| format!("git stash list failed: {}", e))?;
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

pub struct CheckpointTool {
    manager: std::sync::Arc<CheckpointManager>,
}

impl CheckpointTool {
    pub fn new(manager: std::sync::Arc<CheckpointManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for CheckpointTool {
    fn name(&self) -> &str {
        "checkpoint"
    }
    fn description(&self) -> &str {
        "Create a git stash checkpoint before potentially destructive operations. Returns checkpoint ID."
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "description": { "type": "string", "description": "Why this checkpoint is being created" }
            },
            "required": ["description"]
        })
    }
    async fn execute(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let desc = input["description"].as_str().unwrap_or("checkpoint");
        match self.manager.create(desc).await {
            Ok(id) => Ok(ToolOutput {
                output_type: "text".to_string(),
                content: format!("Checkpoint created: {}", id),
                metadata: std::collections::HashMap::new(),
            }),
            Err(e) => Err(ToolError {
                message: e,
                code: None,
            }),
        }
    }
}

pub struct UndoTool {
    manager: std::sync::Arc<CheckpointManager>,
}

impl UndoTool {
    pub fn new(manager: std::sync::Arc<CheckpointManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for UndoTool {
    fn name(&self) -> &str {
        "undo"
    }
    fn description(&self) -> &str {
        "Undo the most recent checkpoint, restoring files to their previous state via git stash pop."
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object", "properties": {} })
    }
    async fn execute(&self, _input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        match self.manager.undo().await {
            Ok(output) => Ok(ToolOutput {
                output_type: "text".to_string(),
                content: format!("Checkpoint restored:\n{}", output),
                metadata: std::collections::HashMap::new(),
            }),
            Err(e) => Err(ToolError {
                message: e,
                code: None,
            }),
        }
    }
}
