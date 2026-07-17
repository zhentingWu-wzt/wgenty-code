//! Execute Command Tool — runs shell commands with sandbox isolation.

use crate::agent::ToolContext;
use crate::sandbox::{
    shell_command_captured, SandboxConfig, SandboxManager, SandboxProfile, SecurityLevel,
};
use crate::tools::{Tool, ToolError, ToolOutput};
use async_trait::async_trait;
use std::path::PathBuf;

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

    /// Build a default sandbox profile for the current working directory.
    fn default_profile(&self, workdir: Option<&std::path::Path>) -> SandboxProfile {
        let cwd = workdir
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let mut profile = SandboxConfig::builder(cwd)
            .security_level(SecurityLevel::Minimal)
            .build();
        // Add home directory so tools (cargo, node, etc.) can read configs/caches
        if let Ok(home) = std::env::var("HOME") {
            profile.readable_paths.push(PathBuf::from(home));
        }
        profile
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
        "Execute a shell command (sandboxed when available)"
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
        self.run(command, user_timeout, None).await
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
        self.run(command, user_timeout, context.workdir).await
    }
}

impl ExecuteCommandTool {
    /// Core execution: sandbox first, direct fallback. `workdir` overrides the
    /// process cwd (s12 worktree isolation) for both sandbox profile and the
    /// fallback platform shell command.
    async fn run(
        &self,
        command: &str,
        user_timeout: u64,
        workdir: Option<&std::path::Path>,
    ) -> Result<ToolOutput, ToolError> {
        // Try sandbox execution first. Only fall back on sandbox infrastructure
        // failure (spawn/wait error). Sandbox-killed or non-zero exits are real
        // results — re-running them outside the sandbox would bypass isolation
        // and (on Windows) risk console corruption.
        if let Some(ref sb) = self.sandbox {
            let mut profile = self.default_profile(workdir);
            profile.resources.max_wall_seconds = user_timeout;

            // Seatbelt does not translate max_wall_seconds into a profile
            // rule, so enforce the wall-clock timeout here. The 30s buffer
            // mirrors the documented "agent adds a 30s buffer" behavior and
            // keeps the sandbox path consistent with the direct-execution
            // fallback below.
            let timeout_secs = user_timeout.saturating_add(30);
            match tokio::time::timeout(
                std::time::Duration::from_secs(timeout_secs),
                sb.execute(command, &profile),
            )
            .await
            {
                Ok(Ok(output)) => {
                    if output.killed_by_sandbox {
                        let signal_info = output
                            .signal
                            .map(|s| format!(", signal {}", s))
                            .unwrap_or_default();
                        return Err(ToolError {
                            message: format!(
                                "command terminated by sandbox (backend: {}{})\nstdout:\n{}\nstderr:\n{}",
                                sb.status().backend_name,
                                signal_info,
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
                    return Ok(ToolOutput {
                        output_type: "text".to_string(),
                        content: output.stdout,
                        metadata: std::collections::HashMap::new(),
                    });
                }
                Ok(Err(e)) => {
                    tracing::warn!(
                        command = %command,
                        backend = %sb.status().backend_name,
                        error = %e,
                        "Sandbox execution failed; falling back to direct (captured stdio)"
                    );
                }
                Err(_) => {
                    return Err(ToolError {
                        message: "Command timed out".to_string(),
                        code: Some("timeout".to_string()),
                    });
                }
            }
        }

        // Direct execution (no sandbox / infrastructure fallback)
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
                    metadata: std::collections::HashMap::new(),
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
