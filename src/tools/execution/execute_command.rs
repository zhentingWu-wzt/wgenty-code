//! Execute Command Tool — runs shell commands with sandbox isolation.

use crate::sandbox::{SandboxConfig, SandboxManager, SandboxProfile, SecurityLevel};
use std::path::PathBuf;
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

    /// Build a default sandbox profile for the current working directory.
    fn default_profile(&self) -> SandboxProfile {
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
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
                    "description": "Timeout in seconds (optional)"
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

        // Try sandbox execution first; fall back to direct execution
        if let Some(ref sb) = self.sandbox {
            let mut profile = self.default_profile();
            profile.resources.max_wall_seconds = user_timeout;

            match sb.execute(command, &profile).await {
                Ok(output) => {
                    if output.killed_by_sandbox {
                        tracing::warn!(
                            "Sandbox ({}) killed process, falling back to direct execution.",
                            sb.status().backend_name
                        );
                    } else if output.exit_code != 0 {
                        return Err(ToolError {
                            message: format!(
                                "exit code: {}\nstdout:\n{}\nstderr:\n{}",
                                output.exit_code, output.stdout, output.stderr
                            ),
                            code: Some("non_zero_exit".to_string()),
                        });
                    } else {
                        return Ok(ToolOutput {
                            output_type: "text".to_string(),
                            content: output.stdout,
                            metadata: std::collections::HashMap::new(),
                        });
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "Sandbox execution failed ({}): {}. Falling back to direct execution.",
                        sb.status().backend_name,
                        e
                    );
                    // Fall through to direct execution
                }
            }
        }

        // Direct execution (fallback)
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(user_timeout),
            tokio::process::Command::new("sh")
                .arg("-c")
                .arg(command)
                .output(),
        )
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
