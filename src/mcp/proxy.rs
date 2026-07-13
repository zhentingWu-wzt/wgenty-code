use crate::mcp::client::McpClientSession;
use crate::tools::{Tool, ToolError, ToolOutput};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

/// Minimal remote invocation surface used by MCP-backed tools.
#[async_trait]
pub trait McpToolCaller: Send + Sync {
    async fn call_tool(&self, name: &str, arguments: Value) -> Result<Value>;
}

#[async_trait]
impl McpToolCaller for McpClientSession {
    async fn call_tool(&self, name: &str, arguments: Value) -> Result<Value> {
        McpClientSession::call_tool(self, name, arguments).await
    }
}

/// Adapter that makes a remote MCP tool indistinguishable from a built-in tool
/// to the agent loop.
pub struct McpToolProxy {
    server_name: String,
    exposed_name: String,
    remote_name: String,
    description: String,
    input_schema: Value,
    read_only: bool,
    caller: Arc<dyn McpToolCaller>,
}

impl McpToolProxy {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        server_name: String,
        exposed_name: String,
        remote_name: String,
        description: String,
        input_schema: Value,
        read_only: bool,
        caller: Arc<dyn McpToolCaller>,
    ) -> Self {
        Self {
            server_name,
            exposed_name,
            remote_name,
            description,
            input_schema,
            read_only,
            caller,
        }
    }
}

#[async_trait]
impl Tool for McpToolProxy {
    fn name(&self) -> &str {
        &self.exposed_name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn input_schema(&self) -> Value {
        self.input_schema.clone()
    }

    fn is_read_only(&self) -> bool {
        self.read_only
    }

    fn requires_confirmation(&self) -> bool {
        !self.read_only
    }

    async fn execute(&self, input: Value) -> Result<ToolOutput, ToolError> {
        let result = self
            .caller
            .call_tool(&self.remote_name, input)
            .await
            .map_err(|error| ToolError {
                message: format!(
                    "MCP server `{}` failed to execute `{}`: {error:#}",
                    self.server_name, self.remote_name
                ),
                code: Some("mcp_tool_error".to_string()),
            })?;

        let content = flatten_mcp_content(&result);
        if result.get("isError").and_then(Value::as_bool) == Some(true) {
            return Err(ToolError {
                message: format!(
                    "MCP server `{}` reported an error for `{}`: {}",
                    self.server_name, self.remote_name, content
                ),
                code: Some("mcp_tool_error".to_string()),
            });
        }

        let mut metadata = HashMap::new();
        metadata.insert(
            "mcp_server".to_string(),
            Value::String(self.server_name.clone()),
        );
        metadata.insert(
            "mcp_tool".to_string(),
            Value::String(self.remote_name.clone()),
        );
        if let Some(structured) = result.get("structuredContent") {
            metadata.insert("structured_content".to_string(), structured.clone());
        }

        Ok(ToolOutput {
            output_type: "text".to_string(),
            content,
            metadata,
        })
    }
}

fn flatten_mcp_content(result: &Value) -> String {
    let Some(items) = result.get("content").and_then(Value::as_array) else {
        return result.to_string();
    };

    items
        .iter()
        .map(|item| {
            if item.get("type").and_then(Value::as_str) == Some("text") {
                item.get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string()
            } else {
                item.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// CodeGraph's query surface is local and observational. Unknown MCP tools are
/// deliberately not classified as read-only.
pub fn is_known_read_only_tool(server_name: &str, tool_name: &str) -> bool {
    server_name.eq_ignore_ascii_case("codegraph")
        && matches!(
            tool_name,
            "codegraph_explore"
                | "codegraph_node"
                | "codegraph_search"
                | "codegraph_callers"
                | "codegraph_callees"
                | "codegraph_impact"
                | "codegraph_files"
                | "codegraph_status"
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::Tool;
    use serde_json::json;
    use std::sync::Mutex;

    struct FakeCaller {
        calls: Mutex<Vec<(String, serde_json::Value)>>,
        result: serde_json::Value,
    }

    #[async_trait::async_trait]
    impl McpToolCaller for FakeCaller {
        async fn call_tool(
            &self,
            name: &str,
            arguments: serde_json::Value,
        ) -> anyhow::Result<serde_json::Value> {
            self.calls
                .lock()
                .expect("fake caller lock")
                .push((name.to_string(), arguments));
            Ok(self.result.clone())
        }
    }

    #[tokio::test]
    async fn exposes_remote_metadata_and_flattens_text_content() {
        let caller = std::sync::Arc::new(FakeCaller {
            calls: Mutex::new(Vec::new()),
            result: json!({
                "content": [
                    {"type": "text", "text": "first"},
                    {"type": "text", "text": "second"}
                ]
            }),
        });
        let proxy = McpToolProxy::new(
            "codegraph".to_string(),
            "codegraph_node".to_string(),
            "codegraph_node".to_string(),
            "Find a symbol".to_string(),
            json!({"type": "object"}),
            true,
            caller.clone(),
        );

        assert_eq!(proxy.name(), "codegraph_node");
        assert_eq!(proxy.description(), "Find a symbol");
        assert_eq!(proxy.input_schema(), json!({"type": "object"}));
        assert!(proxy.is_read_only());

        let output = proxy.execute(json!({"symbol": "Tool"})).await.unwrap();
        assert_eq!(output.content, "first\nsecond");
        assert_eq!(
            caller.calls.lock().unwrap().as_slice(),
            &[("codegraph_node".to_string(), json!({"symbol": "Tool"}))]
        );
    }

    #[tokio::test]
    async fn reports_remote_tool_errors() {
        struct FailingCaller;

        #[async_trait::async_trait]
        impl McpToolCaller for FailingCaller {
            async fn call_tool(
                &self,
                _name: &str,
                _arguments: serde_json::Value,
            ) -> anyhow::Result<serde_json::Value> {
                anyhow::bail!("remote unavailable")
            }
        }

        let proxy = McpToolProxy::new(
            "broken".to_string(),
            "remote".to_string(),
            "remote".to_string(),
            String::new(),
            json!({}),
            false,
            std::sync::Arc::new(FailingCaller),
        );
        let error = proxy.execute(json!({})).await.unwrap_err();
        assert_eq!(error.code.as_deref(), Some("mcp_tool_error"));
        assert!(error.message.contains("broken"));
        assert!(error.message.contains("remote unavailable"));
    }

    #[test]
    fn only_known_codegraph_queries_are_read_only() {
        assert!(is_known_read_only_tool("codegraph", "codegraph_node"));
        assert!(is_known_read_only_tool("CodeGraph", "codegraph_explore"));
        assert!(!is_known_read_only_tool("other", "codegraph_node"));
        assert!(!is_known_read_only_tool("codegraph", "unknown_mutation"));
    }
}
