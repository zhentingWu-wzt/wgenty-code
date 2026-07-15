//! MCP (Model Context Protocol) Module
//!
//! Complete implementation of MCP server with:
//! - Tool registration and execution
//! - Resource management
//! - Prompt system
//! - Sampling support

pub mod client;
pub mod codegraph;
pub mod prompts;
pub mod proxy;
pub mod resources;
pub mod sampling;
pub mod server;
pub mod tools;
pub mod transport;

use chrono::{DateTime, Utc};
use futures::future::join_all;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::mcp::client::{McpClientSession, McpRemoteTool};
use crate::mcp::proxy::{is_known_read_only_tool, McpToolCaller, McpToolProxy};
use crate::tools::Tool;

pub use crate::config::mcp_config::{McpConfig, McpServerStatus};
pub use codegraph::{install_state_notice, probe_install_state, CodegraphInstallState};
pub use prompts::{Prompt, PromptManager};
pub use resources::{Resource, ResourceManager};
pub use sampling::{SamplingManager, SamplingRequest};
pub use server::McpServer;
pub use tools::{McpTool, ToolRegistry};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerInfo {
    pub name: String,
    pub status: McpServerStatus,
    pub tools_count: usize,
    pub resources_count: usize,
    pub prompts_count: usize,
    pub started_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpMessage {
    pub jsonrpc: String,
    pub id: Option<i64>,
    pub method: Option<String>,
    pub params: Option<serde_json::Value>,
    pub result: Option<serde_json::Value>,
    pub error: Option<McpError>,
}

impl McpMessage {
    pub fn request(id: i64, method: &str, params: Option<serde_json::Value>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id: Some(id),
            method: Some(method.to_string()),
            params,
            result: None,
            error: None,
        }
    }

    pub fn response(id: i64, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id: Some(id),
            method: None,
            params: None,
            result: Some(result),
            error: None,
        }
    }

    pub fn error_response(id: i64, code: i32, message: &str) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id: Some(id),
            method: None,
            params: None,
            result: None,
            error: Some(McpError {
                code,
                message: message.to_string(),
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpError {
    pub code: i32,
    pub message: String,
}

pub struct McpManager {
    servers: Arc<RwLock<HashMap<String, McpServerConnection>>>,
    tool_registry: Arc<ToolRegistry>,
    resource_manager: Arc<ResourceManager>,
    prompt_manager: Arc<PromptManager>,
    sampling_manager: Arc<SamplingManager>,
}

struct McpServerConnection {
    config: McpConfig,
    session: Option<Arc<McpClientSession>>,
    tools_count: usize,
    started_at: Option<DateTime<Utc>>,
    last_error: Option<String>,
}

impl McpManager {
    pub fn new() -> Self {
        Self {
            servers: Arc::new(RwLock::new(HashMap::new())),
            tool_registry: Arc::new(ToolRegistry::new()),
            resource_manager: Arc::new(ResourceManager::new()),
            prompt_manager: Arc::new(PromptManager::new()),
            sampling_manager: Arc::new(SamplingManager::new()),
        }
    }

    pub async fn list_servers(&self) -> anyhow::Result<Vec<McpServerInfo>> {
        let settings = crate::config::Settings::load()?;
        Ok(self.list_servers_for_settings(&settings).await)
    }

    pub async fn list_servers_for_settings(
        &self,
        settings: &crate::config::Settings,
    ) -> Vec<McpServerInfo> {
        let servers = self.servers.read().await;
        let mut configs = settings.integrations.mcp_servers.clone();
        if !configs
            .iter()
            .any(|config| config.name.eq_ignore_ascii_case("codegraph"))
        {
            let mut config = McpConfig::codegraph();
            config.cwd = Some(settings.storage.working_dir.clone());
            configs.push(config);
        }
        for config in &mut configs {
            if config.name.eq_ignore_ascii_case("codegraph") && config.cwd.is_none() {
                config.cwd = Some(settings.storage.working_dir.clone());
            }
        }
        configs
            .iter()
            .map(|config| {
                let conn = servers.get(&config.name);
                McpServerInfo {
                    name: config.name.clone(),
                    status: conn.map_or(config.status.clone(), |c| c.config.status.clone()),
                    tools_count: conn.map_or(0, |c| c.tools_count),
                    resources_count: 0,
                    prompts_count: 0,
                    started_at: conn.and_then(|c| c.started_at),
                    last_error: conn.and_then(|c| c.last_error.clone()),
                }
            })
            .collect()
    }

    pub async fn add_server(&self, config: McpConfig) -> anyhow::Result<()> {
        let mut settings = crate::config::Settings::load()?;
        settings.integrations.mcp_servers.push(config);
        settings.save()?;
        Ok(())
    }

    pub async fn remove_server(&self, name: &str) -> anyhow::Result<()> {
        self.stop_server(name).await?;

        let mut settings = crate::config::Settings::load()?;
        settings.integrations.mcp_servers.retain(|s| s.name != name);
        settings.save()?;
        Ok(())
    }

    pub async fn start_server(&self, name: &str) -> anyhow::Result<()> {
        if name == "filesystem" {
            let settings = crate::config::Settings::load()?;
            let mut config = settings
                .integrations
                .mcp_servers
                .iter()
                .find(|s| s.name == name)
                .cloned()
                .unwrap_or_else(|| crate::config::McpConfig::new("filesystem", ""));

            config.status = crate::config::McpServerStatus::Running;
            let mut servers = self.servers.write().await;
            servers.insert(
                name.to_string(),
                McpServerConnection {
                    config,
                    session: None,
                    tools_count: 0,
                    started_at: Some(Utc::now()),
                    last_error: None,
                },
            );
            println!("✅ Filesystem MCP 已启动（内置模式，无需外部进程）");
            return Ok(());
        }
        let settings = crate::config::Settings::load()?;
        let config = settings
            .integrations
            .mcp_servers
            .iter()
            .find(|s| s.name == name)
            .ok_or_else(|| anyhow::anyhow!("Server not found: {}", name))?
            .clone();

        self.connect_server(&config).await.map(|_| ())
    }

    pub async fn stop_server(&self, name: &str) -> anyhow::Result<()> {
        let session = {
            let mut servers = self.servers.write().await;
            let Some(conn) = servers.get_mut(name) else {
                return Ok(());
            };
            conn.config.status = McpServerStatus::Stopped;
            conn.session.take()
        };
        if let Some(session) = session {
            session.shutdown().await?;
        }
        println!("🛑 MCP server stopped: {}", name);
        Ok(())
    }

    pub async fn restart_server(&self, name: &str) -> anyhow::Result<()> {
        self.stop_server(name).await?;
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        self.start_server(name).await
    }

    pub async fn start_all(&self) -> anyhow::Result<()> {
        let settings = crate::config::Settings::load()?;
        for server in &settings.integrations.mcp_servers {
            if server.auto_start {
                let _ = self.start_server(&server.name).await;
            }
        }
        Ok(())
    }

    /// Connect configured MCP servers and return remote tools ready to install
    /// into the agent's native tool registry. Individual server failures are
    /// recorded and logged but do not prevent the coding agent from starting.
    pub async fn connect_configured_tools(
        &self,
        settings: &crate::config::Settings,
        reserved_names: &mut HashSet<String>,
    ) -> Vec<Box<dyn Tool>> {
        let mut configs = settings.integrations.mcp_servers.clone();
        if !configs
            .iter()
            .any(|config| config.name.eq_ignore_ascii_case("codegraph"))
        {
            let mut config = McpConfig::codegraph();
            config.cwd = Some(settings.storage.working_dir.clone());
            configs.push(config);
        }
        for config in &mut configs {
            if config.name.eq_ignore_ascii_case("codegraph") && config.cwd.is_none() {
                config.cwd = Some(settings.storage.working_dir.clone());
            }
        }

        // Probe CodeGraph availability. Skip the spawn entirely when the binary
        // is absent or the user dismissed guidance -- `codegraph serve` would
        // fail with "command not found" (fast but noisy) or run unwanted.
        let skip_codegraph =
            codegraph::should_skip_codegraph(codegraph::probe_install_state(settings));

        // Connect to all auto-start MCP servers concurrently. The slow part of
        // each connection (subprocess spawn + initialize + tools/list, each
        // bounded by a 15s timeout) runs in parallel, so total connect time
        // approaches the max individual time instead of the sum. Proxy building
        // (name reservation) is done serially afterward - it's fast and needs
        // exclusive access to `reserved_names`.
        let auto_start_configs: Vec<&McpConfig> = configs
            .iter()
            .filter(|config| config.auto_start && config.name != "filesystem")
            .filter(|config| !(skip_codegraph && config.name.eq_ignore_ascii_case("codegraph")))
            .collect();
        let connect_futures: Vec<_> = auto_start_configs
            .iter()
            .map(|config| async move { (config.name.clone(), self.connect_server(config).await) })
            .collect();
        let results = join_all(connect_futures).await;

        let mut proxies = Vec::new();
        for (name, result) in results {
            match result {
                Ok((session, tools)) => {
                    let caller: Arc<dyn McpToolCaller> = session;
                    proxies.extend(build_tool_proxies(&name, tools, caller, reserved_names));
                }
                Err(error) => {
                    tracing::warn!(
                        server = %name,
                        error = %error,
                        "MCP server unavailable; continuing without its tools"
                    );
                }
            }
        }
        proxies
    }

    async fn connect_server(
        &self,
        config: &McpConfig,
    ) -> anyhow::Result<(Arc<McpClientSession>, Vec<McpRemoteTool>)> {
        let result = async {
            let session = tokio::time::timeout(
                std::time::Duration::from_secs(15),
                McpClientSession::spawn(config),
            )
            .await
            .map_err(|_| {
                anyhow::anyhow!("MCP server `{}` initialization timed out", config.name)
            })??;
            let tools =
                tokio::time::timeout(std::time::Duration::from_secs(15), session.list_tools())
                    .await
                    .map_err(|_| {
                        anyhow::anyhow!("MCP server `{}` tools/list timed out", config.name)
                    })??;
            Ok::<_, anyhow::Error>((session, tools))
        }
        .await;

        match result {
            Ok((session, tools)) => {
                self.servers.write().await.insert(
                    config.name.clone(),
                    McpServerConnection {
                        config: McpConfig {
                            status: McpServerStatus::Running,
                            ..config.clone()
                        },
                        session: Some(session.clone()),
                        tools_count: tools.len(),
                        started_at: Some(Utc::now()),
                        last_error: None,
                    },
                );
                Ok((session, tools))
            }
            Err(error) => {
                self.servers.write().await.insert(
                    config.name.clone(),
                    McpServerConnection {
                        config: McpConfig {
                            status: McpServerStatus::Error,
                            ..config.clone()
                        },
                        session: None,
                        tools_count: 0,
                        started_at: None,
                        last_error: Some(format!("{error:#}")),
                    },
                );
                Err(error)
            }
        }
    }

    pub async fn stop_all(&self) -> anyhow::Result<()> {
        let servers = self.servers.read().await;
        let names: Vec<String> = servers.keys().cloned().collect();
        drop(servers);

        for name in names {
            self.stop_server(&name).await?;
        }
        Ok(())
    }

    pub fn tool_registry(&self) -> Arc<ToolRegistry> {
        self.tool_registry.clone()
    }

    pub fn resource_manager(&self) -> Arc<ResourceManager> {
        self.resource_manager.clone()
    }

    pub fn prompt_manager(&self) -> Arc<PromptManager> {
        self.prompt_manager.clone()
    }

    pub fn sampling_manager(&self) -> Arc<SamplingManager> {
        self.sampling_manager.clone()
    }
}

fn build_tool_proxies(
    server_name: &str,
    tools: Vec<McpRemoteTool>,
    caller: Arc<dyn McpToolCaller>,
    reserved_names: &mut HashSet<String>,
) -> Vec<Box<dyn Tool>> {
    tools
        .into_iter()
        .map(|tool| {
            let exposed_name = if reserved_names.insert(tool.name.clone()) {
                tool.name.clone()
            } else {
                let prefixed = format!("{server_name}__{}", tool.name);
                reserved_names.insert(prefixed.clone());
                prefixed
            };
            Box::new(McpToolProxy::new(
                server_name.to_string(),
                exposed_name,
                tool.name.clone(),
                tool.description,
                tool.input_schema,
                is_known_read_only_tool(server_name, &tool.name),
                caller.clone(),
            )) as Box<dyn Tool>
        })
        .collect()
}

impl Default for McpManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod external_registration_tests {
    use super::*;
    use crate::mcp::client::McpRemoteTool;
    use crate::mcp::proxy::McpToolCaller;
    use serde_json::json;

    struct NoopCaller;

    #[async_trait::async_trait]
    impl McpToolCaller for NoopCaller {
        async fn call_tool(
            &self,
            _name: &str,
            _arguments: serde_json::Value,
        ) -> anyhow::Result<serde_json::Value> {
            Ok(json!({"content": []}))
        }
    }

    #[test]
    fn builds_read_only_codegraph_proxies_with_standard_names() {
        let tools = vec![McpRemoteTool {
            name: "codegraph_node".to_string(),
            description: "Find symbols".to_string(),
            input_schema: json!({"type": "object"}),
        }];

        let proxies = build_tool_proxies(
            "codegraph",
            tools,
            std::sync::Arc::new(NoopCaller),
            &mut std::collections::HashSet::new(),
        );
        assert_eq!(proxies.len(), 1);
        assert_eq!(proxies[0].name(), "codegraph_node");
        assert!(proxies[0].is_read_only());
    }
}
