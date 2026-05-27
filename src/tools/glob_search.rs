use super::{Tool, ToolError, ToolOutput};
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::Path;

pub struct GlobTool;

impl GlobTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &str {
        "glob"
    }

    fn description(&self) -> &str {
        "Find files matching glob patterns"
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Base path to search from"
                },
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern to match"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of matches to return"
                }
            },
            "required": ["path", "pattern"]
        })
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let path = input["path"].as_str().ok_or_else(|| ToolError {
            message: "path is required".to_string(),
            code: Some("missing_parameter".to_string()),
        })?;
        let pattern = input["pattern"].as_str().ok_or_else(|| ToolError {
            message: "pattern is required".to_string(),
            code: Some("missing_parameter".to_string()),
        })?;
        let max_results = input["max_results"].as_u64().unwrap_or(200) as usize;

        let base = Path::new(path);
        if !base.exists() {
            return Err(ToolError {
                message: format!("Path does not exist: {}", path),
                code: Some("path_not_found".to_string()),
            });
        }

        let glob_pattern = glob::Pattern::new(pattern).map_err(|e| ToolError {
            message: format!("Invalid glob pattern: {}", e),
            code: Some("invalid_pattern".to_string()),
        })?;

        let mut results = Vec::new();
        let mut truncated = false;

        for entry in walkdir::WalkDir::new(base)
            .into_iter()
            .filter_map(|entry| entry.ok())
        {
            let entry_path = entry.path();
            let display = entry_path.to_string_lossy();
            if glob_pattern.matches(&display) {
                results.push(display.to_string());
                if results.len() >= max_results {
                    truncated = true;
                    break;
                }
            }
        }

        let mut metadata = HashMap::new();
        metadata.insert("result_count".to_string(), serde_json::json!(results.len()));
        metadata.insert("truncated".to_string(), serde_json::json!(truncated));

        Ok(ToolOutput {
            output_type: "text".to_string(),
            content: results.join("\n"),
            metadata,
        })
    }
}

impl Default for GlobTool {
    fn default() -> Self {
        Self::new()
    }
}
