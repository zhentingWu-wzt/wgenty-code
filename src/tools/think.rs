//! Think Tool — reasoning scratchpad for the model
//!
//! This tool gives the LLM a dedicated slot to reflect, plan, or evaluate
//! before committing to an action. It is purely a "no-op" execution — the
//! value is in the model using it to structure its own thinking.

use super::{Tool, ToolError, ToolOutput};
use async_trait::async_trait;
use std::collections::HashMap;

pub struct ThinkTool;

impl ThinkTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ThinkTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ThinkTool {
    fn name(&self) -> &str {
        "think"
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn description(&self) -> &str {
        "A thinking scratchpad for reasoning through complex problems before taking action. Use this to plan, evaluate trade-offs, or break down a multi-step task. The thought content is visible in the conversation history but takes no action."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "thought": {
                    "type": "string",
                    "description": "Your reasoning or plan. Be detailed — explain your approach, trade-offs, and what you intend to do."
                }
            },
            "required": ["thought"]
        })
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let thought = input["thought"].as_str().unwrap_or("").to_string();

        let mut metadata = HashMap::new();
        metadata.insert("thought".to_string(), serde_json::json!(thought));

        Ok(ToolOutput {
            output_type: "text".to_string(),
            content: format!("_thinking: {}_", thought),
            metadata,
        })
    }
}
