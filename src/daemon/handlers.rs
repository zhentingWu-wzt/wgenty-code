//! HTTP request handlers for the daemon API.

use crate::api::{ApiClient, ToolDefinition};
use crate::daemon::models::*;
use crate::daemon::state::DaemonState;
use crate::permissions::PolicyDecision;
use crate::tasks::management::{TaskPriority, TaskStatus};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive},
        Json, Sse,
    },
};
use futures::StreamExt;
use std::collections::HashMap;
use std::convert::Infallible;
use std::io::Write;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tokio_stream::Stream;
use tracing::error;

fn debug_log(msg: &str) {
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/wgenty-code-debug.log")
    {
        let _ = writeln!(f, "{}", msg);
    }
}

/// Format an error with its full cause chain.
///
/// reqwest's `Display` only prints the outer kind (e.g. "error decoding
/// response body") and silently drops the actual cause - timeout vs.
/// connection reset vs. HTTP/2 stream error - which lives in
/// `std::error::Error::source()`. This walks the chain so the real reason a
/// stream was interrupted is visible in logs and in the error payload sent to
/// the client.
fn format_error_chain(e: &dyn std::error::Error) -> String {
    let mut s = e.to_string();
    let mut current = e.source();
    while let Some(cause) = current {
        let cause_str = cause.to_string();
        if !cause_str.is_empty() {
            s.push_str(": ");
            s.push_str(&cause_str);
        }
        current = cause.source();
    }
    s
}

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
        model: s.models.main.name.clone(),
        api_base: s.models.main.endpoint_base_url(),
        max_tokens: s.models.transport.max_tokens,
        timeout: s.models.transport.timeout,
        streaming: s.models.transport.streaming,
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
    let tools: Option<Vec<ToolDefinition>> = if body.plan_mode.unwrap_or(false) {
        None
    } else {
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
                let error_json = serde_json::json!({"error": e.to_string()}).to_string();
                let _ = tx.send(Ok(Event::default().data(error_json)));
                return;
            }
        };

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let error_json = serde_json::json!({
                "error": crate::api::format_api_error(status, &body)
            })
            .to_string();
            let _ = tx.send(Ok(Event::default().data(error_json)));
            return;
        }

        // Stream SSE chunks back to the client.
        // Use a buffer to handle chunk boundaries — a TCP chunk may split an SSE line
        // in the middle, and String::lines() would discard the partial fragment.
        let mut stream = response.bytes_stream();
        let mut stream_error: Option<String> = None;
        let mut buffer = String::new();
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    buffer.push_str(&String::from_utf8_lossy(&bytes));
                    // Extract complete lines; keep the trailing partial line in buffer
                    while let Some(idx) = buffer.find('\n') {
                        let line = buffer[..idx].trim().to_string();
                        buffer = buffer[idx + 1..].to_string();
                        if line.is_empty() {
                            continue;
                        }
                        // Upstream already formats SSE as "data: {...}" or "[DONE]";
                        // strip prefix so we don't double-wrap.
                        let payload = line.strip_prefix("data: ").unwrap_or(&line);
                        let _ = tx.send(Ok(Event::default().data(payload)));
                    }
                }
                Err(e) => {
                    // reqwest's `Display` only prints the outer kind ("error
                    // decoding response body") and drops the real cause
                    // (timeout vs. connection reset vs. h2 error) that lives in
                    // `Error::source()`. Walk the chain so it's visible in both
                    // the log and the SSE error payload sent to the client.
                    let chain = format_error_chain(&e);
                    error!(error = ?e, chain = %chain, "stream chunk error");
                    stream_error = Some(format!("Upstream stream interrupted: {}", chain));
                    break;
                }
            }
        }
        // Flush any remaining data in the buffer
        let remainder = buffer.trim().to_string();
        if !remainder.is_empty() && stream_error.is_none() {
            let payload = remainder.strip_prefix("data: ").unwrap_or(&remainder);
            let _ = tx.send(Ok(Event::default().data(payload)));
        }

        // Signal done or error (not normal end — lets the TS side detect incomplete streams)
        if let Some(error_msg) = stream_error {
            let error_json = serde_json::json!({"error": error_msg}).to_string();
            let _ = tx.send(Ok(Event::default().data(error_json)));
        } else {
            let _ = tx.send(Ok(Event::default().data("[DONE]")));
        }
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
    let decision = state
        .tool_executor
        .validate_tool_call(tool_name, args)
        .await;
    tracing::info!("🔐 Daemon: policy for '{}' = {:?}", tool_name, decision);
    match decision {
        Ok(PolicyDecision::Allow) => {
            // Build the trusted root execution context for this session. Uses
            // the coordinator's ensure_root so the root scope is registered and
            // the task tool can reserve children under it.
            let root_context = state
                .root_context(session_id)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            let tool_context = crate::agent::ToolContext {
                agent: &root_context,
                invocation_id: crate::agent::ToolInvocationId::new(
                    uuid::Uuid::new_v4().to_string(),
                ),
                origin_turn_id: body.turn_id.as_deref(),
            };
            // Execute directly with hooks
            let msg = state
                .tool_executor
                .execute_with_hooks(&tool_context, "api", tool_name, args.clone())
                .await;
            let content = msg.content.unwrap_or_default();
            let parsed: serde_json::Value = serde_json::from_str(&content).unwrap_or_default();

            Ok(Json(ExecuteToolResponse {
                success: parsed["success"].as_bool().unwrap_or(false),
                output_type: parsed["output_type"].as_str().map(|s| s.to_string()),
                content: parsed["content"].as_str().map(|s| s.to_string()),
                error: parsed["error"]
                    .get("message")
                    .and_then(|m| m.as_str())
                    .map(|s| s.to_string()),
                metadata: parsed.get("metadata").cloned(),
                permission_required: None,
            }))
        }
        Ok(PolicyDecision::Ask(req)) => {
            // Check if rule was already approved for this session
            if state.is_rule_approved(session_id, &req.session_rule).await {
                let mutating = matches!(
                    tool_name.as_str(),
                    "apply_patch" | "file_edit" | "file_write" | "exec_command"
                );
                if mutating {
                    let _ = state
                        .checkpoint_manager
                        .create(&format!("before {}", tool_name))
                        .await;
                }
                let root_context = state
                    .root_context(session_id)
                    .await
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
                let tool_context = crate::agent::ToolContext {
                    agent: &root_context,
                    invocation_id: crate::agent::ToolInvocationId::new(
                        uuid::Uuid::new_v4().to_string(),
                    ),
                    origin_turn_id: body.turn_id.as_deref(),
                };
                let msg = state
                    .tool_executor
                    .execute_with_hooks(&tool_context, "api", tool_name, args.clone())
                    .await;
                let content = msg.content.unwrap_or_default();
                let parsed: serde_json::Value = serde_json::from_str(&content).unwrap_or_default();

                return Ok(Json(ExecuteToolResponse {
                    success: parsed["success"].as_bool().unwrap_or(false),
                    output_type: parsed["output_type"].as_str().map(|s| s.to_string()),
                    content: parsed["content"].as_str().map(|s| s.to_string()),
                    error: parsed["error"]
                        .get("message")
                        .and_then(|m| m.as_str())
                        .map(|s| s.to_string()),
                    metadata: parsed.get("metadata").cloned(),
                    permission_required: None,
                }));
            }

            // Need permission from user
            tracing::info!(
                "🔐 Daemon: permission required for '{}': {} (rule: {})",
                tool_name,
                req.reason,
                req.session_rule
            );
            Ok(Json(ExecuteToolResponse {
                success: false,
                output_type: None,
                content: None,
                error: None,
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
            content: None,
            error: Some(e.message),
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
    state.approve_rule("default", body.session_rule).await;

    Json(serde_json::json!({"success": true}))
}

pub async fn unapprove_tool(
    State(state): State<Arc<DaemonState>>,
    Json(body): Json<ApproveToolRequest>,
) -> Json<serde_json::Value> {
    state.tool_executor.unapprove_rule(&body.session_rule).await;
    state.unapprove_rule("default", &body.session_rule).await;

    Json(serde_json::json!({"success": true}))
}

// ── Tasks ────────────────────────────────────────────────────────────────────

pub async fn list_tasks(State(state): State<Arc<DaemonState>>) -> Json<ListTasksResponse> {
    let all = state.task_manager.get_all_tasks().await;
    debug_log(&format!(
        "[list_tasks handler] returning {} tasks",
        all.len()
    ));
    let tasks: Vec<TaskInfo> = all
        .into_iter()
        .map(|t| TaskInfo {
            id: t.id,
            subject: t.subject,
            description: t.description,
            status: match t.status {
                TaskStatus::Pending => "pending",
                TaskStatus::InProgress => "in_progress",
                TaskStatus::Completed => "completed",
                TaskStatus::Deleted => "deleted",
            }
            .to_string(),
            priority: match t.priority {
                TaskPriority::Low => "low",
                TaskPriority::Medium => "medium",
                TaskPriority::High => "high",
                TaskPriority::Critical => "critical",
            }
            .to_string(),
            created_at: t.created_at.to_rfc3339(),
            updated_at: t.updated_at.to_rfc3339(),
            tags: t.tags,
        })
        .collect();

    Json(ListTasksResponse { tasks })
}

/// `GET /api/v1/tasks/progress` - ready/blocked counts for agent-loop nudges.
pub async fn task_progress(
    State(state): State<Arc<DaemonState>>,
) -> Json<crate::daemon::models::TaskProgressResponse> {
    let store = state.task_manager.task_store();
    let map = store.read().await;
    let all: std::collections::HashMap<String, crate::tasks::Task> = map.clone();
    drop(map);
    let blocked = crate::tasks::blocked_tasks(&all).len();
    let ready = crate::tasks::ready_tasks(&all).len();
    Json(crate::daemon::models::TaskProgressResponse { blocked, ready })
}

// ── Todos (s03 TodoWrite) ────────────────────────────────────────────────────

pub async fn get_todos(State(state): State<Arc<DaemonState>>) -> Json<GetTodosResponse> {
    let todo_state = state.todo_state.read().await;
    let items: Vec<TodoItemResponse> = todo_state
        .items
        .iter()
        .map(|t| TodoItemResponse {
            content: t.content.clone(),
            status: t.status.clone(),
            active_form: t.active_form.clone(),
            subagent: t.subagent.clone(),
        })
        .collect();
    let has_open = todo_state.has_open_items();
    let display = todo_state.render();
    Json(GetTodosResponse {
        items,
        has_open_items: has_open,
        display,
    })
}

// ── Background Tasks ──────────────────────────────────────────────────────────

pub async fn get_background_results(
    State(state): State<Arc<DaemonState>>,
) -> Json<serde_json::Value> {
    let results = state.background_manager.drain_results().await;
    Json(serde_json::json!({ "results": results }))
}

// ── Subagent Progress ────────────────────────────────────────────────────────

// ── MCP ──────────────────────────────────────────────────────────────────────

pub async fn list_mcp_servers(
    State(state): State<Arc<DaemonState>>,
) -> Json<ListMcpServersResponse> {
    let servers: Vec<McpServerInfo> = state
        .mcp_manager
        .list_servers_for_settings(&state.app_state.settings)
        .await
        .into_iter()
        .map(|server| McpServerInfo {
            name: server.name,
            status: server.status.to_string(),
            tools_count: server.tools_count,
            resources_count: server.resources_count,
        })
        .collect();

    Json(ListMcpServersResponse { servers })
}

// ── Sessions ──────────────────────────────────────────────────────────────────

pub async fn list_sessions(
    State(state): State<Arc<DaemonState>>,
) -> Result<Json<Vec<SessionInfoResponse>>, StatusCode> {
    let sessions = state
        .session_manager
        .list()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(
        sessions
            .into_iter()
            .map(|s| SessionInfoResponse {
                id: s.id,
                name: s.name,
                project_path: s.project_path.map(|p| p.to_string_lossy().to_string()),
                created_at: s.created_at.to_rfc3339(),
                updated_at: s.updated_at.to_rfc3339(),
                message_count: s.message_count,
                status: format!("{:?}", s.status),
            })
            .collect(),
    ))
}

pub async fn create_session(
    State(state): State<Arc<DaemonState>>,
    Json(body): Json<CreateSessionRequest>,
) -> Result<Json<SessionResponse>, StatusCode> {
    let session = state
        .session_manager
        .create(body.name.as_deref())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(SessionResponse {
        id: session.id,
        name: session.name,
        created_at: session.created_at.to_rfc3339(),
        updated_at: session.updated_at.to_rfc3339(),
        messages: session.messages,
    }))
}

pub async fn get_session(
    State(state): State<Arc<DaemonState>>,
    Path(id): Path<String>,
) -> Result<Json<SessionResponse>, StatusCode> {
    let session = state
        .session_manager
        .load(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(SessionResponse {
        id: session.id,
        name: session.name,
        created_at: session.created_at.to_rfc3339(),
        updated_at: session.updated_at.to_rfc3339(),
        messages: session.messages,
    }))
}

pub async fn update_session(
    State(state): State<Arc<DaemonState>>,
    Path(id): Path<String>,
    Json(body): Json<UpdateSessionRequest>,
) -> Result<Json<SessionResponse>, StatusCode> {
    let mut session = state
        .session_manager
        .load(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .unwrap_or_else(|| crate::context::memory_session::Session::new(Some(&id)));

    if let Some(name) = &body.name {
        session.name = name.clone();
    }
    if let Some(messages) = body.messages {
        session.messages = messages;
    }
    session.updated_at = chrono::Utc::now();

    state
        .session_manager
        .save(&session)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(SessionResponse {
        id: session.id,
        name: session.name,
        created_at: session.created_at.to_rfc3339(),
        updated_at: session.updated_at.to_rfc3339(),
        messages: session.messages,
    }))
}

pub async fn delete_session(
    State(state): State<Arc<DaemonState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    match state.session_manager.delete(&id).await {
        Ok(()) => Ok(Json(serde_json::json!({"success": true}))),
        Err(e) => {
            if e.to_string().contains("Invalid session ID") {
                Err(StatusCode::BAD_REQUEST)
            } else {
                Err(StatusCode::INTERNAL_SERVER_ERROR)
            }
        }
    }
}

pub async fn search_sessions(
    State(state): State<Arc<DaemonState>>,
    Query(query): Query<SearchSessionsQuery>,
) -> Result<Json<Vec<SessionInfoResponse>>, StatusCode> {
    let sessions = state.session_manager.search(&query.q).await;

    Ok(Json(
        sessions
            .into_iter()
            .map(|s| SessionInfoResponse {
                id: s.id,
                name: s.name,
                project_path: s.project_path.map(|p| p.to_string_lossy().to_string()),
                created_at: s.created_at.to_rfc3339(),
                updated_at: s.updated_at.to_rfc3339(),
                message_count: s.message_count,
                status: format!("{:?}", s.status),
            })
            .collect(),
    ))
}

// ── Undo ───────────────────────────────────────────────────────────────────

pub async fn undo_checkpoint(State(state): State<Arc<DaemonState>>) -> Result<String, StatusCode> {
    match state.checkpoint_manager.undo().await {
        Ok(output) => Ok(output),
        Err(_e) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

// ── Scoped agent APIs (strict subagent isolation) ────────────────────────────
//
// These handlers replace the flat `/api/v1/subagent/progress` endpoint with
// capability-scoped local views. Every denial (missing/unknown viewer token,
// expired/wrong-viewer/wrong-session/wrong-target capability, hidden target)
// maps to one stable 404 with no target details, so denials are
// indistinguishable.

use axum::http::HeaderMap;

/// Header name carrying the trusted-UI viewer bearer token.
const VIEWER_TOKEN_HEADER: &str = "x-wgenty-viewer-token";

/// Extracts and resolves the viewer token from headers. Returns None on any
/// failure; callers respond with a stable 404.
async fn resolve_viewer_from_headers(
    state: &DaemonState,
    headers: &HeaderMap,
) -> Option<crate::agent::capability::ViewerId> {
    let token = headers.get(VIEWER_TOKEN_HEADER)?.to_str().ok()?;
    state.resolve_viewer(token).await
}

fn map_scoped_coordinator_error(error: crate::agent::CoordinatorError) -> StatusCode {
    match error {
        crate::agent::CoordinatorError::NotVisible => StatusCode::NOT_FOUND,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// `POST /api/v1/ui/viewers` -- create a trusted UI viewer. Generates a
/// 256-bit bearer token, stores only its HMAC digest, returns the token once.
pub async fn create_viewer(
    State(state): State<Arc<DaemonState>>,
) -> Result<Json<CreateViewerResponse>, StatusCode> {
    let token = state.create_viewer().await;
    Ok(Json(CreateViewerResponse {
        viewer_token: token,
    }))
}

/// Builds a `LocalAgentViewResponse` for `caller`, issuing fresh navigate
/// capabilities for each direct child bound to `viewer`.
async fn build_local_view(
    state: &DaemonState,
    caller: &crate::agent::AgentExecutionContext,
    viewer: &crate::agent::capability::ViewerId,
) -> Result<LocalAgentViewResponse, StatusCode> {
    // Cross-populate from the legacy progress store so the TUI focus view has
    // conversation data for self and each direct child. Once the
    // coordinator owns the canonical progress store this lookup becomes a
    // coordinator projection; for now it bridges the migration.
    let session_progress = {
        // Clone only this session's progress so the read guard is released
        // before issuing child capabilities across await points below.
        let progress_store = state.subagent_progress.read().await;
        progress_store
            .get(caller.session_id.as_str())
            .cloned()
            .unwrap_or_default()
    };

    assemble_local_view(
        &state.coordinator,
        &state.capability_service,
        &session_progress,
        caller,
        viewer,
    )
    .await
}

/// Assembles a trusted UI local response and issues generation-bound
/// capabilities for its canonical direct children.
async fn assemble_local_view(
    coordinator: &crate::agent::AgentCoordinator,
    capability_service: &crate::agent::capability::CapabilityService,
    session_progress: &HashMap<String, crate::agent::progress::SubagentProgress>,
    caller: &crate::agent::AgentExecutionContext,
    viewer: &crate::agent::capability::ViewerId,
) -> Result<LocalAgentViewResponse, StatusCode> {
    let (self_record, child_records) = coordinator
        .trusted_ui_local_records(&caller.session_id, &caller.agent_id)
        .await
        .map_err(map_scoped_coordinator_error)?;
    let self_node = session_progress.get(self_record.agent_id.as_str());
    let mut children = Vec::with_capacity(child_records.len());
    for child in child_records {
        let grant = crate::agent::capability::CapabilityGrant::navigate(
            viewer.as_str(),
            caller.session_id.as_str(),
            child.agent_id.as_str(),
            child.generation,
        );
        let cap = capability_service.issue(&grant).await;
        // Cross-fill snapshot data from the legacy progress store.
        let node = session_progress.get(child.agent_id.as_str());
        let text_snapshot = node.and_then(|p| p.text_snapshot.clone());
        let cumulative_tokens = node.map(|p| p.cumulative_tokens).unwrap_or(0);
        let messages = node.map(|p| p.messages.clone()).unwrap_or_default();
        children.push(DirectChildResponse {
            agent_id: child.agent_id.as_str().to_string(),
            status: child.status,
            label: child.label.clone(),
            summary: child.summary.as_ref().map(|summary| summary.text.clone()),
            navigation_capability: cap,
            text_snapshot,
            cumulative_tokens,
            messages,
        });
    }
    Ok(LocalAgentViewResponse {
        self_view: SelfAgentResponse {
            agent_id: self_record.agent_id.as_str().to_string(),
            status: self_record.status,
            label: self_record.label,
            text_snapshot: self_node.and_then(|progress| progress.text_snapshot.clone()),
            cumulative_tokens: self_node
                .map(|progress| progress.cumulative_tokens)
                .unwrap_or(0),
            messages: self_node
                .map(|progress| progress.messages.clone())
                .unwrap_or_default(),
        },
        children,
    })
}

/// Resolves an opaque navigation capability and reconstructs a read-only UI
/// context from the canonical hierarchy record.
async fn resolve_navigation_context(
    coordinator: &crate::agent::AgentCoordinator,
    capability_service: &crate::agent::capability::CapabilityService,
    capability: &str,
    viewer: &crate::agent::capability::ViewerId,
    session_id: &str,
) -> Result<crate::agent::AgentExecutionContext, StatusCode> {
    let resolved = capability_service
        .resolve_navigation(capability, viewer.as_str(), session_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let session = crate::agent::SessionId::new(session_id);
    let record = coordinator
        .trusted_ui_record(&session, &resolved.target)
        .await
        .map_err(map_scoped_coordinator_error)?;
    if record.generation != resolved.generation {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(crate::agent::AgentExecutionContext {
        session_id: record.session_id,
        agent_id: record.agent_id,
        parent_id: record.parent_id,
        depth: record.depth,
        // This context is used only for trusted, read-only UI projection. It
        // must not borrow cancellation authority from an unrelated ancestor.
        cancellation: tokio_util::sync::CancellationToken::new(),
    })
}

/// `GET /api/v1/agents/self?session_id=<id>` -- root local view (self + direct
/// children).
pub async fn get_agent_self(
    State(state): State<Arc<DaemonState>>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Result<Json<LocalAgentViewResponse>, StatusCode> {
    let viewer = resolve_viewer_from_headers(&state, &headers)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;
    let session_id = params
        .get("session_id")
        .map(|s| s.as_str())
        .unwrap_or("default");
    let root = state
        .root_context(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let view = build_local_view(&state, &root, &viewer).await?;
    Ok(Json(view))
}

/// `GET /api/v1/agents/children?session_id=<id>` -- alias for the root local
/// view, kept for the route shape in the plan.
pub async fn get_agent_children(
    State(state): State<Arc<DaemonState>>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Result<Json<LocalAgentViewResponse>, StatusCode> {
    get_agent_self(State(state), Query(params), headers).await
}

/// `GET /api/v1/agents/children/:capability?session_id=<id>` -- navigate into
/// the direct child bound by `capability`. Returns that child's local view
/// (self + its direct children), with fresh navigate capabilities.
pub async fn navigate_agent_view(
    State(state): State<Arc<DaemonState>>,
    Path(capability): Path<String>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Result<Json<LocalAgentViewResponse>, StatusCode> {
    let viewer = resolve_viewer_from_headers(&state, &headers)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;
    let session_id = params
        .get("session_id")
        .map(|s| s.as_str())
        .unwrap_or("default");
    let target_context = resolve_navigation_context(
        &state.coordinator,
        &state.capability_service,
        &capability,
        &viewer,
        session_id,
    )
    .await?;
    let target_view = build_local_view(&state, &target_context, &viewer).await?;
    Ok(Json(target_view))
}

/// `GET /api/v1/agents/children/:capability/transcript?session_id=<id>` --
/// read the transcript of the direct child bound by `capability`.
pub async fn get_child_transcript(
    State(state): State<Arc<DaemonState>>,
    Path(capability): Path<String>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let viewer = resolve_viewer_from_headers(&state, &headers)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;
    let session_id = params
        .get("session_id")
        .map(|s| s.as_str())
        .unwrap_or("default");
    let root = state
        .root_context(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let view = state
        .coordinator
        .list_local(&root)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    for child in &view.children {
        let req = crate::agent::capability::CapabilityRequest::transcript(
            viewer.as_str(),
            session_id,
            child.agent_id.as_str(),
            0,
        );
        if state
            .capability_service
            .verify(&capability, &req)
            .await
            .is_ok()
        {
            let transcript = state
                .coordinator
                .read_transcript(&root, child.agent_id.clone())
                .await
                .map_err(|_| StatusCode::NOT_FOUND)?;
            return Ok(Json(serde_json::json!({ "transcript": transcript })));
        }
    }
    // Indistinguishable denial.
    Err(StatusCode::NOT_FOUND)
}

/// `POST /api/v1/agents/children/:capability/cancel?session_id=<id>` --
/// cancel the direct child bound by `capability`.
pub async fn cancel_child(
    State(state): State<Arc<DaemonState>>,
    Path(capability): Path<String>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Result<StatusCode, StatusCode> {
    let viewer = resolve_viewer_from_headers(&state, &headers)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;
    let session_id = params
        .get("session_id")
        .map(|s| s.as_str())
        .unwrap_or("default");
    let root = state
        .root_context(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let view = state
        .coordinator
        .list_local(&root)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    for child in &view.children {
        let req = crate::agent::capability::CapabilityRequest::cancel(
            viewer.as_str(),
            session_id,
            child.agent_id.as_str(),
            0,
        );
        if state
            .capability_service
            .verify(&capability, &req)
            .await
            .is_ok()
        {
            let result = state
                .coordinator
                .cancel_subtree(&root, child.agent_id.clone())
                .await;
            return match result {
                Ok(()) => Ok(StatusCode::NO_CONTENT),
                Err(_) => Err(StatusCode::NOT_FOUND),
            };
        }
    }
    Err(StatusCode::NOT_FOUND)
}

/// `POST /api/v1/agents/task-groups/claim` -- atomically claim one ready
/// root-direct task group for the persistent main agent. Returns `200` with a
/// delivery when a ready group exists, or `204 No Content` when nothing is
/// ready. Atomicity is coordinator-owned: concurrent claims deliver a group at
/// most once.
pub async fn claim_task_group(
    State(state): State<Arc<DaemonState>>,
    Json(body): Json<ClaimTaskGroupRequest>,
) -> Result<Json<TaskGroupDeliveryResponse>, StatusCode> {
    let root = state
        .root_context(&body.session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let delivery = state
        .coordinator
        .claim_ready_root_group(&root, body.generation)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    match delivery {
        Some(d) => Ok(Json(TaskGroupDeliveryResponse {
            group_id: d.group_id.as_str().to_string(),
            generation: d.generation,
            results: d.results,
        })),
        None => Err(StatusCode::NO_CONTENT),
    }
}

/// `POST /api/v1/agents/generation/reset` -- advance the session generation
/// and cancel obsolete root-direct subtrees. The old generation's ready groups
/// are no longer deliverable; in-flight root children are cancelled
/// bottom-up. Returns the new generation.
pub async fn reset_agent_generation(
    State(state): State<Arc<DaemonState>>,
    Json(body): Json<ResetAgentGenerationRequest>,
) -> Result<Json<ResetAgentGenerationResponse>, StatusCode> {
    let root = state
        .root_context(&body.session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    // Cancel obsolete root-direct children before advancing so their permits
    // are released and their terminals persisted as Cancelled.
    let _ = state.coordinator.cancel_root_children(&root).await;
    let old_generation = state.coordinator.current_generation(&root.session_id).await;
    let _ = state
        .coordinator
        .cancel_generation(&root.session_id, old_generation)
        .await;
    let new_generation = state.coordinator.advance_generation(&root.session_id).await;
    Ok(Json(ResetAgentGenerationResponse {
        generation: new_generation,
    }))
}

/// `POST /api/v1/agents/session/cancel` -- cancel the entire agent session:
/// resolve the trusted root, cancel its live subtrees bottom-up, await handles
/// with the shutdown timeout, persist `Cancelled` descendants, and release
/// every permit. Used on application shutdown so no subagent outlives the
/// session.
pub async fn cancel_agent_session(
    State(state): State<Arc<DaemonState>>,
    Json(body): Json<ResetAgentGenerationRequest>,
) -> Result<StatusCode, StatusCode> {
    let root = state
        .root_context(&body.session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let _ = state.coordinator.cancel_root_children(&root).await;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn agent_routes_support_recursive_generation_bound_navigation() {
        use crate::agent::capability::CapabilityGrant;
        use crate::agent::SpawnChildRequest;
        use crate::config::Settings;
        use crate::daemon::models::{CreateViewerResponse, LocalAgentViewResponse};
        use crate::state::AppState;

        let temp = tempfile::tempdir().unwrap();
        let mut settings = Settings::default();
        settings.storage.working_dir = temp.path().to_path_buf();
        settings.storage.transcript.db_path = temp
            .path()
            .join("subagent-transcripts.db")
            .to_string_lossy()
            .into_owned();
        let state = Arc::new(DaemonState::new(AppState::new(settings)).await);
        let root = state.root_context("session").await.unwrap();
        assert_eq!(
            state.coordinator.advance_generation(&root.session_id).await,
            1
        );
        let child = state
            .coordinator
            .reserve_child(&root, SpawnChildRequest::new("child"))
            .await
            .unwrap()
            .context;
        let grandchild = state
            .coordinator
            .reserve_child(&child, SpawnChildRequest::new("grandchild"))
            .await
            .unwrap()
            .context;

        let app = crate::daemon::routes::agent_routes().with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let base_url = format!("http://{address}");
        let client = reqwest::Client::new();

        let viewer = client
            .post(format!("{base_url}/api/v1/ui/viewers"))
            .send()
            .await
            .unwrap()
            .json::<CreateViewerResponse>()
            .await
            .unwrap()
            .viewer_token;
        let root_response = client
            .get(format!("{base_url}/api/v1/agents/self?session_id=session"))
            .header(VIEWER_TOKEN_HEADER, &viewer)
            .send()
            .await
            .unwrap();
        assert_eq!(root_response.status(), StatusCode::OK);
        let root_view = root_response
            .json::<LocalAgentViewResponse>()
            .await
            .unwrap();
        assert_eq!(root_view.self_view.agent_id, root.agent_id.as_str());
        assert_eq!(root_view.children.len(), 1);
        assert_eq!(root_view.children[0].agent_id, child.agent_id.as_str());

        let child_capability = &root_view.children[0].navigation_capability;
        let child_response = client
            .get(format!(
                "{base_url}/api/v1/agents/children/{child_capability}?session_id=session"
            ))
            .header(VIEWER_TOKEN_HEADER, &viewer)
            .send()
            .await
            .unwrap();
        assert_eq!(child_response.status(), StatusCode::OK);
        let child_view = child_response
            .json::<LocalAgentViewResponse>()
            .await
            .unwrap();
        assert_eq!(child_view.self_view.agent_id, child.agent_id.as_str());
        assert_eq!(child_view.children.len(), 1);
        assert_eq!(
            child_view.children[0].agent_id,
            grandchild.agent_id.as_str()
        );

        let grandchild_capability = &child_view.children[0].navigation_capability;
        let grandchild_response = client
            .get(format!(
                "{base_url}/api/v1/agents/children/{grandchild_capability}?session_id=session"
            ))
            .header(VIEWER_TOKEN_HEADER, &viewer)
            .send()
            .await
            .unwrap();
        assert_eq!(grandchild_response.status(), StatusCode::OK);
        let grandchild_view = grandchild_response
            .json::<LocalAgentViewResponse>()
            .await
            .unwrap();
        assert_eq!(
            grandchild_view.self_view.agent_id,
            grandchild.agent_id.as_str()
        );
        assert!(grandchild_view.children.is_empty());

        let wrong_viewer = client
            .post(format!("{base_url}/api/v1/ui/viewers"))
            .send()
            .await
            .unwrap()
            .json::<CreateViewerResponse>()
            .await
            .unwrap()
            .viewer_token;
        for (capability, denied_viewer, denied_session) in [
            (
                grandchild_capability.as_str(),
                wrong_viewer.as_str(),
                "session",
            ),
            (grandchild_capability.as_str(), viewer.as_str(), "other"),
            ("forged-capability", viewer.as_str(), "session"),
        ] {
            let response = client
                .get(format!(
                    "{base_url}/api/v1/agents/children/{capability}?session_id={denied_session}"
                ))
                .header(VIEWER_TOKEN_HEADER, denied_viewer)
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        }

        let stale_capability = state
            .capability_service
            .issue(&CapabilityGrant::navigate(
                viewer.as_str(),
                "session",
                grandchild.agent_id.as_str(),
                0,
            ))
            .await;
        let stale_response = client
            .get(format!(
                "{base_url}/api/v1/agents/children/{stale_capability}?session_id=session"
            ))
            .header(VIEWER_TOKEN_HEADER, &viewer)
            .send()
            .await
            .unwrap();
        assert_eq!(stale_response.status(), StatusCode::NOT_FOUND);

        server.abort();
    }

    #[tokio::test]
    async fn navigation_resolves_recursive_targets_with_canonical_hierarchy() {
        use crate::agent::capability::{CapabilityGrant, CapabilityService, ViewerId};
        use crate::agent::{AgentCoordinator, SessionId, SpawnChildRequest};
        use std::collections::HashMap;

        let coordinator = AgentCoordinator::new(8, 4);
        let root = coordinator
            .ensure_root(SessionId::new("session"))
            .await
            .unwrap();
        assert_eq!(coordinator.advance_generation(&root.session_id).await, 1);
        let child = coordinator
            .reserve_child(&root, SpawnChildRequest::new("child"))
            .await
            .unwrap()
            .context;
        let grandchild = coordinator
            .reserve_child(&child, SpawnChildRequest::new("grandchild"))
            .await
            .unwrap()
            .context;
        let service = CapabilityService::new([7; 32]);
        let viewer = ViewerId::new("viewer");
        let progress = HashMap::new();

        let root_response = assemble_local_view(&coordinator, &service, &progress, &root, &viewer)
            .await
            .unwrap();
        assert_eq!(root_response.self_view.agent_id, root.agent_id.as_str());
        assert_eq!(root_response.children.len(), 1);
        assert!(!root_response
            .children
            .iter()
            .any(|record| record.agent_id == grandchild.agent_id.as_str()));

        let child_capability = &root_response.children[0].navigation_capability;
        let child_context = resolve_navigation_context(
            &coordinator,
            &service,
            child_capability,
            &viewer,
            "session",
        )
        .await
        .unwrap();
        assert_eq!(child_context.agent_id, child.agent_id);
        assert_eq!(child_context.parent_id, Some(root.agent_id.clone()));
        assert_eq!(child_context.depth, 1);
        let child_response =
            assemble_local_view(&coordinator, &service, &progress, &child_context, &viewer)
                .await
                .unwrap();
        assert_eq!(child_response.self_view.agent_id, child.agent_id.as_str());
        assert_eq!(child_response.children.len(), 1);
        assert_eq!(
            child_response.children[0].agent_id,
            grandchild.agent_id.as_str()
        );

        let grandchild_capability = &child_response.children[0].navigation_capability;
        let grandchild_context = resolve_navigation_context(
            &coordinator,
            &service,
            grandchild_capability,
            &viewer,
            "session",
        )
        .await
        .unwrap();
        assert_eq!(grandchild_context.agent_id, grandchild.agent_id);
        assert_eq!(grandchild_context.parent_id, Some(child.agent_id.clone()));
        assert_eq!(grandchild_context.depth, 2);
        let grandchild_response = assemble_local_view(
            &coordinator,
            &service,
            &progress,
            &grandchild_context,
            &viewer,
        )
        .await
        .unwrap();
        assert_eq!(
            grandchild_response.self_view.agent_id,
            grandchild.agent_id.as_str()
        );
        assert!(grandchild_response.children.is_empty());

        let stale_capability = service
            .issue(&CapabilityGrant::navigate(
                viewer.as_str(),
                "session",
                grandchild.agent_id.as_str(),
                0,
            ))
            .await;
        assert_eq!(
            resolve_navigation_context(
                &coordinator,
                &service,
                &stale_capability,
                &viewer,
                "session",
            )
            .await
            .unwrap_err(),
            StatusCode::NOT_FOUND
        );

        for (capability, denied_viewer, denied_session) in [
            (
                grandchild_capability.as_str(),
                ViewerId::new("wrong-viewer"),
                "session",
            ),
            (
                grandchild_capability.as_str(),
                viewer.clone(),
                "wrong-session",
            ),
            ("forged-capability", viewer.clone(), "session"),
        ] {
            assert_eq!(
                resolve_navigation_context(
                    &coordinator,
                    &service,
                    capability,
                    &denied_viewer,
                    denied_session,
                )
                .await
                .unwrap_err(),
                StatusCode::NOT_FOUND
            );
        }
    }

    #[test]
    fn scoped_coordinator_error_preserves_not_found_boundary() {
        assert_eq!(
            map_scoped_coordinator_error(crate::agent::CoordinatorError::NotVisible),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            map_scoped_coordinator_error(crate::agent::CoordinatorError::Storage(
                "invariant".to_string()
            )),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }
}
