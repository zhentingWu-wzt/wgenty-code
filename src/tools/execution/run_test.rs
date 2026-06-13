//! Run Test Tool — framework-aware test execution with structured output.
//!
//! Detects the project's test framework, builds the correct command,
//! executes via the sandbox, and returns parsed results.

use async_trait::async_trait;
use serde_json::json;
use std::path::Path;

use super::test_output::TestOutput;
use crate::sandbox::{SandboxConfig, SandboxManager, SecurityLevel};
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
                    "description": "Allow network access for integration tests. Uses Minimal sandbox level."
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
        let cwd = std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf());
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
                "python-unittest" => vec![
                    "python".to_string(),
                    "-m".to_string(),
                    "unittest".to_string(),
                ],
                "go" => vec!["go".to_string(), "test".to_string(), "./...".to_string()],
                _ => vec!["cargo".to_string(), "test".to_string()],
            };
            (framework, cmd)
        };

        // Apply filter and file arguments
        if let Some(f) = filter {
            match fw_name {
                "rust-cargo" | "go" => base_cmd.push(f.to_string()),
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

        // Build shell command
        let command = base_cmd.join(" ");

        // Build sandbox profile
        let security = if allow_network {
            SecurityLevel::Minimal
        } else {
            SecurityLevel::Standard
        };
        let profile = SandboxConfig::builder(&cwd)
            .security_level(security)
            .wall_timeout_secs(timeout)
            .build();

        // Execute via sandbox
        let sb = self.sandbox.as_ref();
        let output = match sb.execute(&command, &profile).await {
            Ok(out) => out,
            Err(e) => {
                let error_msg = format!("Test execution failed: {}", e);
                return Ok(ToolOutput {
                    output_type: "test_result".to_string(),
                    content: json!({
                        "success": false,
                        "error": error_msg,
                        "framework": fw_name,
                        "command": command,
                    })
                    .to_string(),
                    metadata: std::collections::HashMap::new(),
                });
            }
        };

        // Parse output
        let parsed = TestOutput::parse(fw_name, &output.stdout, &output.stderr, output.exit_code);

        // Build structured result
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

        let mut metadata = std::collections::HashMap::new();
        metadata.insert("framework".to_string(), json!(fw_name));
        metadata.insert("passed".to_string(), json!(parsed.passed));
        metadata.insert("failed".to_string(), json!(parsed.failed));

        if verbose {
            let mut m = metadata;
            m.insert("stdout".to_string(), json!(output.stdout));
            m.insert("stderr".to_string(), json!(output.stderr));
            return Ok(ToolOutput {
                output_type: "test_result".to_string(),
                content: result.to_string(),
                metadata: m,
            });
        }

        Ok(ToolOutput {
            output_type: "test_result".to_string(),
            content: result.to_string(),
            metadata,
        })
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
    }
}
