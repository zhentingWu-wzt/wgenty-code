//! HTTP request handlers for the daemon API.

use crate::api::{ApiClient, ToolDefinition};
use crate::daemon::models::*;
use crate::daemon::state::DaemonState;
use crate::permissions::{PolicyDecision, ToolPermissionPolicy};
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
    // Read from the live handle so a `/model` switch is reflected here too.
    let s = state
        .settings_handle
        .read()
        .expect("lock poisoned: settings");
    Json(ConfigResponse {
        model: s.models.main.name.clone(),
        api_base: s.models.main.endpoint_base_url(),
        max_tokens: s.models.transport.max_tokens,
        timeout: s.models.transport.timeout,
        streaming: s.models.transport.streaming,
    })
}

/// GET /api/v1/models - list switchable model profiles for the `/model` picker.
/// Always includes the currently active one (marked `active: true`). If
/// `models.profiles` is empty, returns an empty list (picker can show a hint).
pub async fn list_models(State(state): State<Arc<DaemonState>>) -> Json<ListModelsResponse> {
    let s = state
        .settings_handle
        .read()
        .expect("lock poisoned: settings");
    let active = s.models.active_profile.as_deref();
    let mut profiles: Vec<ModelProfileInfo> = s
        .models
        .profiles
        .iter()
        .map(|(key, ep)| ModelProfileInfo {
            key: key.clone(),
            label: ep.display_name.clone().unwrap_or_else(|| ep.name.clone()),
            model_name: ep.name.clone(),
            provider: ep.provider.clone(),
            tier: ep.tier.map(|t| {
                match t {
                    crate::config::models::ModelTier::Light => "light",
                    crate::config::models::ModelTier::Medium => "medium",
                    crate::config::models::ModelTier::Heavy => "heavy",
                }
                .to_string()
            }),
            active: active == Some(key.as_str()),
        })
        .collect();
    // Stable, alphabetical ordering for a predictable picker.
    profiles.sort_by(|a, b| a.key.cmp(&b.key));
    Json(ListModelsResponse { profiles })
}

/// POST /api/v1/model/switch - activate a named profile. Copies the profile
/// endpoint into `models.main`, records `active_profile`, persists to disk,
/// and updates the live handle so the next chat turn uses the new model.
///
/// Returns 400 with an actionable message (listing available profiles) when
/// the profile key is unknown.
pub async fn switch_model(
    State(state): State<Arc<DaemonState>>,
    Json(body): Json<SwitchModelRequest>,
) -> Result<Json<SwitchModelResponse>, (StatusCode, String)> {
    // 1. Read current settings, switch in a clone, validate the profile exists.
    let mut settings = state
        .settings_handle
        .read()
        .expect("lock poisoned: settings")
        .clone();
    settings
        .switch_to_profile(&body.profile)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("{e:#}")))?;

    // 2. Persist so the choice survives a restart. Use the disk-load form to
    //    avoid writing back a runtime-resolved absolute working_dir.
    let mut disk = crate::config::Settings::load_from_disk().unwrap_or_else(|_| settings.clone());
    disk.switch_to_profile(&body.profile)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?;
    if let Err(e) = disk.save() {
        tracing::warn!(error = %e, "failed to persist settings.json after model switch");
    }

    let label = settings.main_model_label();
    let model_name = settings.models.main.name.clone();
    let provider = settings.models.main.provider.clone();

    // 3. Publish to the live handle — next chat_stream reads this.
    *state
        .settings_handle
        .write()
        .expect("lock poisoned: settings") = settings;

    tracing::info!(
        profile = %body.profile,
        model = %model_name,
        provider = ?provider,
        "model switched via /model"
    );

    state.broadcast_global(
        crate::daemon::global_events::GlobalEventKind::ModelChanged,
        serde_json::json!({
            "profile": body.profile,
            "model_name": model_name,
            "provider": provider,
        }),
    );

    Ok(Json(SwitchModelResponse {
        success: true,
        profile: body.profile,
        label,
        model_name,
        provider,
    }))
}

// ── Chat / Stream ────────────────────────────────────────────────────────────

pub async fn chat_stream(
    State(state): State<Arc<DaemonState>>,
    Json(body): Json<ChatStreamRequest>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    // Clone from the live handle so a `/model` switch takes effect on the
    // very next turn. The shared pooled HTTP clients below keep their
    // keep-alive pool + TLS session cache across requests.
    let settings = state
        .settings_handle
        .read()
        .expect("lock poisoned: settings")
        .clone();
    let client = ApiClient::with_clients(
        settings,
        state.http_client.clone(),
        state.http_client_stream.clone(),
    );

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

// ── Subagent Trace Stream (SSE) ──────────────────────────────────────────────

/// Query parameters for `GET /api/v1/subagents/trace/stream`.
#[derive(Debug, serde::Deserialize)]
pub struct TraceStreamQuery {
    /// Filter to a single session. When omitted, the stream is global (all
    /// sessions, live-only -- no cold-start replay).
    #[serde(default)]
    pub session_id: Option<String>,
    /// Unix epoch milliseconds. Replayed headers and live events at or before
    /// this timestamp are skipped.
    #[serde(default)]
    pub since: Option<i64>,
}

/// `GET /api/v1/subagents/trace/stream` -- SSE stream of live subagent trace
/// events with optional cold-start replay.
///
/// On connect (when `session_id` is given) the endpoint replays persisted
/// transcript headers for that session from the global transcript store, then
/// streams live redacted events from the process-global trace hub. A slow
/// subscriber observes `Lagged` (drop-oldest); file persistence is unaffected.
/// Requires the standard bearer token (`require_auth`). See design D3 / Q5.
pub async fn subagent_trace_stream(
    State(state): State<Arc<DaemonState>>,
    Query(q): Query<TraceStreamQuery>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let (tx, rx) = mpsc::unbounded_channel::<Result<Event, Infallible>>();

    // Subscribe to the global live hub BEFORE cold-start replay so events
    // emitted during replay are buffered in the receiver (avoids a race that
    // would drop events between replay and subscribe).
    let live = crate::teams::trace_sink::trace_hub_subscribe();

    let session_id = q.session_id;
    let since = q.since.unwrap_or(0);
    let store = state.transcript_store.clone();

    tokio::spawn(async move {
        let mut live = live;

        // 1. Cold-start replay from the global transcript store. Only when a
        //    session is requested: a global (no session_id) subscription has
        //    no single persisted history to replay and starts live.
        if let Some(sid) = session_id.as_deref() {
            if let Some(store) = store.as_ref() {
                for ev in replay_session_events(store, sid, since) {
                    let data = serde_json::to_string(&ev).unwrap_or_default();
                    if tx.send(Ok(Event::default().data(data))).is_err() {
                        return; // client disconnected
                    }
                }
            }
        }

        // 2. Live stream from the global hub.
        loop {
            match live.recv().await {
                Ok(ev) => {
                    if !should_emit_live(&ev, session_id.as_deref(), since) {
                        continue;
                    }
                    let data = serde_json::to_string(&ev).unwrap_or_default();
                    if tx.send(Ok(Event::default().data(data))).is_err() {
                        return; // client disconnected
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(
                        target: "wgenty::daemon",
                        lagged = n,
                        "trace SSE subscriber lagged; oldest events dropped for this subscriber"
                    );
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    Sse::new(UnboundedReceiverStream::new(rx)).keep_alive(KeepAlive::default())
}

/// Reconstruct persisted transcript headers for `session_id` into trace events
/// for cold-start SSE replay. Headers newer than `since` (by `started_at`, unix
/// ms) are emitted in chronological (ascending) order. Per-step detail
/// (round/tool/params) is not stored at the header level, so each replayed
/// event carries the run's terminal state; live events provide per-step detail
/// going forward. `pub(crate)` for unit testing.
pub(crate) fn replay_session_events(
    store: &crate::transcript::SubagentTranscriptStore,
    session_id: &str,
    since: i64,
) -> Vec<crate::teams::trace_sink::TraceEvent> {
    match store.list_by_session(session_id) {
        Ok(headers) => {
            // list_by_session returns DESC by started_at; emit ASC so the
            // client observes chronological order.
            let mut ordered: Vec<_> = headers
                .into_iter()
                .filter(|h| h.started_at > since)
                .collect();
            ordered.reverse();
            ordered
                .into_iter()
                .map(|h| trace_event_from_header(&h))
                .collect()
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                session_id = session_id,
                "cold-start replay: list_by_session failed"
            );
            Vec::new()
        }
    }
}

/// Whether a live trace event should be emitted to a subscriber filtered by an
/// optional `session_id` and a `since` (unix ms) watermark. `since` is
/// inclusive-skip: events with `ts <= since` are dropped. `pub(crate)` for unit
/// testing.
pub(crate) fn should_emit_live(
    ev: &crate::teams::trace_sink::TraceEvent,
    session_id: Option<&str>,
    since: i64,
) -> bool {
    if let Some(sid) = session_id {
        if ev.session_id != sid {
            return false;
        }
    }
    ev.ts > since
}

/// Reconstruct a `TraceEvent` summary from a persisted transcript header.
///
/// The `status` string is the raw DB value (lowercase, e.g. "completed"); live
/// events use the runtime `SubagentStatus` serde name (PascalCase, e.g.
/// "Completed"). Consumers should treat both case-insensitively. The `error`
/// object carries the persisted message + denormalized `root_cause`.
fn trace_event_from_header(
    h: &crate::transcript::SubagentTranscriptHeader,
) -> crate::teams::trace_sink::TraceEvent {
    use crate::teams::trace_sink::TraceEvent;
    let error = h.error_message.as_ref().map(|m| {
        let mut obj = serde_json::Map::new();
        obj.insert("message".to_string(), serde_json::Value::String(m.clone()));
        obj.insert(
            "root_cause".to_string(),
            serde_json::to_value(&h.root_cause).unwrap_or(serde_json::Value::Null),
        );
        serde_json::Value::Object(obj)
    });
    TraceEvent {
        ts: h.started_at,
        session_id: h.session_id.clone(),
        node_id: h.id.clone(),
        parent_id: h.parent_id.clone(),
        label: h.label.clone(),
        status: h.status.clone(),
        round: Some(h.actual_rounds as usize),
        current_tool: None,
        current_params: None,
        elapsed_ms: 0,
        progress_delta: None,
        token_budget_k: None,
        cumulative_tokens: h.total_tokens,
        error,
        kind: crate::teams::trace_sink::TraceEventKind::Progress,
        permission: None,
        question: None,
    }
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

    // Multi-project: validate against the session's own effective root (bound
    // worktree > project > main working_dir) so the policy boundary always
    // matches the execution workdir, and snapshot into that project's
    // checkpoint store.
    let session_root = state.effective_session_root(session_id).await;
    let (cp_manager, cp_store) = state.checkpoints_for_project(&session_root);

    // Per-project permission mode for this session's working directory.
    let mode_entry = state.permission_modes.get(&session_root);

    // Validate against policy
    let decision = {
        let policy = ToolPermissionPolicy::new(session_root.clone());
        let rules_handle = state.tool_executor.session_rules_handle();
        let rules = rules_handle.read().await;
        crate::tools::executor::validate_tool_call_shared(
            &state.tool_registry,
            &policy,
            &rules,
            tool_name,
            args,
        )
    };
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
            let effective_mode = mode_entry.effective_mode;
            // Ensure the turn snapshot exists when the client supplies a turn id
            // (TUI/REPL generate one per user message). Plan mode skips capture
            // inside maybe_capture_pre_edit via EffectiveMode::Plan.
            if let Some(turn_id) = body.turn_id.as_deref() {
                if let Err(e) = cp_manager.begin_turn(turn_id) {
                    tracing::warn!(error = %e, turn = %turn_id, "checkpoint begin_turn failed");
                }
            }
            let tool_context = crate::agent::ToolContext {
                agent: &root_context,
                invocation_id: crate::agent::ToolInvocationId::new(
                    uuid::Uuid::new_v4().to_string(),
                ),
                origin_turn_id: body.turn_id.as_deref(),
                workdir: Some(session_root.as_path()),
                effective_mode,
                checkpoint: Some(cp_store.as_ref()),
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
            // Check if rule was already approved for this session, OR root mode
            // auto-approves this tool (AcceptEdits / Yolo). Without the mode
            // bypass, AcceptEdits still bounced every write through the TUI.
            let mode_auto = mode_entry.root_mode.auto_approves(tool_name);
            let already = state.is_rule_approved(session_id, &req.session_rule).await;
            if already || mode_auto {
                if mode_auto && !already {
                    tracing::info!(
                        "🔐 Daemon: root_mode auto-approved '{}' (rule: {})",
                        tool_name,
                        req.session_rule
                    );
                }
                // Per-tool git-stash checkpoints removed: pre-edit capture happens
                // inside ToolRegistry::execute_with_context via CheckpointStore.
                if let Some(turn_id) = body.turn_id.as_deref() {
                    if let Err(e) = cp_manager.begin_turn(turn_id) {
                        tracing::warn!(error = %e, turn = %turn_id, "checkpoint begin_turn failed");
                    }
                }
                let root_context = state
                    .root_context(session_id)
                    .await
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
                let effective_mode = mode_entry.effective_mode;
                let tool_context = crate::agent::ToolContext {
                    agent: &root_context,
                    invocation_id: crate::agent::ToolInvocationId::new(
                        uuid::Uuid::new_v4().to_string(),
                    ),
                    origin_turn_id: body.turn_id.as_deref(),
                    workdir: Some(session_root.as_path()),
                    effective_mode,
                    checkpoint: Some(cp_store.as_ref()),
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
                    tool_name: tool_name.clone(),
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

/// GET /api/v1/tools/pending-permissions — subagent policy Ask waiters.
pub async fn list_pending_permissions(
    State(state): State<Arc<DaemonState>>,
) -> Json<crate::daemon::models::ListPendingPermissionsResponse> {
    let pending = state
        .permission_bridge
        .pending()
        .await
        .into_iter()
        .map(|a| crate::daemon::models::PendingSubagentPermission {
            request_id: a.request_id,
            from: a.from,
            kind: a.kind,
            tool: a.tool,
            policy_reason: a.policy_reason,
            session_rule: a.session_rule,
            human_summary: a.human_summary,
        })
        .collect();
    Json(crate::daemon::models::ListPendingPermissionsResponse { pending })
}

/// POST /api/v1/tools/resolve-permission — unblock a subagent Ask waiter.
///
/// Duplicate answers get 409 with the standing (first) decision instead of an
/// indistinguishable `{success:false}`; unknown ids get 404.
pub async fn resolve_subagent_permission(
    State(state): State<Arc<DaemonState>>,
    Json(body): Json<crate::daemon::models::ResolveSubagentPermissionRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    use crate::teams::PermissionResolveOutcome as Outcome;
    match state
        .permission_bridge
        .resolve(&body.request_id, body.approved)
        .await
    {
        Outcome::Resolved => {
            // Approve the standing rule only on the FIRST resolution — a
            // duplicate answer must produce no second effect (spec §审批).
            if body.approved && body.always {
                if let Some(rule) = body.session_rule.clone() {
                    state.tool_executor.approve_rule(rule.clone()).await;
                    // deprecated(compat): legacy global rule scope, see Task 12.
                    state.approve_rule("default", rule).await;
                }
            }
            Ok(Json(
                serde_json::json!({ "success": true, "resolved": true }),
            ))
        }
        Outcome::AlreadyResolved(approved) => Err((
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "success": false, "resolved": true, "approved": approved,
            })),
        )),
        Outcome::Unknown => Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "success": false, "resolved": false })),
        )),
    }
}

/// `POST /api/v1/interactions/:id/resolve` — answer a pending ask_user_question
/// prompt from the server-side loop. The answer string unblocks the waiting
/// InteractionBridge waiter. Duplicate answers get 409 with the standing
/// (first) resolution instead of an indistinguishable 404.
pub async fn resolve_interaction(
    State(state): State<Arc<DaemonState>>,
    Path(request_id): Path<String>,
    Json(body): Json<crate::daemon::models::ResolveInteractionRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    match state
        .interaction_bridge
        .resolve(&request_id, body.answer)
        .await
    {
        crate::daemon::interaction_bridge::ResolveOutcome::Resolved => {
            Ok(Json(serde_json::json!({ "resolved": true })))
        }
        crate::daemon::interaction_bridge::ResolveOutcome::AlreadyResolved(answer) => {
            // Duplicate answer: conflict, and hand back the standing resolution.
            Err((
                StatusCode::CONFLICT,
                Json(serde_json::json!({ "resolved": false, "answer": answer })),
            ))
        }
        // Never existed (or waiter gone). 404 so the client stops retrying.
        crate::daemon::interaction_bridge::ResolveOutcome::Unknown => Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "unknown interaction" })),
        )),
    }
}

/// POST /api/v1/permission-mode - update the root agent's runtime permission
/// mode (Yolo/AcceptEdits/Normal) and optional sandbox effective mode (Plan).
/// Subagents snapshot values at spawn time.
pub async fn set_permission_mode(
    State(state): State<Arc<DaemonState>>,
    Json(body): Json<crate::daemon::models::SetPermissionModeRequest>,
) -> Json<serde_json::Value> {
    let session_id = body.session_id.as_deref().unwrap_or("default");
    let session_root = state.effective_session_root(session_id).await;
    let effective = body
        .effective_mode
        .unwrap_or_else(|| crate::sandbox::EffectiveMode::from_root_permission_mode(body.mode));
    state
        .permission_modes
        .set(session_root, body.mode, effective);
    tracing::info!(
        mode = ?body.mode,
        effective_mode = ?effective,
        "root permission / effective mode updated"
    );
    state.broadcast_global(
        crate::daemon::global_events::GlobalEventKind::ModeChanged,
        serde_json::json!({
            "session_id": session_id,
            "mode": body.mode,
            "effective_mode": effective,
        }),
    );
    Json(serde_json::json!({
        "success": true,
        "mode": body.mode,
        "effective_mode": effective,
    }))
}

/// GET /api/v1/permission-mode - get the current root agent permission mode.
pub async fn get_permission_mode(
    State(state): State<Arc<DaemonState>>,
    Query(params): Query<crate::daemon::models::PermissionModeQuery>,
) -> Json<serde_json::Value> {
    let session_id = params.session_id.as_deref().unwrap_or("default");
    let session_root = state.effective_session_root(session_id).await;
    let entry = state.permission_modes.get(&session_root);
    Json(serde_json::json!({
        "mode": entry.root_mode,
        "effective_mode": entry.effective_mode,
    }))
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
    // Snapshot read (no drain): results are retained so every client can
    // query them; the old first-come-first-served drain is abolished.
    let results = state.background_results_snapshot().await;
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

/// Map a stored [`SessionInfo`] to its API response shape (shared by the list
/// and search handlers).
fn session_info_response(s: crate::context::memory_session::SessionInfo) -> SessionInfoResponse {
    SessionInfoResponse {
        id: s.id,
        name: s.name,
        project_path: s.project_path.map(|p| p.to_string_lossy().to_string()),
        created_at: s.created_at.to_rfc3339(),
        updated_at: s.updated_at.to_rfc3339(),
        message_count: s.message_count,
        status: format!("{:?}", s.status),
        worktree: s.worktree.map(|w| WorktreeRef {
            path: w.path,
            branch: w.branch,
        }),
    }
}

/// Sessions stored under a non-main project's directory but lacking
/// `project_path` (e.g. moved there manually) still group under that project.
fn fill_project(
    root: &std::path::Path,
    sessions: &mut [crate::context::memory_session::SessionInfo],
) {
    for s in sessions.iter_mut() {
        if s.project_path.is_none() {
            s.project_path = Some(root.to_path_buf());
        }
    }
}

pub async fn list_sessions(
    State(state): State<Arc<DaemonState>>,
) -> Result<Json<Vec<SessionInfoResponse>>, StatusCode> {
    // Aggregate the main project and every registered project.
    let mut sessions = state
        .session_manager
        .list()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    for root in state.projects.registered_roots() {
        let mgr = state.session_manager_for_project(&root).await;
        let mut project_sessions = mgr
            .list()
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        fill_project(&root, &mut project_sessions);
        sessions.extend(project_sessions);
    }

    Ok(Json(
        sessions.into_iter().map(session_info_response).collect(),
    ))
}

pub async fn create_session(
    State(state): State<Arc<DaemonState>>,
    Json(body): Json<CreateSessionRequest>,
) -> Result<Json<SessionResponse>, (StatusCode, String)> {
    // Route to the owning project's session store (default: main project).
    let root = match &body.project_path {
        Some(p) => state.projects.resolve(p).ok_or((
            StatusCode::BAD_REQUEST,
            format!("not a registered project: {p}"),
        ))?,
        None => state.projects.main_root(),
    };
    let mgr = state.session_manager_for_project(&root).await;
    let session =
        crate::context::memory_session::Session::new(body.name.as_deref()).with_project(root);
    mgr.save(&session)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(SessionResponse {
        worktree: crate::context::memory_session::worktree_of(&session).map(|w| WorktreeRef {
            path: w.path,
            branch: w.branch,
        }),
        id: session.id,
        name: session.name,
        created_at: session.created_at.to_rfc3339(),
        updated_at: session.updated_at.to_rfc3339(),
        messages: session.messages,
        ui_messages: session.ui_messages,
    }))
}

pub async fn get_session(
    State(state): State<Arc<DaemonState>>,
    Path(id): Path<String>,
) -> Result<Json<SessionResponse>, StatusCode> {
    let (_mgr, session) = state
        .resolve_session(&id)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(SessionResponse {
        worktree: crate::context::memory_session::worktree_of(&session).map(|w| WorktreeRef {
            path: w.path,
            branch: w.branch,
        }),
        id: session.id,
        name: session.name,
        created_at: session.created_at.to_rfc3339(),
        updated_at: session.updated_at.to_rfc3339(),
        messages: session.messages,
        ui_messages: session.ui_messages,
    }))
}

pub async fn update_session(
    State(state): State<Arc<DaemonState>>,
    Path(id): Path<String>,
    Json(body): Json<UpdateSessionRequest>,
) -> Result<Json<SessionResponse>, (StatusCode, Json<serde_json::Value>)> {
    // Run lock: mutating a session mid-run would race the run's final save.
    if state.session_runs.is_active(&id) {
        return Err((
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error": "run active"})),
        ));
    }

    // Route to the owning project's store; an unknown id upserts into the
    // main project (legacy TUI behavior).
    let (mgr, mut session) = match state.resolve_session(&id).await {
        Some((mgr, session)) => (mgr, session),
        // Upsert must preserve the path id. Session::new() mints a fresh UUID and
        // previously caused every SaveSession to write a new file (duplicate names
        // in the session panel) while the TUI continued using the original id.
        None => (
            state.session_manager.clone(),
            crate::context::memory_session::Session::with_id(id.clone(), None),
        ),
    };

    // Defense in depth: even if a future constructor changes, never let the
    // on-disk / response id diverge from the request path.
    session.id = id;

    if let Some(name) = &body.name {
        session.name = name.clone();
    }
    if let Some(messages) = body.messages {
        session.messages = messages;
    }
    if let Some(ui_messages) = body.ui_messages {
        session.ui_messages = ui_messages;
    }
    session.updated_at = chrono::Utc::now();
    // Fully materialised write — clear any lazy index marker.
    session.lazy_message_count = None;

    mgr.save(&session).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
    })?;

    Ok(Json(SessionResponse {
        worktree: crate::context::memory_session::worktree_of(&session).map(|w| WorktreeRef {
            path: w.path,
            branch: w.branch,
        }),
        id: session.id,
        name: session.name,
        created_at: session.created_at.to_rfc3339(),
        updated_at: session.updated_at.to_rfc3339(),
        messages: session.messages,
        ui_messages: session.ui_messages,
    }))
}

pub async fn delete_session(
    State(state): State<Arc<DaemonState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let mgr = match state.resolve_session(&id).await {
        Some((mgr, _)) => mgr,
        // Unknown id: keep the legacy error semantics of the main store.
        None => state.session_manager.clone(),
    };
    match mgr.delete(&id).await {
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
    let mut sessions = state.session_manager.search(&query.q).await;
    for root in state.projects.registered_roots() {
        let mgr = state.session_manager_for_project(&root).await;
        let mut project_sessions = mgr.search(&query.q).await;
        fill_project(&root, &mut project_sessions);
        sessions.extend(project_sessions);
    }

    Ok(Json(
        sessions.into_iter().map(session_info_response).collect(),
    ))
}

// ── Undo ───────────────────────────────────────────────────────────────────

pub async fn undo_checkpoint(State(state): State<Arc<DaemonState>>) -> Result<String, StatusCode> {
    match state.checkpoint_manager.undo(None).await {
        Ok(output) => Ok(output),
        Err(e) => {
            tracing::warn!(error = %e, "undo_checkpoint failed");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// Request body for `POST /api/v1/tools/undo-turn`.
///
/// `turn_ids` is oldest-first; the store rewinds them newest-first so the
/// oldest turn's pre-edit content wins for files edited across multiple turns.
#[derive(serde::Deserialize)]
pub struct UndoTurnRangeRequest {
    pub turn_ids: Vec<String>,
    /// Project whose checkpoint store holds the turns (`None` = main project).
    #[serde(default)]
    pub project: Option<String>,
}

/// POST /api/v1/tools/undo-turn - rewind a range of turns, restoring files to
/// the state before the oldest turn in the range. Backs the TUI `/undo`
/// code-rollback flow (roll back all turns after the kept turn N).
pub async fn undo_turn_range(
    State(state): State<Arc<DaemonState>>,
    Json(body): Json<UndoTurnRangeRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let manager = match &body.project {
        Some(p) => {
            let root = state.projects.resolve(p).ok_or((
                StatusCode::BAD_REQUEST,
                format!("not a registered project: {p}"),
            ))?;
            state.checkpoints_for_project(&root).0
        }
        None => state.checkpoint_manager.clone(),
    };
    match manager.undo_range(body.turn_ids).await {
        Ok(report) => Ok(Json(serde_json::json!({
            "restored": report.restored,
            "skipped": report.skipped,
            "failed": report.failed,
            "rewound_turns": report.rewound_turns,
        }))),
        Err(e) => {
            tracing::warn!(error = %e, "undo_turn_range failed");
            Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
        }
    }
}

/// Query for `GET /api/v1/checkpoints`.
#[derive(serde::Deserialize)]
pub struct ListCheckpointsQuery {
    /// Project whose checkpoints to list (`None` = main project).
    #[serde(default)]
    pub project: Option<String>,
}

/// GET /api/v1/checkpoints - list per-turn checkpoint snapshots (newest-first).
/// Each entry carries the turn id, creation timestamp, and file count, used by
/// the TUI `/undo` turn-picker to show how many files each turn touched.
pub async fn list_checkpoints(
    State(state): State<Arc<DaemonState>>,
    Query(q): Query<ListCheckpointsQuery>,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, String)> {
    let manager = match &q.project {
        Some(p) => {
            let root = state.projects.resolve(p).ok_or((
                StatusCode::BAD_REQUEST,
                format!("not a registered project: {p}"),
            ))?;
            state.checkpoints_for_project(&root).0
        }
        None => state.checkpoint_manager.clone(),
    };
    match manager.store().list() {
        Ok(infos) => Ok(Json(
            infos
                .into_iter()
                .map(|i| {
                    serde_json::json!({
                        "turn_id": i.turn_id,
                        "created_at": i.created_at,
                        "file_count": i.file_count,
                    })
                })
                .collect(),
        )),
        Err(e) => {
            tracing::warn!(error = %e, "list_checkpoints failed");
            Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
        }
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

/// Computes a live `elapsed_ms` for a subagent progress entry.
///
/// The subagent loop only emits progress at round/tool boundaries, so a
/// long-running tool (e.g. `sleep`) freezes the stored `elapsed_ms` until the
/// next emit. For still-running agents recompute from `started_at` so the TUI
/// timer keeps ticking; terminal agents keep their stored final value.
fn live_elapsed_ms(progress: Option<&crate::agent::progress::SubagentProgress>) -> u64 {
    let Some(p) = progress else {
        return 0;
    };
    let stored = p.elapsed_ms;
    if p.status.is_terminal() || p.started_at <= 0 {
        return stored;
    }
    let now_ms = chrono::Utc::now().timestamp_millis();
    ((now_ms - p.started_at).max(0) as u64).max(stored)
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
        let started_at = node.map(|p| p.started_at).unwrap_or(0);
        let elapsed_ms = live_elapsed_ms(node);
        let round = node.and_then(|p| p.round);
        let max_rounds = node.and_then(|p| p.max_rounds);
        let messages = node.map(|p| p.messages.clone()).unwrap_or_default();
        children.push(DirectChildResponse {
            agent_id: child.agent_id.as_str().to_string(),
            status: child.status,
            label: child.label.clone(),
            summary: child.summary.as_ref().map(|summary| summary.text.clone()),
            navigation_capability: cap,
            text_snapshot,
            cumulative_tokens,
            started_at,
            elapsed_ms,
            round,
            max_rounds,
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
            started_at: self_node.map(|progress| progress.started_at).unwrap_or(0),
            elapsed_ms: live_elapsed_ms(self_node),
            round: self_node.and_then(|progress| progress.round),
            max_rounds: self_node.and_then(|progress| progress.max_rounds),
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
        Some(d) => {
            // Broadcast the claimed group's result summary on the global bus;
            // `project` lets multi-project clients filter (design §10).
            let project = state.effective_session_root(&body.session_id).await;
            state.broadcast_global(
                crate::daemon::global_events::GlobalEventKind::TaskGroupResult,
                serde_json::json!({
                    "task_group_id": d.group_id.as_str(),
                    "session_id": body.session_id,
                    "generation": d.generation,
                    "project": project,
                    "result_count": d.results.len(),
                    "statuses": d.results.iter().map(|r| r.status).collect::<Vec<_>>(),
                }),
            );
            Ok(Json(TaskGroupDeliveryResponse {
                group_id: d.group_id.as_str().to_string(),
                generation: d.generation,
                results: d.results,
            }))
        }
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

// ── Memory ops API (Tier 2 web-ops-console) ──────────────────────────────────
// Thin wrappers over the shared MemoryManager. All read-only except prune.

/// Resolve the memory pool for an optional `project` query param (`None` =
/// main project). Shared by all memory endpoints.
async fn memory_manager_for(
    state: &DaemonState,
    project: Option<&str>,
) -> Result<Arc<crate::context::MemoryManager>, (StatusCode, String)> {
    match project {
        Some(p) => {
            let root = state.projects.resolve(p).ok_or((
                StatusCode::BAD_REQUEST,
                format!("not a registered project: {p}"),
            ))?;
            Ok(state.memory_router.for_project(&root).await)
        }
        None => Ok(state.memory_manager.clone()),
    }
}

/// `GET /api/v1/memory/status` — dual-pool status summary.
pub async fn memory_status(
    State(state): State<Arc<DaemonState>>,
    Query(q): Query<MemoryProjectQuery>,
) -> Result<Json<crate::context::MemoryStatus>, (StatusCode, String)> {
    let mgr = memory_manager_for(&state, q.project.as_deref()).await?;
    let status = mgr.status().await.map_err(|e| {
        error!(error = ?e, "memory status failed");
        (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    })?;
    Ok(Json(status))
}

/// `GET /api/v1/memory` — list with optional scope/min_importance/limit filters.
pub async fn list_memory(
    State(state): State<Arc<DaemonState>>,
    Query(q): Query<MemoryListQuery>,
) -> Result<Json<MemoryListResponse>, (StatusCode, String)> {
    let mgr = memory_manager_for(&state, q.project.as_deref()).await?;
    let limit = q.limit.unwrap_or(500).clamp(1, 5000);
    let entries = mgr.list_memories(q.min_importance, limit).await;

    let items: Vec<MemoryItemResponse> = entries
        .into_iter()
        .filter(|(origin, _)| match q.scope.as_deref() {
            Some("project") => matches!(origin, crate::context::MemoryOrigin::Project),
            Some("global") => matches!(origin, crate::context::MemoryOrigin::Global),
            _ => true, // "all" or unspecified
        })
        .map(|(origin, entry)| MemoryItemResponse {
            origin: origin_str(origin).to_string(),
            entry,
        })
        .collect();
    let total = items.len();
    Ok(Json(MemoryListResponse { items, total }))
}

/// `GET /api/v1/memory/:id` — single memory with origin.
pub async fn get_memory(
    State(state): State<Arc<DaemonState>>,
    Path(id): Path<String>,
    Query(q): Query<MemoryProjectQuery>,
) -> Result<Json<MemoryDetailResponse>, (StatusCode, String)> {
    // get_memory doesn't return origin; re-derive it by checking both pools.
    let mgr = memory_manager_for(&state, q.project.as_deref()).await?;
    if let Some(entry) = mgr.get_memory(&id).await {
        // list_memories gives us the origin mapping cheaply for the lookup.
        let origin = mgr
            .list_memories(None, 5000)
            .await
            .into_iter()
            .find(|(_, e)| e.id == id)
            .map(|(o, _)| origin_str(o).to_string())
            .unwrap_or_else(|| "project".to_string());
        Ok(Json(MemoryDetailResponse { origin, entry }))
    } else {
        Err((StatusCode::NOT_FOUND, format!("no such memory: {id}")))
    }
}

/// `POST /api/v1/memory/prune` — invoke prune; dry_run is advisory (the
/// underlying prune() always executes, so a true dry-run requires a manager
/// change — for now we honor the flag by returning status without pruning).
pub async fn prune_memory(
    State(state): State<Arc<DaemonState>>,
    Query(q): Query<MemoryProjectQuery>,
    req: Option<Json<PruneRequest>>,
) -> Result<Json<crate::context::PruneResult>, (StatusCode, String)> {
    let mgr = memory_manager_for(&state, q.project.as_deref()).await?;
    let dry_run = req.map(|b| b.dry_run).unwrap_or(false);
    // Dry-run: return the would-be result by reading status deltas. The
    // current MemoryManager::prune is destructive with no preview, so we
    // approximate dry-run as a no-op returning current counts as before==after.
    if dry_run {
        let s = mgr.status().await.map_err(|e| {
            error!(error = ?e, "memory status (dry-run prune) failed");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?;
        return Ok(Json(crate::context::PruneResult {
            before: s.total_memories,
            after: s.total_memories,
            removed: 0,
            project_before: s.project_count,
            project_after: s.project_count,
            global_before: s.global_count,
            global_after: s.global_count,
        }));
    }
    let result = mgr.prune().await.map_err(|e| {
        error!(error = ?e, "memory prune failed");
        (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    })?;
    Ok(Json(result))
}

/// Project MemoryOrigin to its serialized string form.
fn origin_str(o: crate::context::MemoryOrigin) -> &'static str {
    match o {
        crate::context::MemoryOrigin::Project => "project",
        crate::context::MemoryOrigin::Global => "global",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn daemon_state_saves_sessions_under_project_local_dir() {
        use crate::config::Settings;
        use crate::daemon::models::{SessionResponse, UpdateSessionRequest};
        use crate::state::AppState;
        use crate::utils::project_sessions_dir;
        use axum::extract::{Path, State};
        use axum::Json;

        let temp = tempfile::tempdir().unwrap();
        let mut settings = Settings::default();
        settings.storage.working_dir = temp.path().to_path_buf();
        // Do not override session_manager — this asserts DaemonState::new wires
        // MemorySessionManager::with_project_root(working_dir).
        let state = Arc::new(DaemonState::new(AppState::new(settings)).await);

        let fixed_id = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".to_string();
        let body = UpdateSessionRequest {
            name: Some("project-local".to_string()),
            messages: Some(vec![crate::context::memory_session::SessionMessage {
                role: "user".to_string(),
                content: "hello".to_string(),
                tool_call_id: None,
                tool_calls: None,
                timestamp: chrono::Utc::now(),
                metadata: Default::default(),
            }]),
            ui_messages: None,
        };
        let Json(resp): Json<SessionResponse> =
            update_session(State(state.clone()), Path(fixed_id.clone()), Json(body))
                .await
                .expect("update_session should succeed");
        assert_eq!(resp.id, fixed_id);

        let expected_path = project_sessions_dir(temp.path()).join(format!("{fixed_id}.json"));
        assert!(
            expected_path.is_file(),
            "session must be written under project-local dir, expected {}",
            expected_path.display()
        );

        // Must not land only in the global home sessions dir for this working_dir.
        let home_sessions = dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join(".wgenty-code")
            .join("sessions")
            .join(format!("{fixed_id}.json"));
        assert!(
            !home_sessions.is_file(),
            "session must not be written to global ~/.wgenty-code/sessions when project dir is writable"
        );
    }

    #[tokio::test]
    async fn multi_project_session_routing() {
        use crate::config::Settings;
        use crate::daemon::models::CreateSessionRequest;
        use crate::state::AppState;
        use axum::extract::{Path, State};
        use axum::Json;

        let main = tempfile::tempdir().unwrap();
        let mut settings = Settings::default();
        settings.storage.working_dir = main.path().to_path_buf();
        let mut state = DaemonState::new(AppState::new(settings)).await;
        // Isolate the registry from the developer's real projects.json.
        let reg_store = tempfile::tempdir().unwrap();
        state.projects = crate::daemon::projects::ProjectRegistry::load(
            main.path().to_path_buf(),
            reg_store.path().join("projects.json"),
        );
        let state = Arc::new(state);

        // Register project B and create a session in it.
        let proj_b = tempfile::tempdir().unwrap();
        let b_canon = proj_b.path().canonicalize().unwrap();
        state.projects.add(proj_b.path().to_str().unwrap()).unwrap();
        let Json(created) = create_session(
            State(state.clone()),
            Json(CreateSessionRequest {
                name: Some("in-b".to_string()),
                project_path: Some(b_canon.to_string_lossy().to_string()),
            }),
        )
        .await
        .expect("create_session into project B");
        let sid = created.id;

        // The session file lands under project B's store, not the main one.
        assert!(crate::utils::project_sessions_dir(&b_canon)
            .join(format!("{sid}.json"))
            .is_file());
        assert!(!crate::utils::project_sessions_dir(main.path())
            .join(format!("{sid}.json"))
            .is_file());

        // list_sessions aggregates both projects and tags the entry.
        let Json(listed) = list_sessions(State(state.clone())).await.unwrap();
        let entry = listed.iter().find(|s| s.id == sid).expect("listed");
        assert_eq!(
            entry.project_path.as_deref(),
            Some(b_canon.to_string_lossy().as_ref())
        );

        // get/update/delete route across projects.
        let Json(got) = get_session(State(state.clone()), Path(sid.clone()))
            .await
            .expect("get_session routes to project B");
        assert_eq!(got.name, "in-b");

        // The effective working root follows the session's project.
        assert_eq!(state.effective_session_root(&sid).await, b_canon);

        // Unknown project path is rejected.
        let rejected = create_session(
            State(state.clone()),
            Json(CreateSessionRequest {
                name: None,
                project_path: Some("/no/such/project".to_string()),
            }),
        )
        .await;
        assert!(rejected.is_err());

        // delete routes to the owning store.
        let _ = delete_session(State(state.clone()), Path(sid.clone()))
            .await
            .expect("delete routes to project B");
        assert!(state.resolve_session(&sid).await.is_none());
    }

    #[tokio::test]
    async fn update_session_upsert_preserves_path_id_across_saves() {
        use crate::config::Settings;
        use crate::daemon::models::{SessionResponse, UpdateSessionRequest};
        use crate::state::AppState;
        use axum::extract::{Path, State};
        use axum::Json;

        let temp = tempfile::tempdir().unwrap();
        let mut settings = Settings::default();
        settings.storage.working_dir = temp.path().to_path_buf();
        // Isolate session files for this test before wrapping in Arc.
        let sessions_dir = temp.path().join("sessions-test");
        let mut state = DaemonState::new(AppState::new(settings)).await;
        state.session_manager =
            crate::context::memory_session::SessionManager::with_dir(sessions_dir.clone());
        let state = Arc::new(state);

        let fixed_id = "11111111-2222-3333-4444-555555555555".to_string();
        for i in 0..3 {
            let body = UpdateSessionRequest {
                name: Some("duplicate-name".to_string()),
                messages: Some(vec![crate::context::memory_session::SessionMessage {
                    role: "user".to_string(),
                    content: format!("turn-{i}"),
                    tool_call_id: None,
                    tool_calls: None,
                    timestamp: chrono::Utc::now(),
                    metadata: Default::default(),
                }]),
                ui_messages: None,
            };
            let Json(resp): Json<SessionResponse> =
                update_session(State(state.clone()), Path(fixed_id.clone()), Json(body))
                    .await
                    .expect("update_session should succeed");
            assert_eq!(resp.id, fixed_id, "response id must match path id");
            assert_eq!(resp.name, "duplicate-name");
        }

        let mut entries = tokio::fs::read_dir(&sessions_dir).await.unwrap();
        let mut files = Vec::new();
        while let Some(entry) = entries.next_entry().await.unwrap() {
            files.push(entry.file_name().to_string_lossy().into_owned());
        }
        assert_eq!(
            files,
            vec![format!("{fixed_id}.json")],
            "must not mint a new file per save"
        );

        let loaded = state
            .session_manager
            .load(&fixed_id)
            .await
            .unwrap()
            .expect("session file exists");
        assert_eq!(loaded.id, fixed_id);
        assert_eq!(loaded.messages.len(), 1); // last write replaces messages
    }

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
        // This test exercises a root -> child -> grandchild chain (depth 3),
        // but the product default disables subagent recursion (max_depth=1).
        // Raise the limit here so the navigation scenario under test can run.
        settings.agent.subagent.max_depth = 3;
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

    // ── Subagent trace SSE (Task 3.4 / 3.5) ──────────────────────────────────

    #[tokio::test]
    async fn replay_session_events_filters_by_session_and_since() {
        use crate::transcript::{SubagentTranscript, SubagentTranscriptStore, TranscriptStatus};

        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("replay.db");
        let store = SubagentTranscriptStore::open(&db).unwrap();

        let mk =
            |id: &str, sid: &str, started_at: i64, status: TranscriptStatus| SubagentTranscript {
                id: id.into(),
                session_id: sid.into(),
                parent_id: None,
                label: format!("label-{id}"),
                status,
                system_prompt: None,
                user_prompt: "u".into(),
                started_at,
                finished_at: Some(started_at + 1000),
                total_tokens: 100,
                max_rounds: None,
                actual_rounds: 3,
                token_budget_k: None,
                error_message: None,
                summary: None,
                failure_diagnostics: None,
                project_path: None,
                events: vec![],
            };

        store
            .save(&mk("a", "alpha", 1000, TranscriptStatus::Completed), None)
            .unwrap();
        store
            .save(&mk("b", "alpha", 2000, TranscriptStatus::Failed), None)
            .unwrap();
        store
            .save(&mk("c", "beta", 3000, TranscriptStatus::Completed), None)
            .unwrap();

        // No since filter: both alpha events, ascending by started_at
        // (list_by_session returns DESC; replay reverses to ASC).
        let evs = replay_session_events(&store, "alpha", 0);
        assert_eq!(evs.len(), 2);
        assert_eq!(evs[0].node_id, "a");
        assert_eq!(evs[0].ts, 1000);
        assert_eq!(evs[1].node_id, "b");
        assert_eq!(evs[1].ts, 2000);

        // since=1000 skips the first (ts <= since is inclusive-skip).
        let evs = replay_session_events(&store, "alpha", 1000);
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].node_id, "b");

        // beta only.
        let evs = replay_session_events(&store, "beta", 0);
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].node_id, "c");

        // unknown session -> empty.
        assert!(replay_session_events(&store, "gamma", 0).is_empty());
    }

    #[test]
    fn should_emit_live_filters_session_and_since() {
        use crate::teams::trace_sink::TraceEvent;

        let mk = |ts: i64, sid: &str| TraceEvent {
            ts,
            session_id: sid.into(),
            node_id: "n".into(),
            parent_id: None,
            label: "l".into(),
            status: "Running".into(),
            round: None,
            current_tool: None,
            current_params: None,
            elapsed_ms: 0,
            progress_delta: None,
            token_budget_k: None,
            cumulative_tokens: 0,
            error: None,
            kind: crate::teams::trace_sink::TraceEventKind::Progress,
            permission: None,
            question: None,
        };

        // session filter keeps matching, drops non-matching.
        assert!(should_emit_live(&mk(100, "alpha"), Some("alpha"), 0));
        assert!(!should_emit_live(&mk(100, "beta"), Some("alpha"), 0));
        // no session filter (global) keeps all sessions.
        assert!(should_emit_live(&mk(100, "alpha"), None, 0));
        // since: ts > since passes; ts <= since dropped.
        assert!(should_emit_live(&mk(101, "alpha"), Some("alpha"), 100));
        assert!(!should_emit_live(&mk(100, "alpha"), Some("alpha"), 100));
    }

    #[tokio::test]
    async fn sse_trace_stream_requires_bearer_auth() {
        use crate::config::Settings;
        use crate::state::AppState;

        let temp = tempfile::tempdir().unwrap();
        let mut settings = Settings::default();
        settings.storage.working_dir = temp.path().to_path_buf();
        settings.storage.transcript.db_path = temp
            .path()
            .join("sse-auth.db")
            .to_string_lossy()
            .into_owned();
        let state = Arc::new(DaemonState::new(AppState::new(settings)).await);
        let token = "sse-auth-token".to_string();
        let (health, protected) = crate::daemon::routes::create_routers(state, token);
        let app = health.merge(protected);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let client = reqwest::Client::new();
        // No bearer -> 401 (middleware short-circuits before the handler).
        let resp = client
            .get(format!("http://{addr}/api/v1/subagents/trace/stream"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        // Wrong bearer -> 401.
        let resp = client
            .get(format!("http://{addr}/api/v1/subagents/trace/stream"))
            .bearer_auth("wrong")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn sse_trace_stream_cold_start_replays_persisted_session() {
        use crate::config::Settings;
        use crate::state::AppState;
        use crate::transcript::{SubagentTranscript, SubagentTranscriptStore, TranscriptStatus};
        use std::time::{Duration, Instant};

        let temp = tempfile::tempdir().unwrap();
        let mut settings = Settings::default();
        settings.storage.working_dir = temp.path().to_path_buf();
        let db_path = temp.path().join("sse-replay.db");
        settings.storage.transcript.db_path = db_path.to_string_lossy().into_owned();

        // Seed a persisted transcript for session "alpha-cs" before starting the
        // daemon so the SSE cold-start path has history to replay.
        {
            let store = SubagentTranscriptStore::open(&db_path).unwrap();
            let t = SubagentTranscript {
                id: "node-cs-1".into(),
                session_id: "alpha-cs".into(),
                parent_id: None,
                label: "cold-start-seed".into(),
                status: TranscriptStatus::Completed,
                system_prompt: None,
                user_prompt: "u".into(),
                started_at: 5_000,
                finished_at: Some(6_000),
                total_tokens: 42,
                max_rounds: None,
                actual_rounds: 2,
                token_budget_k: None,
                error_message: None,
                summary: None,
                failure_diagnostics: None,
                project_path: None,
                events: vec![],
            };
            store.save(&t, None).unwrap();
        }

        let state = Arc::new(DaemonState::new(AppState::new(settings)).await);
        let token = "sse-replay-token".to_string();
        let (health, protected) = crate::daemon::routes::create_routers(state, token);
        let app = health.merge(protected);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let client = reqwest::Client::new();
        let resp = client
            .get(format!(
                "http://{addr}/api/v1/subagents/trace/stream?session_id=alpha-cs"
            ))
            .bearer_auth("sse-replay-token")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // Read the SSE body stream; cold-start replay emits the seeded header
        // immediately. Collect with a deadline (the live loop never ends).
        let mut stream = resp.bytes_stream();
        let mut buf = String::new();
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if let Ok(Some(Ok(chunk))) =
                tokio::time::timeout(Duration::from_millis(250), stream.next()).await
            {
                buf.push_str(&String::from_utf8_lossy(&chunk));
                if buf.contains("\"node_id\":\"node-cs-1\"") {
                    break;
                }
            }
        }
        assert!(
            buf.contains("\"node_id\":\"node-cs-1\""),
            "cold-start replay event missing; got: {buf}"
        );
        assert!(
            buf.contains("\"session_id\":\"alpha-cs\""),
            "session-scoped replay missing; got: {buf}"
        );
    }

    #[tokio::test]
    async fn memory_list_filters_by_scope_and_importance() {
        use crate::config::Settings;
        use crate::context::{MemoryEntry, MemoryOrigin, MemoryType};
        use crate::daemon::models::MemoryListQuery;
        use crate::state::AppState;
        use axum::extract::{Query, State};
        use axum::Json;
        use std::sync::Arc;

        let temp = tempfile::tempdir().unwrap();
        let mut settings = Settings::default();
        settings.storage.working_dir = temp.path().to_path_buf();
        let state = Arc::new(DaemonState::new(AppState::new(settings)).await);

        // Seed: one high-importance project memory, one low project, one global.
        let mut hi = MemoryEntry::new(MemoryType::Knowledge, "important project fact");
        hi.importance = 0.9;
        let mut lo = MemoryEntry::new(MemoryType::Session, "trivial project note");
        lo.importance = 0.1;
        let mut gl = MemoryEntry::new(MemoryType::Preference, "global pref");
        gl.importance = 0.8;
        state
            .memory_manager
            .add_memory(hi, MemoryOrigin::Project)
            .await
            .unwrap();
        state
            .memory_manager
            .add_memory(lo, MemoryOrigin::Project)
            .await
            .unwrap();
        state
            .memory_manager
            .add_memory(gl, MemoryOrigin::Global)
            .await
            .unwrap();

        // No filter → all three.
        let Json(all) = list_memory(State(state.clone()), Query(MemoryListQuery::default()))
            .await
            .expect("list all");
        assert_eq!(all.total, 3, "default list should return all 3 memories");

        // Scope=project → only the two project entries.
        let Json(proj) = list_memory(
            State(state.clone()),
            Query(MemoryListQuery {
                scope: Some("project".to_string()),
                ..Default::default()
            }),
        )
        .await
        .expect("list project");
        assert_eq!(proj.total, 2, "project scope should return 2");
        assert!(proj.items.iter().all(|m| m.origin == "project"));

        // Scope=global → one global entry.
        let Json(glob) = list_memory(
            State(state.clone()),
            Query(MemoryListQuery {
                scope: Some("global".to_string()),
                ..Default::default()
            }),
        )
        .await
        .expect("list global");
        assert_eq!(glob.total, 1, "global scope should return 1");
        assert_eq!(glob.items[0].origin, "global");

        // min_importance=0.5 → only the two >= 0.5 entries (hi + global).
        let Json(imp) = list_memory(
            State(state.clone()),
            Query(MemoryListQuery {
                min_importance: Some(0.5),
                ..Default::default()
            }),
        )
        .await
        .expect("list by importance");
        assert_eq!(imp.total, 2, "importance>=0.5 should return 2");
        assert!(imp.items.iter().all(|m| m.entry.importance >= 0.5));
    }

    #[tokio::test]
    async fn memory_prune_dry_run_is_noop() {
        use crate::config::Settings;
        use crate::context::{MemoryEntry, MemoryOrigin, MemoryType};
        use crate::daemon::models::PruneRequest;
        use crate::state::AppState;
        use axum::extract::State;
        use axum::Json;
        use std::sync::Arc;

        let temp = tempfile::tempdir().unwrap();
        let mut settings = Settings::default();
        settings.storage.working_dir = temp.path().to_path_buf();
        let state = Arc::new(DaemonState::new(AppState::new(settings)).await);

        let entry = MemoryEntry::new(MemoryType::Knowledge, "kept");
        state
            .memory_manager
            .add_memory(entry, MemoryOrigin::Project)
            .await
            .unwrap();

        // Dry-run must not remove anything: before == after.
        let Json(result) = prune_memory(
            State(state.clone()),
            Query(MemoryProjectQuery { project: None }),
            Some(Json(PruneRequest { dry_run: true })),
        )
        .await
        .expect("dry-run prune");
        assert_eq!(result.removed, 0, "dry-run must remove nothing");
        assert_eq!(result.before, result.after);

        // The memory is still there.
        let Json(still) = crate::daemon::handlers::memory_status(
            State(state.clone()),
            Query(MemoryProjectQuery { project: None }),
        )
        .await
        .expect("status");
        assert!(
            still.total_memories >= 1,
            "memory must survive a dry-run prune"
        );
    }

    // ── Global event producers (daemon-session-orchestration Task 7) ─────────

    /// Tempdir-backed DaemonState for global-event tests. Construction-time
    /// I/O finishes inside `DaemonState::new`, so the tempdir may drop once
    /// the helper returns (mirrors global_events.rs::test_daemon_state).
    async fn global_event_test_state() -> DaemonState {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut settings = crate::config::Settings::default();
        settings.storage.working_dir = temp.path().to_path_buf();
        DaemonState::new(crate::state::AppState::new(settings)).await
    }

    #[tokio::test]
    async fn apply_todos_update_broadcasts_full_snapshot() {
        let state = global_event_test_state().await;
        let mut rx = state.global_event_hub.subscribe();
        state
            .apply_todos_update(vec![crate::tasks::TodoItem {
                content: "write plan".into(),
                status: "in_progress".into(),
                active_form: String::new(),
                subagent: None,
            }])
            .await;
        let ev = rx.recv().await.expect("todos event");
        assert_eq!(
            ev.kind,
            crate::daemon::global_events::GlobalEventKind::TodosChanged
        );
        assert_eq!(ev.data["items"][0]["content"], "write plan");
        // Multi-project dimension: clients filter by `project`.
        assert!(ev.data["project"].is_string(), "data: {}", ev.data);
        assert_eq!(ev.data["has_open_items"], true);
        // 快照与 GET /todos 读取同源。
        let todos = state.todo_state.read().await;
        assert_eq!(todos.items.len(), 1);
    }

    #[tokio::test]
    async fn set_permission_mode_broadcasts_mode_changed() {
        use axum::extract::State;
        use axum::Json;

        let state = Arc::new(global_event_test_state().await);
        let mut rx = state.global_event_hub.subscribe();
        let Json(resp) = set_permission_mode(
            State(state.clone()),
            Json(crate::daemon::models::SetPermissionModeRequest {
                mode: crate::config::agent::RootPermissionMode::Yolo,
                effective_mode: None,
                session_id: Some("s1".to_string()),
            }),
        )
        .await;
        assert_eq!(resp["success"], true);

        let ev = rx.recv().await.expect("mode event");
        assert_eq!(
            ev.kind,
            crate::daemon::global_events::GlobalEventKind::ModeChanged
        );
        assert_eq!(ev.data["session_id"], "s1");
        assert_eq!(ev.data["mode"], "yolo");
        // Derived from the root mode when effective_mode is omitted.
        assert_eq!(ev.data["effective_mode"], "yolo");
    }

    /// `switch_model` persists to `~/.wgenty-code/settings.json`, so the test
    /// scopes a fake `$HOME` (serial, mirroring tui token-budget tests) to
    /// never touch the developer's real config.
    #[tokio::test]
    #[serial_test::serial]
    async fn switch_model_broadcasts_model_changed() {
        use axum::extract::State;
        use axum::Json;

        let temp = tempfile::tempdir().unwrap();
        let fake_home = tempfile::tempdir().unwrap();
        let mut settings = crate::config::Settings::default();
        settings.storage.working_dir = temp.path().to_path_buf();
        settings.models.profiles.insert(
            "p2".to_string(),
            crate::config::models::ModelEndpoint {
                name: "gpt-test".to_string(),
                provider: Some("openai".to_string()),
                ..Default::default()
            },
        );
        // Pre-seed the on-disk settings (same profile) so the handler's
        // load-from-disk + switch + save path succeeds under the fake home.
        let cfg_dir = fake_home.path().join(".wgenty-code");
        std::fs::create_dir_all(&cfg_dir).unwrap();
        std::fs::write(
            cfg_dir.join("settings.json"),
            serde_json::to_string_pretty(&settings).unwrap(),
        )
        .unwrap();

        let prev_home = std::env::var_os("HOME");
        std::env::set_var("HOME", fake_home.path());
        let run = async {
            let state = Arc::new(DaemonState::new(crate::state::AppState::new(settings)).await);
            let mut rx = state.global_event_hub.subscribe();
            let resp = switch_model(
                State(state.clone()),
                Json(SwitchModelRequest {
                    profile: "p2".to_string(),
                }),
            )
            .await
            .expect("switch_model should succeed");
            assert!(resp.success);

            let ev = rx.recv().await.expect("model event");
            assert_eq!(
                ev.kind,
                crate::daemon::global_events::GlobalEventKind::ModelChanged
            );
            assert_eq!(ev.data["profile"], "p2");
            assert_eq!(ev.data["model_name"], "gpt-test");
            assert_eq!(ev.data["provider"], "openai");
        }
        .await;
        match prev_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        run
    }

    #[tokio::test]
    async fn claim_task_group_broadcasts_task_group_result() {
        use axum::extract::State;
        use axum::Json;

        let state = Arc::new(global_event_test_state().await);
        // Produce a ready root group through the coordinator (same path as
        // coordinator.rs tests): reserve a child in the group, finish it.
        let root = state.root_context("s").await.expect("root context");
        let group = state
            .coordinator
            .create_root_task_group(
                &root,
                "turn-1",
                tokio::time::Instant::now() + std::time::Duration::from_secs(30),
            )
            .await
            .expect("root group");
        let child = state
            .coordinator
            .reserve_child_in_group(
                &root,
                crate::agent::SpawnChildRequest::new("work"),
                group.clone(),
            )
            .await
            .expect("reserve child");
        state
            .coordinator
            .finish_child(
                &child.context,
                crate::agent::ChildTerminal::completed("done"),
            )
            .await
            .expect("finish child");

        let mut rx = state.global_event_hub.subscribe();
        let Json(resp) = claim_task_group(
            State(state.clone()),
            Json(ClaimTaskGroupRequest {
                session_id: "s".to_string(),
                generation: 0,
            }),
        )
        .await
        .expect("claim should deliver the ready group");
        assert_eq!(resp.results.len(), 1);

        let ev = rx.recv().await.expect("task-group event");
        assert_eq!(
            ev.kind,
            crate::daemon::global_events::GlobalEventKind::TaskGroupResult
        );
        assert_eq!(ev.data["task_group_id"], group.as_str());
        assert_eq!(ev.data["session_id"], "s");
        assert_eq!(ev.data["generation"], 0);
        assert_eq!(ev.data["result_count"], 1);
        // Multi-project dimension: clients filter by `project`.
        assert!(ev.data["project"].is_string(), "data: {}", ev.data);
    }

    /// The global events SSE endpoint lives on the protected router: no (or
    /// wrong) bearer token must short-circuit with 401 before the handler.
    #[tokio::test]
    async fn global_events_stream_requires_bearer_auth() {
        let state = Arc::new(global_event_test_state().await);
        let (health, protected) =
            crate::daemon::routes::create_routers(state, "events-auth-token".to_string());
        let app = health.merge(protected);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let client = reqwest::Client::new();
        // No bearer -> 401 (middleware short-circuits before the handler).
        let resp = client
            .get(format!("http://{addr}/api/v1/events"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        // Wrong bearer -> 401.
        let resp = client
            .get(format!("http://{addr}/api/v1/events"))
            .bearer_auth("wrong")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // ── Background results retention (daemon-session-orchestration Task 8) ────

    fn sample_bg_result(task_id: &str) -> crate::tools::execution::background::BackgroundResult {
        crate::tools::execution::background::BackgroundResult {
            task_id: task_id.to_string(),
            result_type: "command".to_string(),
            command: "true".to_string(),
            stdout: String::new(),
            stderr: String::new(),
            exit_code: Some(0),
            success: true,
            sandbox_bypassed: false,
            permission_mode: None,
            sandbox_level: None,
        }
    }

    #[tokio::test]
    async fn background_results_are_retained_not_drained() {
        let state = global_event_test_state().await;
        let mut rx = state.global_event_hub.subscribe();
        state.record_background_result(sample_bg_result("r1")).await;
        state.record_background_result(sample_bg_result("r2")).await;
        // 广播在入队之后到达。
        let ev = rx.recv().await.expect("broadcast");
        assert_eq!(
            ev.kind,
            crate::daemon::global_events::GlobalEventKind::BackgroundResult
        );
        // 两次快照读取内容一致（不再先到先得）。
        let first = state.background_results_snapshot().await;
        let second = state.background_results_snapshot().await;
        assert_eq!(first.len(), 2);
        assert_eq!(first.len(), second.len());
    }

    #[tokio::test]
    async fn background_results_evict_oldest_beyond_capacity() {
        let state = global_event_test_state().await;
        let capacity = crate::daemon::state::BACKGROUND_RESULTS_CAPACITY;
        // capacity + 1 results: the oldest ("r0") must be evicted.
        for i in 0..=capacity {
            state
                .record_background_result(sample_bg_result(&format!("r{i}")))
                .await;
        }
        let snapshot = state.background_results_snapshot().await;
        assert_eq!(snapshot.len(), capacity);
        assert_eq!(snapshot[0].task_id, "r1");
        assert_eq!(
            snapshot.last().expect("non-empty snapshot").task_id,
            format!("r{capacity}")
        );
    }

    #[tokio::test]
    async fn get_background_results_returns_snapshot_without_draining() {
        use axum::extract::State;
        use axum::Json;

        let state = Arc::new(global_event_test_state().await);
        state.record_background_result(sample_bg_result("r1")).await;
        state.record_background_result(sample_bg_result("r2")).await;
        // Two consecutive reads see the same results (no first-come-first-served
        // drain): old polling clients keep working, results are not stolen.
        let Json(first) = get_background_results(State(state.clone())).await;
        let Json(second) = get_background_results(State(state.clone())).await;
        assert_eq!(first["results"].as_array().expect("results array").len(), 2);
        assert_eq!(first, second);
    }

    /// End-to-end wiring: results completed in the tool-layer background
    /// manager flow into the daemon's retained queue (hook replaces the
    /// drain queue daemon-side) and are broadcast on the global event bus.
    #[tokio::test]
    async fn background_manager_results_flow_into_retained_queue() {
        let state = global_event_test_state().await;
        let mut rx = state.global_event_hub.subscribe();
        state
            .background_manager
            .push_subagent_result("demo", "ok", true)
            .await;
        // Retain-before-broadcast: once the event arrives the result is
        // guaranteed queryable via the snapshot.
        let ev = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("event within timeout")
            .expect("broadcast");
        assert_eq!(
            ev.kind,
            crate::daemon::global_events::GlobalEventKind::BackgroundResult
        );
        let snapshot = state.background_results_snapshot().await;
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].result_type, "subagent");
        // The tool-layer drain queue stays empty: the retained queue is the
        // single source of truth daemon-side.
        assert!(state.background_manager.drain_results().await.is_empty());
    }
}
