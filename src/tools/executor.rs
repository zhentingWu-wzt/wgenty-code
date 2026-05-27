use crate::api::ChatMessage;
use crate::tools::ToolRegistry;
use std::sync::Arc;

pub struct ToolExecutor {
    registry: Arc<ToolRegistry>,
}

impl ToolExecutor {
    pub fn new(registry: Arc<ToolRegistry>) -> Self {
        Self { registry }
    }

    pub fn tool_definitions(&self) -> Vec<crate::api::ToolDefinition> {
        self.registry
            .list()
            .into_iter()
            .map(|t| {
                crate::api::ToolDefinition::new(t.name(), t.description(), t.input_schema())
            })
            .collect()
    }

    pub async fn execute_tool_call(
        &self,
        tool_call_id: &str,
        tool_name: &str,
        args: serde_json::Value,
    ) -> ChatMessage {
        let result = self.registry.execute(tool_name, args).await;
        let content = match result {
            Ok(result) => serde_json::json!({
                "success": true,
                "output_type": result.output_type,
                "content": result.content,
                "metadata": result.metadata
            })
            .to_string(),
            Err(e) => serde_json::json!({
                "success": false,
                "error": {
                    "message": e.message,
                    "code": e.code
                }
            })
            .to_string(),
        };

        ChatMessage::tool(tool_call_id, content)
    }
}
