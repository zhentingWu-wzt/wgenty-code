use crate::agent::ToolContext;
use crate::sandbox::EffectiveMode;
use crate::tools::execution::sandbox_exec::{resolve_for_context, sandbox_metadata};
use crate::tools::execution::CommandSessionManager;
use crate::tools::{Tool, ToolError, ToolOutput};
use async_trait::async_trait;
use std::sync::Arc;

pub struct ExecCommandTool {
    sessions: Arc<CommandSessionManager>,
}

impl ExecCommandTool {
    pub fn new(sessions: Arc<CommandSessionManager>) -> Self {
        Self { sessions }
    }

    async fn run(
        &self,
        command: &str,
        workdir: Option<std::path::PathBuf>,
        yield_time_ms: u64,
        max_output_chars: usize,
        mode: EffectiveMode,
    ) -> Result<ToolOutput, ToolError> {
        let policy = resolve_for_context(mode, workdir.as_deref(), None);
        // Best-effort: if settings disabled sandbox, mark bypassed. Runtime
        // degrade-on-spawn is still reflected only when spawn fails open under
        // DegradeWithMark (session continues; full fidelity needs spawn result).
        let status = self.sessions.sandbox_status();
        let session_id = self.sessions.spawn(command, workdir.clone(), mode).await?;
        let chunk = self
            .sessions
            .read_incremental(session_id, yield_time_ms, max_output_chars)
            .await?;

        let bypassed = !policy.enabled;
        let mut metadata = sandbox_metadata(
            mode,
            policy.level,
            &status.backend_name,
            bypassed,
            status.is_hardware_enforced && !bypassed,
            policy.fail_mode,
        );
        metadata.insert(
            "session_id".to_string(),
            serde_json::json!(chunk.session_id),
        );
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
            .map(|v| usize::try_from(v).unwrap_or(usize::MAX))
            .unwrap_or(4000);

        self.run(
            command,
            workdir,
            yield_time_ms,
            max_output_chars,
            EffectiveMode::default(),
        )
        .await
    }

    async fn execute_with_context(
        &self,
        context: &ToolContext<'_>,
        input: serde_json::Value,
    ) -> Result<ToolOutput, ToolError> {
        let command = input["command"].as_str().ok_or_else(|| ToolError {
            message: "command is required".to_string(),
            code: Some("missing_parameter".to_string()),
        })?;
        let workdir = input["workdir"]
            .as_str()
            .map(std::path::PathBuf::from)
            .or_else(|| context.workdir.map(|p| p.to_path_buf()));
        let yield_time_ms = input["yield_time_ms"].as_u64().unwrap_or(1000);
        let max_output_chars = input["max_output_chars"]
            .as_u64()
            .map(|v| usize::try_from(v).unwrap_or(usize::MAX))
            .unwrap_or(4000);

        self.run(
            command,
            workdir,
            yield_time_ms,
            max_output_chars,
            context.effective_mode,
        )
        .await
    }
}
