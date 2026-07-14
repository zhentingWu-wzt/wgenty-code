//! File Read Tool

use crate::agent::ToolContext;
use crate::tools::{Tool, ToolError, ToolOutput};
use async_trait::async_trait;
use serde_json;
use std::collections::HashMap;
use std::path::Path;

pub struct FileReadTool;

impl Default for FileReadTool {
    fn default() -> Self {
        Self::new()
    }
}

impl FileReadTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FileReadTool {
    fn name(&self) -> &str {
        "file_read"
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn description(&self) -> &str {
        "Read file contents, optionally restricted to a line range"
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to read"
                },
                "start_line": {
                    "type": "integer",
                    "description": "1-based start line (optional)"
                },
                "end_line": {
                    "type": "integer",
                    "description": "1-based end line inclusive (optional)"
                },
                "max_chars": {
                    "type": "integer",
                    "description": "Maximum number of characters to return"
                }
            },
            "required": ["path"]
        })
    }

    async fn execute_with_context(
        &self,
        context: &ToolContext<'_>,
        mut input: serde_json::Value,
    ) -> Result<ToolOutput, ToolError> {
        // s12: resolve relative paths against the per-agent workdir.
        if let Some(path) = input.get("path").and_then(|v| v.as_str()) {
            let resolved = crate::tools::resolve_path(path, context.workdir);
            input["path"] = serde_json::Value::String(resolved.to_string_lossy().into_owned());
        }
        self.execute(input).await
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let file_path = input["path"].as_str().ok_or_else(|| ToolError {
            message: "path is required".to_string(),
            code: Some("missing_parameter".to_string()),
        })?;

        let path = Path::new(file_path);

        // Use tokio::fs to avoid blocking the async runtime.
        // Skip the exists() TOCTOU check — read already returns a clear
        // error if the file doesn't exist.
        let bytes = tokio::fs::read(path).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ToolError {
                    message: format!("File does not exist: {}", file_path),
                    code: Some("file_not_found".to_string()),
                }
            } else {
                ToolError {
                    message: format!("Failed to read file: {}", e),
                    code: Some("read_error".to_string()),
                }
            }
        })?;

        if bytes.contains(&0) {
            return Err(ToolError {
                message: format!("Refusing to read binary file: {}", file_path),
                code: Some("binary_file".to_string()),
            });
        }

        let content = String::from_utf8(bytes).map_err(|e| ToolError {
            message: format!("Failed to decode file as UTF-8: {}", e),
            code: Some("encoding_error".to_string()),
        })?;

        let start_line = input["start_line"]
            .as_u64()
            .map(|v| v as usize)
            .unwrap_or(1);
        let end_line = input["end_line"].as_u64().map(|v| v as usize);
        let max_chars = input["max_chars"]
            .as_u64()
            .map(|v| v as usize)
            .unwrap_or(6000);

        let lines: Vec<&str> = content.lines().collect();
        let total_lines = lines.len();

        if total_lines == 0 {
            return Ok(ToolOutput {
                output_type: "text".to_string(),
                content: String::new(),
                metadata: {
                    let mut m = HashMap::new();
                    m.insert("path".to_string(), serde_json::json!(file_path));
                    m.insert("total_lines".to_string(), serde_json::json!(0));
                    m
                },
            });
        }

        let start_idx = (start_line.saturating_sub(1)).min(total_lines.saturating_sub(1));
        let end_idx = end_line
            .unwrap_or(total_lines)
            .min(total_lines)
            .max(start_idx + 1); // ensure end > start, at least 1 line

        let max_line_len: usize = 300;
        let mut rendered = lines[start_idx..end_idx]
            .iter()
            .enumerate()
            .map(|(idx, line)| {
                let display_line = if line.chars().count() > max_line_len {
                    format!(
                        "{}…[truncated]",
                        line.chars().take(max_line_len).collect::<String>()
                    )
                } else {
                    (*line).to_string()
                };
                format!("{:>4}\t{}", start_idx + idx + 1, display_line)
            })
            .collect::<Vec<_>>()
            .join("\n");

        let truncated = rendered.chars().count() > max_chars;
        if truncated {
            rendered = format!(
                "{}\n...[truncated]",
                rendered.chars().take(max_chars).collect::<String>()
            );
        }

        let mut metadata = HashMap::new();
        metadata.insert("path".to_string(), serde_json::json!(file_path));
        metadata.insert("start_line".to_string(), serde_json::json!(start_idx + 1));
        metadata.insert("end_line".to_string(), serde_json::json!(end_idx));
        metadata.insert("total_lines".to_string(), serde_json::json!(total_lines));
        metadata.insert("truncated".to_string(), serde_json::json!(truncated));

        Ok(ToolOutput {
            output_type: "text".to_string(),
            content: rendered,
            metadata,
        })
    }
}
