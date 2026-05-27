use crate::tools::execution::CommandSessionManager;
use crate::tools::{Tool, ToolError, ToolOutput};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

pub struct ExecCommandTool {
    sessions: Arc<CommandSessionManager>,
}

impl ExecCommandTool {
    pub fn new(sessions: Arc<CommandSessionManager>) -> Self {
        Self { sessions }
    }
}

#[async_trait]
impl Tool for ExecCommandTool {
    fn name(&self) -> &str {
        "exec_command"
    }

    fn description(&self) -> &str {
        "Start a long-lived shell command session and return a session ID for follow-up input/output"
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Shell command to execute"
                },
                "workdir": {
                    "type": "string",
                    "description": "Working directory for the command (optional)"
                },
                "yield_time_ms": {
                    "type": "integer",
                    "description": "Milliseconds to wait before collecting initial output"
                },
                "max_output_chars": {
                    "type": "integer",
                    "description": "Maximum number of characters to return from stdout/stderr"
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let command = input["command"].as_str().ok_or_else(|| ToolError {
            message: "command is required".to_string(),
            code: Some("missing_parameter".to_string()),
        })?;
        let workdir = input["workdir"].as_str().map(std::path::PathBuf::from);
        let yield_time_ms = input["yield_time_ms"].as_u64().unwrap_or(1000);
        let max_output_chars = input["max_output_chars"]
            .as_u64()
            .map(|v| v as usize)
            .unwrap_or(4000);

        let session_id = self.sessions.spawn(command, workdir).await?;
        let chunk = self
            .sessions
            .read_incremental(session_id, yield_time_ms, max_output_chars)
            .await?;

        let mut metadata = HashMap::new();
        metadata.insert("session_id".to_string(), serde_json::json!(chunk.session_id));
        metadata.insert("stdout".to_string(), serde_json::json!(chunk.stdout));
        metadata.insert("stderr".to_string(), serde_json::json!(chunk.stderr));
        metadata.insert("finished".to_string(), serde_json::json!(chunk.finished));
        metadata.insert("exit_code".to_string(), serde_json::json!(chunk.exit_code));

        Ok(ToolOutput {
            output_type: "text".to_string(),
            content: chunk.combined,
            metadata,
        })
    }
}
