//! File Write Tool

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

        // Read old content before writing (for diff)
        let old_content = if path.exists() {
            std::fs::read_to_string(path).ok()
        } else {
            None
        };

        // Create parent directories if needed
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| ToolError {
                message: format!("Failed to create directory: {}", e),
                code: Some("directory_error".to_string()),
            })?;
        }

        std::fs::write(path, content).map_err(|e| ToolError {
            message: format!("Failed to write file: {}", e),
            code: Some("write_error".to_string()),
        })?;

        let mut metadata = std::collections::HashMap::new();
        if let Some(old) = old_content {
            metadata.insert("old_content".to_string(), serde_json::json!(old));
            metadata.insert("new_content".to_string(), serde_json::json!(content));
        }

        Ok(ToolOutput {
            output_type: "text".to_string(),
            content: format!("Successfully wrote {}", file_path),
            metadata,
        })
    }
}
