use crate::tools::execution::CommandSessionManager;
use crate::tools::{Tool, ToolError, ToolOutput};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

pub struct WriteStdinTool {
    sessions: Arc<CommandSessionManager>,
}

impl WriteStdinTool {
    pub fn new(sessions: Arc<CommandSessionManager>) -> Self {
        Self { sessions }
    }
}

#[async_trait]
impl Tool for WriteStdinTool {
    fn name(&self) -> &str {
        "write_stdin"
    }

    fn description(&self) -> &str {
        "Write input to a running command session and collect incremental output"
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "session_id": {
                    "type": "integer",
                    "description": "Session ID returned by exec_command"
                },
                "chars": {
                    "type": "string",
                    "description": "Characters to write to stdin"
                },
                "yield_time_ms": {
                    "type": "integer",
                    "description": "Milliseconds to wait before collecting output"
                },
                "max_output_chars": {
                    "type": "integer",
                    "description": "Maximum number of characters to return from stdout/stderr"
                }
            },
            "required": ["session_id"]
        })
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let session_id = input["session_id"].as_u64().ok_or_else(|| ToolError {
            message: "session_id is required".to_string(),
            code: Some("missing_parameter".to_string()),
        })?;
        let chars = input["chars"].as_str().unwrap_or("");
        let yield_time_ms = input["yield_time_ms"].as_u64().unwrap_or(1000);
        let max_output_chars = input["max_output_chars"]
            .as_u64()
            .map(|v| v as usize)
            .unwrap_or(4000);

        let chunk = self
            .sessions
            .write_stdin(session_id, chars, yield_time_ms, max_output_chars)
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
