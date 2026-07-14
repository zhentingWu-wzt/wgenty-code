//! File Write Tool

use crate::agent::ToolContext;
use crate::tools::{Tool, ToolError, ToolOutput};
use async_trait::async_trait;
use serde_json;
use std::path::Path;

pub struct FileWriteTool;

impl Default for FileWriteTool {
    fn default() -> Self {
        Self::new()
    }
}

impl FileWriteTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FileWriteTool {
    fn name(&self) -> &str {
        "file_write"
    }

    fn description(&self) -> &str {
        "Write content to a new file or overwrite an existing file"
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to write"
                },
                "content": {
                    "type": "string",
                    "description": "Content to write to the file"
                }
            },
            "required": ["path", "content"]
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

        let content = input["content"].as_str().ok_or_else(|| ToolError {
            message: "content is required".to_string(),
            code: Some("missing_parameter".to_string()),
        })?;

        let path = Path::new(file_path);

        // Read old content before writing (for diff) — use tokio::fs to
        // avoid blocking the async runtime.
        let old_content = tokio::fs::read_to_string(path).await.ok();

        // Create parent directories if needed
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| ToolError {
                    message: format!("Failed to create directory: {}", e),
                    code: Some("directory_error".to_string()),
                })?;
        }

        // Atomic write: write to a temp file in the same directory, then
        // rename. This prevents data loss if the process crashes mid-write
        // (the original file is untouched until rename succeeds).
        let tmp_path = path.with_extension(format!(
            "{}.tmp",
            path.extension()
                .map(|e| e.to_string_lossy().to_string())
                .unwrap_or_default()
        ));
        tokio::fs::write(&tmp_path, content)
            .await
            .map_err(|e| ToolError {
                message: format!("Failed to write temp file: {}", e),
                code: Some("write_error".to_string()),
            })?;
        tokio::fs::rename(&tmp_path, path)
            .await
            .map_err(|e| ToolError {
                message: format!("Failed to rename temp file: {}", e),
                code: Some("write_error".to_string()),
            })?;

        let mut metadata = std::collections::HashMap::new();
        let old = old_content.unwrap_or_default();
        metadata.insert("old_content".to_string(), serde_json::json!(old));
        metadata.insert("new_content".to_string(), serde_json::json!(content));

        Ok(ToolOutput {
            output_type: "text".to_string(),
            content: format!("Successfully wrote {}", file_path),
            metadata,
        })
    }
}
