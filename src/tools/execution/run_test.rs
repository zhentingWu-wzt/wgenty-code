//! Run Test Tool — framework-aware test execution with structured output.
//!
//! Detects the project's test framework, builds the correct command,
//! executes via the sandbox, and returns parsed results.

use async_trait::async_trait;
use serde_json::json;
use std::path::Path;

use super::sandbox_exec::{
    resolve_for_context, sandbox_infra_tool_error, sandbox_metadata, should_degrade_to_direct,
};
use super::test_output::TestOutput;
use crate::agent::ToolContext;
use crate::sandbox::{shell_command_captured, EffectiveMode, NetworkPolicy, SandboxManager};
use crate::tools::{Tool, ToolError, ToolOutput};

pub struct RunTestTool {
    sandbox: std::sync::Arc<SandboxManager>,
}

impl RunTestTool {
    pub fn new(sandbox: std::sync::Arc<SandboxManager>) -> Self {
        Self { sandbox }
    }

    /// Detect the test framework from the project root.
    fn detect_framework(&self, cwd: &Path) -> (&'static str, Vec<String>) {
        // Check in priority order
        let checks: &[(&str, &str, &[&str])] = &[
            ("rust-cargo", "Cargo.toml", &["cargo", "test"]),
            ("go", "go.mod", &["go", "test", "./..."]),
            ("python-pytest", "pyproject.toml", &["pytest"]),
            ("python-pytest", "setup.cfg", &["pytest"]),
            ("python-pytest", "setup.py", &["pytest"]),
            ("python-unittest", ".", &["python", "-m", "unittest"]),
            ("node-jest", "jest.config.js", &["npx", "jest"]),
            ("node-jest", "jest.config.ts", &["npx", "jest"]),
            ("node-vitest", "vitest.config.js", &["npx", "vitest", "run"]),
            ("node-vitest", "vitest.config.ts", &["npx", "vitest", "run"]),
            ("node-npm", "package.json", &["npm", "test"]),
        ];

        for (name, marker, cmd) in checks {
            if cwd.join(marker).exists() {
                return (name, cmd.iter().map(|s| s.to_string()).collect());
            }
        }

        // If package.json exists but no specific config, npm test is the fallback
        if cwd.join("package.json").exists() {
            return ("node-npm", vec!["npm".to_string(), "test".to_string()]);
        }

        ("unknown", vec!["cargo".to_string(), "test".to_string()])
    }

    async fn run(
        &self,
        input: serde_json::Value,
        workdir: Option<&Path>,
        mode: EffectiveMode,
    ) -> Result<ToolOutput, ToolError> {
        let cwd = workdir
            .map(|p| p.to_path_buf())
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| Path::new(".").to_path_buf());
        let framework = input["framework"].as_str().unwrap_or("auto");
        let filter = input["filter"].as_str();
        let file = input["file"].as_str();
        let timeout = input["timeout_secs"].as_u64().unwrap_or(120);
        let allow_network = input["allow_network"].as_bool().unwrap_or(false);
        let verbose = input["verbose"].as_bool().unwrap_or(false);

        // Detect or override framework
        let (fw_name, mut base_cmd) = if framework == "auto" {
            self.detect_framework(&cwd)
        } else {
            let cmd = match framework {
                "rust-cargo" => vec!["cargo".to_string(), "test".to_string()],
                "node-jest" => vec!["npx".to_string(), "jest".to_string()],
                "node-vitest" => vec!["npx".to_string(), "vitest".to_string(), "run".to_string()],
                "node-npm" => vec!["npm".to_string(), "test".to_string()],
                "python-pytest" => vec!["pytest".to_string()],
                "python-unittest" => {
                    vec![
                        "python".to_string(),
                        "-m".to_string(),
                        "unittest".to_string(),
                    ]
                }
                "go" => vec!["go".to_string(), "test".to_string(), "./...".to_string()],
                other => {
                    return Ok(ToolOutput {
                        output_type: "test_result".to_string(),
                        content: json!({
                            "success": false,
                            "error": format!("Unknown framework: {}", other),
                        })
                        .to_string(),
                        metadata: std::collections::HashMap::new(),
                    });
                }
            };
            (framework, cmd)
        };

        // Apply filter and file arguments (match prior run_test behavior)
        if let Some(f) = filter {
            match fw_name {
                "rust-cargo" => {
                    base_cmd.push(f.to_string());
                    base_cmd.push("--".to_string());
                    base_cmd.push("--nocapture".to_string());
                }
                "node-jest" | "node-vitest" => {
                    base_cmd.push("-t".to_string());
                    base_cmd.push(f.to_string());
                }
                "python-pytest" => {
                    base_cmd.push("-k".to_string());
                    base_cmd.push(f.to_string());
                }
                _ => base_cmd.push(f.to_string()),
            }
        }

        if let Some(f) = file {
            base_cmd.push(f.to_string());
        }

        let command = base_cmd.join(" ");

        let network = if allow_network {
            Some(NetworkPolicy::Full)
        } else {
            None
        };
        let mut policy = resolve_for_context(mode, Some(cwd.as_path()), network);
        policy.profile.resources.max_wall_seconds = timeout;

        let status = self.sandbox.status();
        let mut metadata = sandbox_metadata(
            mode,
            policy.level,
            &status.backend_name,
            false,
            status.is_hardware_enforced,
            policy.fail_mode,
        );

        // Execute via sandbox (or degrade when allowed)
        let output = if policy.enabled {
            match self.sandbox.execute(&command, &policy.profile).await {
                Ok(out) => out,
                Err(e) => {
                    if !should_degrade_to_direct(policy.fail_mode) {
                        let err = sandbox_infra_tool_error(&status.backend_name, &e);
                        return Ok(ToolOutput {
                            output_type: "test_result".to_string(),
                            content: json!({
                                "success": false,
                                "error": err.message,
                                "framework": fw_name,
                                "command": command,
                                "code": "sandbox_spawn_failed",
                            })
                            .to_string(),
                            metadata,
                        });
                    }
                    tracing::warn!(
                        command = %command,
                        backend = %status.backend_name,
                        error = %e,
                        "Sandbox test execution failed; degrading to direct"
                    );
                    metadata = sandbox_metadata(
                        mode,
                        policy.level,
                        &status.backend_name,
                        true,
                        false,
                        policy.fail_mode,
                    );
                    match run_direct(&command, &cwd, timeout).await {
                        Ok(out) => out,
                        Err(err_msg) => {
                            return Ok(ToolOutput {
                                output_type: "test_result".to_string(),
                                content: json!({
                                    "success": false,
                                    "error": err_msg,
                                    "framework": fw_name,
                                    "command": command,
                                })
                                .to_string(),
                                metadata,
                            });
                        }
                    }
                }
            }
        } else {
            metadata = sandbox_metadata(
                mode,
                policy.level,
                &status.backend_name,
                true,
                false,
                policy.fail_mode,
            );
            match run_direct(&command, &cwd, timeout).await {
                Ok(out) => out,
                Err(err_msg) => {
                    return Ok(ToolOutput {
                        output_type: "test_result".to_string(),
                        content: json!({
                            "success": false,
                            "error": err_msg,
                            "framework": fw_name,
                            "command": command,
                        })
                        .to_string(),
                        metadata,
                    });
                }
            }
        };

        let parsed = TestOutput::parse(fw_name, &output.stdout, &output.stderr, output.exit_code);

        let result = json!({
            "success": parsed.success,
            "framework": fw_name,
            "command": command,
            "passed": parsed.passed,
            "failed": parsed.failed,
            "skipped": parsed.skipped,
            "timed_out": parsed.timed_out,
            "duration_ms": parsed.duration_ms,
            "exit_code": output.exit_code,
            "summary": parsed.summary,
            "failures": parsed.failures,
        });

        metadata.insert("framework".to_string(), json!(fw_name));
        metadata.insert("passed".to_string(), json!(parsed.passed));
        metadata.insert("failed".to_string(), json!(parsed.failed));

        if verbose {
            metadata.insert("stdout".to_string(), json!(output.stdout));
            metadata.insert("stderr".to_string(), json!(output.stderr));
        }

        Ok(ToolOutput {
            output_type: "test_result".to_string(),
            content: result.to_string(),
            metadata,
        })
    }
}

/// Direct test run used only under DegradeWithMark / disabled sandbox.
async fn run_direct(
    command: &str,
    cwd: &Path,
    timeout: u64,
) -> Result<crate::sandbox::SandboxOutput, String> {
    let output = tokio::time::timeout(std::time::Duration::from_secs(timeout), {
        let mut cmd = shell_command_captured(command);
        cmd.current_dir(cwd);
        cmd.output()
    })
    .await
    .map_err(|_| "Test execution timed out".to_string())?
    .map_err(|e| format!("Test execution failed: {}", e))?;

    #[cfg(unix)]
    let signal = {
        use std::os::unix::process::ExitStatusExt;
        output.status.signal()
    };
    #[cfg(not(unix))]
    let signal: Option<i32> = None;

    Ok(crate::sandbox::SandboxOutput {
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        exit_code: output.status.code().unwrap_or(-1),
        killed_by_sandbox: false,
        signal,
    })
}

#[async_trait]
impl Tool for RunTestTool {
    fn name(&self) -> &str {
        "run_test"
    }

    fn description(&self) -> &str {
        "Run project tests with automatic framework detection. \
         Supports Rust (cargo test), JS/TS (jest/vitest/npm test), \
         Python (pytest/unittest), Go (go test). \
         Returns structured results with pass/fail counts."
    }

    fn is_read_only(&self) -> bool {
        // Tests may write to target/ or __pycache__ but shouldn't modify source
        false
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "filter": {
                    "type": "string",
                    "description": "Optional test name pattern to filter which tests run."
                },
                "file": {
                    "type": "string",
                    "description": "Optional path to a specific test file to run."
                },
                "framework": {
                    "type": "string",
                    "enum": ["auto", "rust-cargo", "node-jest", "node-vitest", "node-npm", "python-pytest", "python-unittest", "go"],
                    "description": "Test framework override. Default 'auto' detects from project files."
                },
                "timeout_secs": {
                    "type": "integer",
                    "default": 120,
                    "description": "Timeout in seconds (default 120)."
                },
                "allow_network": {
                    "type": "boolean",
                    "default": false,
                    "description": "Allow network access for integration tests within the mode's security level (does not lower the level)."
                },
                "verbose": {
                    "type": "boolean",
                    "default": false,
                    "description": "Include full test output in result."
                }
            },
            "required": []
        })
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        self.run(input, None, EffectiveMode::default()).await
    }

    async fn execute_with_context(
        &self,
        context: &ToolContext<'_>,
        input: serde_json::Value,
    ) -> Result<ToolOutput, ToolError> {
        self.run(input, context.workdir, context.effective_mode)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_framework_detection_rust() {
        let tool = RunTestTool::new(std::sync::Arc::new(SandboxManager::new()));
        // In the project root, Cargo.toml should exist
        let cwd = std::env::current_dir().unwrap();
        let (name, cmd) = tool.detect_framework(&cwd);
        assert_eq!(name, "rust-cargo");
        assert_eq!(cmd, vec!["cargo", "test"]);
    }

    #[test]
    fn test_input_schema_valid() {
        let tool = RunTestTool::new(std::sync::Arc::new(SandboxManager::new()));
        let schema = tool.input_schema();
        assert!(schema["properties"]["filter"].is_object());
        assert!(schema["properties"]["timeout_secs"].is_object());
        let desc = schema["properties"]["allow_network"]["description"]
            .as_str()
            .unwrap_or("");
        assert!(!desc.contains("Minimal sandbox level"));
    }
}
