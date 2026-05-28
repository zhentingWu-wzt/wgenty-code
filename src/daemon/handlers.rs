//! HTTP request handlers for the daemon API.

use crate::api::{ApiClient, ToolDefinition};
use crate::daemon::models::*;
use crate::daemon::state::DaemonState;
use crate::permissions::PolicyDecision;
use axum::{
    extract::State,
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive},
        Json, Sse,
    },
};
use futures::StreamExt;
use std::convert::Infallible;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tokio_stream::Stream;
use tracing::error;

// ── Health ───────────────────────────────────────────────────────────────────

pub async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

// ── Config ───────────────────────────────────────────────────────────────────

pub async fn get_config(State(state): State<Arc<DaemonState>>) -> Json<ConfigResponse> {
    let s = &state.app_state.settings;
    Json(ConfigResponse {
        model: s.model.clone(),
        api_base: s.api.base_url.clone(),
        max_tokens: s.api.max_tokens,
        timeout: s.api.timeout,
        streaming: s.api.streaming,
    })
}

// ── Chat / Stream ────────────────────────────────────────────────────────────

pub async fn chat_stream(
    State(state): State<Arc<DaemonState>>,
    Json(body): Json<ChatStreamRequest>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let settings = state.app_state.settings.clone();
    let client = ApiClient::new(settings);

    // Build messages and tools
    let messages = body.messages;
    let tools: Option<Vec<ToolDefinition>> = {
        let defs = state.tool_executor.tool_definitions();
        if defs.is_empty() {
            None
        } else {
            Some(defs)
        }
    };

    let (tx, rx) = mpsc::unbounded_channel::<Result<Event, Infallible>>();

    tokio::spawn(async move {
        // Make the API call
        let response = match client.chat_stream(messages, tools).await {
            Ok(r) => r,
            Err(e) => {
                let _ = tx.send(Ok(Event::default().data(format!(
                    r#"{{"error":"{}"}}"#,
                    e
                ))));
                return;
            }
        };

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let _ = tx.send(Ok(Event::default().data(format!(
                r#"{{"error":"API error ({}): {}"}}"#,
                status, body
            ))));
            return;
        }

        // Stream SSE chunks back to the client
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    let text = String::from_utf8_lossy(&bytes);
                    for line in text.lines() {
                        let line = line.trim();
                        if line.is_empty() {
                            continue;
                        }
                        // Upstream already formats SSE as "data: {...}" or "[DONE]";
                        // strip prefix so we don't double-wrap.
                        let payload = line
                            .strip_prefix("data: ")
                            .unwrap_or(line);
                        let _ = tx.send(Ok(Event::default().data(payload)));
                    }
                }
                Err(e) => {
                    error!(error = %e, "stream chunk error");
                    break;
                }
            }
        }

        // Signal done
        let _ = tx.send(Ok(Event::default().data("[DONE]")));
    });

    Sse::new(UnboundedReceiverStream::new(rx)).keep_alive(KeepAlive::default())
}

// ── Tools ────────────────────────────────────────────────────────────────────

pub async fn list_tools(State(state): State<Arc<DaemonState>>) -> Json<ListToolsResponse> {
    let tools: Vec<ToolInfo> = state
        .tool_registry
        .list()
        .into_iter()
        .map(|t| ToolInfo {
            name: t.name().to_string(),
            description: t.description().to_string(),
            input_schema: t.input_schema(),
            is_read_only: t.is_read_only(),
        })
        .collect();

    Json(ListToolsResponse { tools })
}

pub async fn execute_tool(
    State(state): State<Arc<DaemonState>>,
    Json(body): Json<ExecuteToolRequest>,
) -> Result<Json<ExecuteToolResponse>, StatusCode> {
    let tool_name = &body.tool_name;
    let args = &body.arguments;
    let session_id = body.session_id.as_deref().unwrap_or("default");

    // Validate against policy
    match state
        .tool_executor
        .validate_tool_call(tool_name, args)
        .await
    {
        Ok(PolicyDecision::Allow) => {
            // Execute directly
            let msg = state
                .tool_executor
                .execute_tool_call("api", tool_name, args.clone())
                .await;
            let content = msg.content.unwrap_or_default();
            let parsed: serde_json::Value =
                serde_json::from_str(&content).unwrap_or_default();

            Ok(Json(ExecuteToolResponse {
                success: parsed["success"].as_bool().unwrap_or(false),
                output_type: parsed["output_type"].as_str().map(|s| s.to_string()),
                content: parsed["content"].as_str().map(|s| s.to_string()),
                metadata: parsed.get("metadata").cloned(),
                permission_required: None,
            }))
        }
        Ok(PolicyDecision::Ask(req)) => {
            // Check if rule was already approved for this session
            if state.is_rule_approved(session_id, &req.session_rule).await {
                let msg = state
                    .tool_executor
                    .execute_tool_call("api", tool_name, args.clone())
                    .await;
                let content = msg.content.unwrap_or_default();
                let parsed: serde_json::Value =
                    serde_json::from_str(&content).unwrap_or_default();

                return Ok(Json(ExecuteToolResponse {
                    success: parsed["success"].as_bool().unwrap_or(false),
                    output_type: parsed["output_type"].as_str().map(|s| s.to_string()),
                    content: parsed["content"].as_str().map(|s| s.to_string()),
                    metadata: parsed.get("metadata").cloned(),
                    permission_required: None,
                }));
            }

            // Need permission from user
            Ok(Json(ExecuteToolResponse {
                success: false,
                output_type: None,
                content: None,
                metadata: None,
                permission_required: Some(PermissionRequiredInfo {
                    reason: req.reason,
                    session_rule: req.session_rule,
                }),
            }))
        }
        Err(e) => Ok(Json(ExecuteToolResponse {
            success: false,
            output_type: None,
            content: Some(format!("{}: {}", e.code.as_deref().unwrap_or("error"), e.message)),
            metadata: None,
            permission_required: None,
        })),
    }
}

pub async fn approve_tool(
    State(state): State<Arc<DaemonState>>,
    Json(body): Json<ApproveToolRequest>,
) -> Json<serde_json::Value> {
    state
        .tool_executor
        .approve_rule(body.session_rule.clone())
        .await;
    state
        .approve_rule("default", body.session_rule)
        .await;

    Json(serde_json::json!({"success": true}))
}

// ── MCP ──────────────────────────────────────────────────────────────────────

pub async fn list_mcp_servers(
    State(state): State<Arc<DaemonState>>,
) -> Json<ListMcpServersResponse> {
    let servers: Vec<McpServerInfo> = state
        .app_state
        .settings
        .mcp_servers
        .iter()
        .map(|cfg| McpServerInfo {
            name: cfg.name.clone(),
            status: format!("{:?}", cfg.status),
            tools_count: 0,
            resources_count: 0,
        })
        .collect();

    Json(ListMcpServersResponse { servers })
}
