//! MCP Tools - Tool registration and execution

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    pub server_name: Option<String>,
}

impl McpTool {
    pub fn new(name: &str, description: &str, input_schema: serde_json::Value) -> Self {
        Self {
            name: name.to_string(),
            description: description.to_string(),
            input_schema,
            server_name: None,
        }
    }

    pub fn with_server(mut self, server_name: &str) -> Self {
        self.server_name = Some(server_name.to_string());
        self
    }
}

pub struct ToolRegistry {
    tools: Arc<RwLock<HashMap<String, McpTool>>>,
    local_registry: Arc<crate::tools::ToolRegistry>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: Arc::new(RwLock::new(HashMap::new())),
            local_registry: Arc::new(crate::tools::ToolRegistry::new()),
        }
    }

    pub async fn unregister(&self, name: &str) {
        let mut tools = self.tools.write().await;
        tools.remove(name);
    }

    pub async fn get(&self, name: &str) -> Option<McpTool> {
        let tools = self.tools.read().await;
        tools.get(name).cloned()
    }

    pub async fn list(&self) -> Vec<McpTool> {
        let tools = self.tools.read().await;
        tools.values().cloned().collect()
    }

    pub async fn execute(
        &self,
        name: &str,
        params: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let tool = self
            .local_registry
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("Tool not found: {}", name))?;

        match tool.execute(params).await {
            Ok(result) => Ok(serde_json::json!({
                "success": true,
                "output_type": result.output_type,
                "content": result.content,
                "metadata": result.metadata
            })),
            Err(error) => Ok(serde_json::json!({
                "success": false,
                "error": {
                    "message": error.message,
                    "code": error.code
                }
            })),
        }
    }

    pub async fn register_builtin_tools(&self) {
        let mut tools = self.tools.write().await;
        tools.clear();

        for tool in self.local_registry.list() {
            tools.insert(
                tool.name().to_string(),
                McpTool::new(tool.name(), tool.description(), tool.input_schema()),
            );
        }
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}
