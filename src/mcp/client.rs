use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicI64, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

const MCP_PROTOCOL_VERSION: &str = "2024-11-05";

/// Metadata advertised by a remote MCP server for one tool.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct McpRemoteTool {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(rename = "inputSchema", alias = "input_schema")]
    pub input_schema: Value,
}

#[derive(Debug, Deserialize)]
struct McpToolsListResult {
    tools: Vec<McpRemoteTool>,
}

struct McpTransportIo<R, W> {
    reader: BufReader<R>,
    writer: W,
}

/// Serialized JSON-RPC transport used by MCP stdio servers.
///
/// MCP stdio messages are single-line JSON values. Requests are serialized so
/// one caller cannot consume another caller's response from the shared stdout.
pub struct McpJsonLineTransport<R, W> {
    io: Mutex<McpTransportIo<R, W>>,
    next_id: AtomicI64,
}

/// Live MCP session backed by one stdio child process.
pub struct McpClientSession {
    transport: McpJsonLineTransport<ChildStdout, ChildStdin>,
    child: Mutex<Child>,
}

impl McpClientSession {
    /// Spawn and initialize an MCP stdio server from application settings.
    pub async fn spawn(config: &crate::config::McpConfig) -> Result<std::sync::Arc<Self>> {
        if config.command.trim().is_empty() {
            anyhow::bail!("MCP server `{}` has an empty command", config.name);
        }

        let mut command = Command::new(&config.command);
        command
            .args(&config.args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .kill_on_drop(true);
        if let Some(cwd) = &config.cwd {
            command.current_dir(cwd);
        }
        command.envs(&config.env);

        let mut child = command.spawn().with_context(|| {
            format!(
                "start MCP server `{}` with command `{}`",
                config.name, config.command
            )
        })?;
        let stdin = child
            .stdin
            .take()
            .with_context(|| format!("capture stdin for MCP server `{}`", config.name))?;
        let stdout = child
            .stdout
            .take()
            .with_context(|| format!("capture stdout for MCP server `{}`", config.name))?;

        let session = std::sync::Arc::new(Self {
            transport: McpJsonLineTransport::new(stdout, stdin),
            child: Mutex::new(child),
        });
        session
            .transport
            .initialize()
            .await
            .with_context(|| format!("initialize MCP server `{}`", config.name))?;
        Ok(session)
    }

    pub async fn list_tools(&self) -> Result<Vec<McpRemoteTool>> {
        self.transport.list_tools().await
    }

    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<Value> {
        self.transport.call_tool(name, arguments).await
    }

    pub async fn shutdown(&self) -> Result<()> {
        let mut child = self.child.lock().await;
        match child
            .try_wait()
            .context("check MCP server process status")?
        {
            Some(_) => Ok(()),
            None => {
                child.kill().await.context("stop MCP server process")?;
                child.wait().await.context("wait for MCP server process")?;
                Ok(())
            }
        }
    }
}

impl<R, W> McpJsonLineTransport<R, W>
where
    R: AsyncRead + Unpin + Send,
    W: AsyncWrite + Unpin + Send,
{
    pub fn new(reader: R, writer: W) -> Self {
        Self {
            io: Mutex::new(McpTransportIo {
                reader: BufReader::new(reader),
                writer,
            }),
            next_id: AtomicI64::new(1),
        }
    }

    /// Perform the MCP initialization handshake.
    pub async fn initialize(&self) -> Result<()> {
        self.request(
            "initialize",
            json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {
                    "name": "wgenty-code",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }),
        )
        .await?;

        self.notify("notifications/initialized", json!({}))
            .await
            .context("notify MCP server that initialization completed")
    }

    /// Discover all tools exposed by the server.
    pub async fn list_tools(&self) -> Result<Vec<McpRemoteTool>> {
        let result = self.request("tools/list", json!({})).await?;
        let parsed: McpToolsListResult =
            serde_json::from_value(result).context("decode MCP tools/list response payload")?;
        Ok(parsed.tools)
    }

    /// Invoke one remote MCP tool.
    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<Value> {
        self.request(
            "tools/call",
            json!({
                "name": name,
                "arguments": arguments
            }),
        )
        .await
    }

    async fn notify(&self, method: &str, params: Value) -> Result<()> {
        let message = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        });
        let mut io = self.io.lock().await;
        write_message(&mut io.writer, &message)
            .await
            .with_context(|| format!("write MCP notification `{method}`"))
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });

        let mut io = self.io.lock().await;
        write_message(&mut io.writer, &request)
            .await
            .with_context(|| format!("write MCP request `{method}`"))?;

        loop {
            let mut line = String::new();
            let bytes = io
                .reader
                .read_line(&mut line)
                .await
                .with_context(|| format!("read MCP response for `{method}`"))?;
            if bytes == 0 {
                anyhow::bail!("MCP server closed stdout while waiting for `{method}`");
            }

            let response: Value = serde_json::from_str(line.trim_end())
                .with_context(|| format!("decode MCP response for `{method}`"))?;
            if response.get("id").and_then(Value::as_i64) != Some(id) {
                // Notifications and unrelated server messages do not complete
                // this serialized client request.
                continue;
            }

            if let Some(error) = response.get("error") {
                let code = error.get("code").and_then(Value::as_i64).unwrap_or(0);
                let message = error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown MCP error");
                anyhow::bail!("MCP `{method}` failed ({code}): {message}");
            }

            return response
                .get("result")
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("MCP `{method}` response omitted `result`"));
        }
    }
}

async fn write_message<W>(writer: &mut W, message: &Value) -> Result<()>
where
    W: AsyncWrite + Unpin,
{
    let mut bytes = serde_json::to_vec(message).context("encode MCP JSON-RPC message")?;
    bytes.push(b'\n');
    writer
        .write_all(&bytes)
        .await
        .context("write MCP JSON-RPC message bytes")?;
    writer.flush().await.context("flush MCP JSON-RPC message")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio::io::{duplex, AsyncBufReadExt, AsyncWriteExt, BufReader};

    #[tokio::test]
    async fn initializes_lists_and_calls_tools_over_json_lines() {
        let (client_stream, server_stream) = duplex(16 * 1024);
        let (client_read, client_write) = tokio::io::split(client_stream);
        let (server_read, mut server_write) = tokio::io::split(server_stream);

        let server = tokio::spawn(async move {
            let mut lines = BufReader::new(server_read).lines();

            let initialize: serde_json::Value = serde_json::from_str(
                &lines
                    .next_line()
                    .await
                    .unwrap()
                    .expect("initialize request"),
            )
            .unwrap();
            assert_eq!(initialize["method"], "initialize");
            let initialize_id = initialize["id"].as_i64().unwrap();
            server_write
                .write_all(
                    format!(
                        "{}\n",
                        json!({
                            "jsonrpc": "2.0",
                            "id": initialize_id,
                            "result": {
                                "protocolVersion": "2024-11-05",
                                "capabilities": {"tools": {}},
                                "serverInfo": {"name": "test", "version": "1.0"}
                            }
                        })
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();

            let initialized: serde_json::Value = serde_json::from_str(
                &lines
                    .next_line()
                    .await
                    .unwrap()
                    .expect("initialized notification"),
            )
            .unwrap();
            assert_eq!(initialized["method"], "notifications/initialized");

            let list: serde_json::Value =
                serde_json::from_str(&lines.next_line().await.unwrap().expect("tools/list"))
                    .unwrap();
            assert_eq!(list["method"], "tools/list");
            let list_id = list["id"].as_i64().unwrap();
            server_write
                .write_all(
                    format!(
                        "{}\n",
                        json!({
                            "jsonrpc": "2.0",
                            "id": list_id,
                            "result": {
                                "tools": [{
                                    "name": "echo",
                                    "description": "Echo input",
                                    "inputSchema": {
                                        "type": "object",
                                        "properties": {"text": {"type": "string"}},
                                        "required": ["text"]
                                    }
                                }]
                            }
                        })
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();

            let call: serde_json::Value =
                serde_json::from_str(&lines.next_line().await.unwrap().expect("tools/call"))
                    .unwrap();
            assert_eq!(call["method"], "tools/call");
            assert_eq!(call["params"]["name"], "echo");
            assert_eq!(call["params"]["arguments"], json!({"text": "hello"}));
            let call_id = call["id"].as_i64().unwrap();
            server_write
                .write_all(
                    format!(
                        "{}\n",
                        json!({
                            "jsonrpc": "2.0",
                            "id": call_id,
                            "result": {
                                "content": [{"type": "text", "text": "hello"}]
                            }
                        })
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });

        let transport = McpJsonLineTransport::new(client_read, client_write);
        transport.initialize().await.unwrap();

        let tools = transport.list_tools().await.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "echo");
        assert_eq!(tools[0].description, "Echo input");

        let result = transport
            .call_tool("echo", json!({"text": "hello"}))
            .await
            .unwrap();
        assert_eq!(result["content"][0]["text"], "hello");

        server.await.unwrap();
    }

    #[tokio::test]
    async fn surfaces_json_rpc_errors_with_context() {
        let (client_stream, server_stream) = duplex(4096);
        let (client_read, client_write) = tokio::io::split(client_stream);
        let (server_read, mut server_write) = tokio::io::split(server_stream);

        let server = tokio::spawn(async move {
            let mut lines = BufReader::new(server_read).lines();
            let request: serde_json::Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            server_write
                .write_all(
                    format!(
                        "{}\n",
                        json!({
                            "jsonrpc": "2.0",
                            "id": request["id"],
                            "error": {"code": -32601, "message": "missing method"}
                        })
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });

        let transport = McpJsonLineTransport::new(client_read, client_write);
        let error = transport.list_tools().await.unwrap_err().to_string();
        assert!(error.contains("tools/list"));
        assert!(error.contains("missing method"));
        server.await.unwrap();
    }
}
