//! File Edit Tool

use crate::agent::ToolContext;
use crate::tools::{Tool, ToolError, ToolOutput};
use async_trait::async_trait;
use serde_json;
use std::path::Path;

pub struct FileEditTool;

impl Default for FileEditTool {
    fn default() -> Self {
        Self::new()
    }
}

impl FileEditTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FileEditTool {
    fn name(&self) -> &str {
        "file_edit"
    }

    fn description(&self) -> &str {
        "Edit a file by replacing specific content"
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to edit"
                },
                "old_content": {
                    "type": "string",
                    "description": "Content to find and replace"
                },
                "new_content": {
                    "type": "string",
                    "description": "New content to replace with"
                }
            },
            "required": ["path", "old_content", "new_content"]
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

        let old_content = input["old_content"].as_str().ok_or_else(|| ToolError {
            message: "old_content is required".to_string(),
            code: Some("missing_parameter".to_string()),
        })?;

        let new_content = input["new_content"].as_str().ok_or_else(|| ToolError {
            message: "new_content is required".to_string(),
            code: Some("missing_parameter".to_string()),
        })?;

        let path = Path::new(file_path);

        if !path.exists() {
            return Err(ToolError {
                message: format!("File does not exist: {}", file_path),
                code: Some("file_not_found".to_string()),
            });
        }

        let old_file_content = std::fs::read_to_string(path).map_err(|e| ToolError {
            message: format!("Failed to read file: {}", e),
            code: Some("read_error".to_string()),
        })?;

        if !old_file_content.contains(old_content) {
            return Err(ToolError {
                message: "old_content not found in file".to_string(),
                code: Some("content_not_found".to_string()),
            });
        }

        let new_file_content = old_file_content.replace(old_content, new_content);

        let write_result = std::fs::write(path, &new_file_content).map_err(|e| ToolError {
            message: format!("Failed to write file: {}", e),
            code: Some("write_error".to_string()),
        });

        let mut metadata = std::collections::HashMap::new();
        metadata.insert(
            "old_content".to_string(),
            serde_json::json!(old_file_content),
        );
        metadata.insert(
            "new_content".to_string(),
            serde_json::json!(new_file_content),
        );

        write_result?;

        Ok(ToolOutput {
            output_type: "text".to_string(),
            content: format!("Successfully edited {}", file_path),
            metadata,
        })
    }
}
