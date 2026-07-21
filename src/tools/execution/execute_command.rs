//! Execute Command Tool — runs shell commands with sandbox isolation.

use crate::agent::ToolContext;
use crate::sandbox::{shell_command_captured, EffectiveMode, SandboxManager};
use crate::tools::execution::sandbox_exec::{
    resolve_for_context, sandbox_infra_tool_error, sandbox_metadata, should_degrade_to_direct,
};
use crate::tools::{Tool, ToolError, ToolOutput};
use async_trait::async_trait;

pub struct ExecuteCommandTool {
    sandbox: Option<std::sync::Arc<SandboxManager>>,
}

impl ExecuteCommandTool {
    pub fn new() -> Self {
        Self { sandbox: None }
    }

    /// Create with sandbox enabled.
    pub fn with_sandbox(sandbox: std::sync::Arc<SandboxManager>) -> Self {
        Self {
            sandbox: Some(sandbox),
        }
    }
}

impl Default for ExecuteCommandTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ExecuteCommandTool {
    fn name(&self) -> &str {
        "execute_command"
    }

    fn description(&self) -> &str {
        "Execute a shell command and block until it completes or times out (sandboxed when available). Use for one-shot commands including long-running ones (builds, tests, sleep, watches); set timeout (seconds, default 60) for commands that may exceed it. Returns full stdout on success; returns an error on non-zero exit or timeout. For interactive sessions needing follow-up stdin, use exec_command instead."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Command to execute"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Timeout in seconds (optional); the agent adds a 30s buffer to this value, with a minimum of 120s"
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
        let user_timeout = input["timeout"].as_u64().unwrap_or(60);
        self.run(command, user_timeout, None, EffectiveMode::default())
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
        let user_timeout = input["timeout"].as_u64().unwrap_or(60);
        self.run(
            command,
            user_timeout,
            context.workdir,
            context.effective_mode,
        )
        .await
    }
}

impl ExecuteCommandTool {
    /// Core execution: resolve mode policy, sandbox first, HardFail vs Degrade.
    /// `workdir` overrides the process cwd (s12 worktree isolation).
    async fn run(
        &self,
        command: &str,
        user_timeout: u64,
        workdir: Option<&std::path::Path>,
        mode: EffectiveMode,
    ) -> Result<ToolOutput, ToolError> {
        let mut policy = resolve_for_context(mode, workdir, None);
        policy.profile.resources.max_wall_seconds = user_timeout;

        let backend_name = self
            .sandbox
            .as_ref()
            .map(|sb| sb.status().backend_name.clone())
            .unwrap_or_else(|| "none".to_string());
        let hardware_enforced = self
            .sandbox
            .as_ref()
            .map(|sb| sb.status().is_hardware_enforced)
            .unwrap_or(false);

        // Attempt sandboxed execution when manager present and sandbox enabled.
        if policy.enabled {
            if let Some(ref sb) = self.sandbox {
                match sb.execute(command, &policy.profile).await {
                    Ok(output) => {
                        if output.killed_by_sandbox {
                            return Err(ToolError {
                                message: format!(
                                    "command killed by sandbox ({})\nstdout:\n{}\nstderr:\n{}",
                                    sb.status().backend_name,
                                    output.stdout,
                                    output.stderr
                                ),
                                code: Some("sandbox_killed".to_string()),
                            });
                        }
                        if output.exit_code != 0 {
                            return Err(ToolError {
                                message: format!(
                                    "exit code: {}\nstdout:\n{}\nstderr:\n{}",
                                    output.exit_code, output.stdout, output.stderr
                                ),
                                code: Some("non_zero_exit".to_string()),
                            });
                        }
                        let metadata = sandbox_metadata(
                            mode,
                            policy.level,
                            &sb.status().backend_name,
                            false,
                            sb.status().is_hardware_enforced,
                            policy.fail_mode,
                        );
                        return Ok(ToolOutput {
                            output_type: "text".to_string(),
                            content: output.stdout,
                            metadata,
                        });
                    }
                    Err(e) => {
                        if !should_degrade_to_direct(policy.fail_mode) {
                            return Err(sandbox_infra_tool_error(&sb.status().backend_name, e));
                        }
                        tracing::warn!(
                            command = %command,
                            backend = %sb.status().backend_name,
                            error = %e,
                            "Sandbox execution failed; degrading to direct (captured stdio)"
                        );
                    }
                }
            } else {
                // No manager installed: treat as infrastructure missing.
                if !should_degrade_to_direct(policy.fail_mode) {
                    return Err(sandbox_infra_tool_error(
                        "none",
                        "sandbox manager not attached",
                    ));
                }
            }
        }

        // Direct execution (settings disabled, DegradeWithMark, or missing manager under degrade).
        let bypassed = true;
        let metadata = sandbox_metadata(
            mode,
            policy.level,
            &backend_name,
            bypassed,
            hardware_enforced && !bypassed,
            policy.fail_mode,
        );

        let output = tokio::time::timeout(std::time::Duration::from_secs(user_timeout), {
            let mut cmd = shell_command_captured(command);
            if let Some(dir) = workdir {
                cmd.current_dir(dir);
            }
            cmd.output()
        })
        .await;

        match output {
            Ok(Ok(result)) => {
                let stdout = String::from_utf8_lossy(&result.stdout).to_string();
                let stderr = String::from_utf8_lossy(&result.stderr).to_string();
                if !result.status.success() {
                    return Err(ToolError {
                        message: format!(
                            "exit code: {}\nstdout:\n{}\nstderr:\n{}",
                            result.status, stdout, stderr
                        ),
                        code: Some("non_zero_exit".to_string()),
                    });
                }
                Ok(ToolOutput {
                    output_type: "text".to_string(),
                    content: stdout,
                    metadata,
                })
            }
            Ok(Err(e)) => Err(ToolError {
                message: format!("Failed to execute command: {}", e),
                code: Some("execution_error".to_string()),
            }),
            Err(_) => Err(ToolError {
                message: "Command timed out".to_string(),
                code: Some("timeout".to_string()),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{AgentExecutionContext, SessionId, ToolInvocationId};
    use serde_json::json;

    #[tokio::test]
    async fn normal_without_sandbox_manager_hard_fails() {
        let tool = ExecuteCommandTool::new();
        let err = tool
            .execute(json!({"command": "echo hi"}))
            .await
            .unwrap_err();
        assert_eq!(err.code.as_deref(), Some("sandbox_spawn_failed"));
    }

    #[tokio::test]
    async fn yolo_without_sandbox_manager_degrades() {
        let tool = ExecuteCommandTool::new();
        let root = AgentExecutionContext::root(SessionId::new("test-session"));
        let ctx = ToolContext {
            agent: &root,
            invocation_id: ToolInvocationId::new("inv"),
            origin_turn_id: None,
            workdir: None,
            effective_mode: EffectiveMode::Yolo,
            checkpoint: None,
        };
        let out = tool
            .execute_with_context(&ctx, json!({"command": "echo hi"}))
            .await
            .expect("yolo should degrade to direct");
        assert_eq!(
            out.metadata
                .get("sandbox_bypassed")
                .and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            out.metadata.get("permission_mode").and_then(|v| v.as_str()),
            Some("yolo")
        );
    }
}
