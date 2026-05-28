//! Compact Tool — trigger context compaction in the agent loop
//!
//! When the agent calls this tool, the TypeScript agent loop intercepts the
//! call and performs conversation compaction: the full transcript is archived
//! to disk and the conversation history is replaced with a summary.
//! The Rust side simply acknowledges the request.

use crate::tools::{Tool, ToolError, ToolOutput};
use async_trait::async_trait;
use std::collections::HashMap;

pub struct CompactTool;

impl CompactTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for CompactTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for CompactTool {
    fn name(&self) -> &str {
        "compact"
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn description(&self) -> &str {
        "Compress conversation history to save context. Archives the full transcript to disk and replaces it with a generated summary. Use this when the conversation is getting long or you need to free up context window space."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn execute(&self, _input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput {
            output_type: "text".to_string(),
            content: "Compaction triggered. The conversation history will be archived and compressed to save context. The agent loop will handle the actual compaction.".to_string(),
            metadata: HashMap::new(),
        })
    }
}
