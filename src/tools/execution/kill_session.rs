use crate::tools::execution::CommandSessionManager;
use crate::tools::{Tool, ToolError, ToolOutput};
use async_trait::async_trait;
use std::sync::Arc;

pub struct KillSessionTool {
    sessions: Arc<CommandSessionManager>,
}

impl KillSessionTool {
    pub fn new(sessions: Arc<CommandSessionManager>) -> Self {
        Self { sessions }
    }
}

#[async_trait]
impl Tool for KillSessionTool {
    fn name(&self) -> &str {
        "kill_session"
    }

    fn description(&self) -> &str {
        "Terminate a running command session created by exec_command"
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "session_id": {
                    "type": "integer",
                    "description": "Session ID returned by exec_command"
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

        self.sessions.kill_session(session_id).await
    }
}
