//! Search Tool

use crate::tools::{Tool, ToolError, ToolOutput};
use async_trait::async_trait;
use serde_json;

pub struct SearchTool;

impl Default for SearchTool {
    fn default() -> Self {
        Self::new()
    }
}

impl SearchTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for SearchTool {
    fn name(&self) -> &str {
        "search"
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn description(&self) -> &str {
        "Search for patterns in files using regex. Prefer grep for richer include/exclude controls."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Directory path to search in"
                },
                "pattern": {
                    "type": "string",
                    "description": "Regex pattern to search for"
                },
                "file_pattern": {
                    "type": "string",
                    "description": "File pattern to match (optional)"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of matching lines to return"
                }
            },
            "required": ["path", "pattern"]
        })
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let mut grep_input = input;
        if let Some(file_pattern) = grep_input["file_pattern"].as_str() {
            grep_input["include"] = serde_json::json!([file_pattern]);
        }
        super::grep::GrepTool::run_search(&grep_input)
    }
}
