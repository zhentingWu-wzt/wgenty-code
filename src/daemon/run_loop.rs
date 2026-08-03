//! Daemon-side agent run loop — session event layer and root tool port.
//!
//! Task 1 scope: the `SessionEvent` envelope, a broadcast hub, and a
//! `DaemonEventSink` that maps runtime events onto the hub so HTTP/SSE
//! handlers (later tasks) can fan them out to web clients.
//!
//! Task 3 scope: [`RootToolPort`], the fully-owned [`ToolPort`] the daemon's
//! root run loop executes tools through (policy → session rules → indefinite
//! bridge escalation → guardian → registry).
//!
//! Task 4 scope: the [`RunRegistry`] (one active run per session) and the
//! `POST /sessions/:id/run` / `POST /sessions/:id/cancel` endpoints that spawn
//! and cancel server-side agent turns.
//!
//! Task 5 scope: the `GET /sessions/:id/events` SSE endpoint (live fan-out
//! from the hub, filtered by session id) and the mid-run persistence bridge
//! that turns [`RuntimeEvent::SaveSession`] into a spawned history save.

use crate::agent::runtime::{EventSink, RuntimeEvent};
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// A single event in a daemon-run session, envelope for SSE fan-out.
#[derive(Debug, Clone, Serialize)]
pub struct SessionEvent {
    /// Monotonically increasing sequence number within one run, starting at 1.
    pub seq: u64,
    pub session_id: String,
    pub run_id: String,
    pub kind: SessionEventKind,
    /// Kind-specific payload (see [`DaemonEventSink::emit`] mapping).
    pub data: serde_json::Value,
}

/// The subset of runtime events worth broadcasting to clients (v1).
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionEventKind {
    ContentDelta,
    ReasoningDelta,
    ToolStart,
    ToolResult,
    TurnDone,
    TurnError,
    Save,
}

/// Broadcast channel over which one daemon run publishes [`SessionEvent`]s.
/// Subscribers use `subscribe()`; lagging receivers see `RecvError::Lagged`.
pub type SessionEventHub = tokio::sync::broadcast::Sender<SessionEvent>;

/// [`EventSink`] that translates runtime events into [`SessionEvent`]s and
/// broadcasts them on a per-run hub. Connection noise (Connecting,
/// PreparingTools, CompactionStarted, …) is intentionally dropped in v1.
pub struct DaemonEventSink {
    session_id: String,
    run_id: String,
    hub: SessionEventHub,
    next_seq: Arc<AtomicU64>,
    /// Mid-run persistence bridge (Task 5); `None` until the run task wires
    /// the live history handle in via [`DaemonEventSink::set_save_bridge`].
    save_bridge: Option<SaveBridge>,
}

/// Handles the persistence side of [`RuntimeEvent::SaveSession`]: a snapshot
/// of the live history plus the daemon state needed to persist it. `emit` is
/// sync, so the save itself is always spawned, never run inline.
struct SaveBridge {
    state: Arc<DaemonState>,
    history: Arc<tokio::sync::Mutex<Vec<ChatMessage>>>,
}

impl DaemonEventSink {
    pub fn new(session_id: String, run_id: String, hub: SessionEventHub) -> Self {
        Self {
            session_id,
            run_id,
            hub,
            next_seq: Arc::new(AtomicU64::new(1)),
            save_bridge: None,
        }
    }

    /// Attach the mid-run persistence bridge: subsequent `SaveSession` events
    /// spawn a history snapshot save in addition to broadcasting `Save`.
    fn set_save_bridge(
        &mut self,
        state: Arc<DaemonState>,
        history: Arc<tokio::sync::Mutex<Vec<ChatMessage>>>,
    ) {
        self.save_bridge = Some(SaveBridge { state, history });
    }

    fn publish(&self, kind: SessionEventKind, data: serde_json::Value) {
        let event = SessionEvent {
            seq: self.next_seq.fetch_add(1, Ordering::Relaxed),
            session_id: self.session_id.clone(),
            run_id: self.run_id.clone(),
            kind,
            data,
        };
        // No subscribers is normal (e.g. run without an attached SSE client);
        // broadcast::Sender::send only errors in that case, so ignore it.
        let _ = self.hub.send(event);
    }
}

impl EventSink for DaemonEventSink {
    fn emit(&self, event: RuntimeEvent) {
        match event {
            RuntimeEvent::ContentDelta(text) => {
                self.publish(
                    SessionEventKind::ContentDelta,
                    serde_json::json!({ "text": text }),
                );
            }
            RuntimeEvent::ReasoningDelta(text) => {
                self.publish(
                    SessionEventKind::ReasoningDelta,
                    serde_json::json!({ "text": text }),
                );
            }
            RuntimeEvent::ToolStart { name, args } => {
                self.publish(
                    SessionEventKind::ToolStart,
                    serde_json::json!({ "name": name, "args": args }),
                );
            }
            RuntimeEvent::ToolResult {
                name,
                args,
                content,
            } => {
                self.publish(
                    SessionEventKind::ToolResult,
                    serde_json::json!({ "name": name, "args": args, "content": content }),
                );
            }
            RuntimeEvent::StreamDone { finish_reason } => {
                self.publish(
                    SessionEventKind::TurnDone,
                    serde_json::json!({ "finish_reason": finish_reason }),
                );
            }
            RuntimeEvent::StreamError(message) => {
                self.publish(
                    SessionEventKind::TurnError,
                    serde_json::json!({ "message": message }),
                );
            }
            RuntimeEvent::SaveSession => {
                self.publish(SessionEventKind::Save, serde_json::json!({}));
                // Mid-run persistence bridge (Task 5): snapshot the live
                // history and persist it in a spawned task — `emit` is sync
                // and must never block the run loop on disk I/O.
                if let Some(bridge) = &self.save_bridge {
                    let state = Arc::clone(&bridge.state);
                    let history = Arc::clone(&bridge.history);
                    let session_id = self.session_id.clone();
                    tokio::spawn(async move {
                        let snapshot = history.lock().await.clone();
                        save_session_history(&state.session_manager, &session_id, &snapshot).await;
                    });
                }
            }
            // v1: connection noise and UI-only signals are not broadcast.
            _ => {}
        }
    }
}

// ── RootToolPort (Task 3) ────────────────────────────────────────────────────

use crate::agent::runtime::ports::{ToolPort, ToolRequest, ToolResponse};
use crate::agent::{AgentExecutionContext, SessionId, ToolContext, ToolInvocationId};
use crate::api::ToolDefinition;
use crate::daemon::state::DaemonState;
use crate::permissions::policy::{PolicyDecision, ToolPermissionPolicy};
use crate::teams::permission_bridge::{PermissionBridge, StructuredApproval};
use crate::tools::executor::validate_tool_call_shared;
use crate::tools::ToolRegistry;
use async_trait::async_trait;
use std::collections::HashSet;
use std::path::PathBuf;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Outcome for a policy `Ask` before touching the permission bridge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AskDecision {
    /// Approved session rule or auto-approving root mode: run without asking.
    Execute,
    /// Escalate to the shared permission bridge and wait indefinitely.
    Escalate,
}

/// Pure Ask-branch decision (mirrors the `/tools/execute` handler ordering).
fn decide_ask(rule_approved: bool, mode_auto: bool) -> AskDecision {
    if rule_approved || mode_auto {
        AskDecision::Execute
    } else {
        AskDecision::Escalate
    }
}

/// Fully-owned [`ToolPort`] for the daemon's root run loop.
///
/// Pipeline (modeled on `GuardingToolPort` + the headless `RegistryToolPort`):
/// policy validation → shared session rules → root-mode auto-approve →
/// indefinite [`PermissionBridge`] escalation → guardian pre-check →
/// registry execution. Unlike `GuardingToolPort` everything here is owned
/// (`Arc<ToolRegistry>`, owned root [`AgentExecutionContext`]) so the port
/// outlives the stack frame that built it.
///
/// The shared `session_rules` handle comes from `DaemonState::tool_executor`,
/// so a rule approved here ("AlwaysAllow") is also visible to `/tools/execute`
/// and subagent ports, and vice versa.
pub struct RootToolPort {
    registry: Arc<ToolRegistry>,
    policy: ToolPermissionPolicy,
    session_rules: Arc<RwLock<HashSet<String>>>,
    bridge: Arc<PermissionBridge>,
    root_mode: Arc<std::sync::RwLock<crate::config::RootPermissionMode>>,
    effective_mode: Arc<std::sync::RwLock<crate::sandbox::EffectiveMode>>,
    agent: AgentExecutionContext,
    workdir: Option<PathBuf>,
    session_id: String,
}

impl RootToolPort {
    /// Build the port for a server-side root run. `workdir` is the session's
    /// bound worktree (`None` = main working dir); it is injected into every
    /// [`ToolContext`] so relative tool paths resolve inside the worktree.
    pub fn new(state: &DaemonState, session_id: &str, workdir: Option<PathBuf>) -> Self {
        Self {
            registry: Arc::clone(&state.tool_registry),
            policy: state.tool_executor.policy().clone(),
            session_rules: state.tool_executor.session_rules_handle(),
            bridge: Arc::clone(&state.permission_bridge),
            root_mode: Arc::clone(&state.root_mode),
            effective_mode: Arc::clone(&state.effective_mode),
            agent: AgentExecutionContext::root(SessionId::new(session_id)),
            workdir,
            session_id: session_id.to_string(),
        }
    }

    /// Test-only constructor: a full in-memory `DaemonState` is too heavy, so
    /// tests inject the registry / rules / bridge directly. The policy is
    /// rooted at `workdir` (falls back to the current directory).
    #[cfg(test)]
    fn new_for_test(
        registry: Arc<ToolRegistry>,
        session_rules: Arc<RwLock<HashSet<String>>>,
        bridge: Arc<PermissionBridge>,
        workdir: Option<PathBuf>,
    ) -> Self {
        let policy_root = workdir
            .clone()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        Self {
            registry,
            policy: ToolPermissionPolicy::new(policy_root),
            session_rules,
            bridge,
            root_mode: Arc::new(std::sync::RwLock::new(
                crate::config::RootPermissionMode::Normal,
            )),
            effective_mode: Arc::new(std::sync::RwLock::new(
                crate::sandbox::EffectiveMode::Normal,
            )),
            agent: AgentExecutionContext::root(SessionId::new("test")),
            workdir,
            session_id: "test".to_string(),
        }
    }

    fn fail(code: &str, message: impl Into<String>) -> ToolResponse {
        let message = message.into();
        ToolResponse {
            content: serde_json::json!({
                "success": false,
                "error": { "message": message, "code": code }
            })
            .to_string(),
            success: false,
        }
    }

    /// Resolve a policy `Ask`: approved rule / auto-approving mode short-
    /// circuit; otherwise escalate to the bridge and wait indefinitely (the
    /// human decides when to answer). On approval the session rule is
    /// persisted into the shared rule set.
    async fn resolve_ask(
        &self,
        perm: &crate::permissions::policy::PermissionRequest,
    ) -> Result<(), ToolResponse> {
        let rule_approved = self.session_rules.read().await.contains(&perm.session_rule);
        let mode_auto = self
            .root_mode
            .read()
            .map(|m| m.auto_approves(&perm.tool_name))
            .unwrap_or(false);
        if decide_ask(rule_approved, mode_auto) == AskDecision::Execute {
            return Ok(());
        }

        let mut approval = StructuredApproval::policy_ask(
            Uuid::new_v4().to_string(),
            self.session_id.clone(),
            perm.tool_name.clone(),
            perm.reason.clone(),
            perm.session_rule.clone(),
        );
        // Prefer path-shaped session rules as paths for structured consumers.
        if let Some(path) = perm.session_rule.strip_prefix("path:") {
            approval.paths = vec![path.to_string()];
        } else if let Some(cmd) = perm.session_rule.strip_prefix("command:") {
            approval.command = Some(cmd.to_string());
        }

        if !self.bridge.request_indefinite(approval).await {
            return Err(Self::fail(
                "permission_denied",
                format!(
                    "Permission denied for `{}`: {}",
                    perm.tool_name, perm.reason
                ),
            ));
        }

        // Approved: persist the rule so future calls (and /tools/execute,
        // which shares this rule set) skip the prompt.
        self.session_rules
            .write()
            .await
            .insert(perm.session_rule.clone());
        Ok(())
    }

    /// Guardian pre-check, copied from the headless `RegistryToolPort`:
    /// critical-risk shell commands are blocked before execution.
    fn guardian_block(&self, req: &ToolRequest) -> Option<ToolResponse> {
        if req.name != "execute_command" && req.name != "exec_command" {
            return None;
        }
        let cmd = req.arguments.get("command").and_then(|v| v.as_str())?;
        let risk = crate::runtime::guardian::classify_risk(cmd);
        if risk >= crate::runtime::guardian::RiskLevel::Critical {
            let content = format!(
                r#"{{"success":false,"error":"GUARDIAN BLOCK: critical-risk command rejected. {}"}}"#,
                cmd
            );
            return Some(ToolResponse {
                content,
                success: false,
            });
        }
        None
    }
}

#[async_trait]
impl ToolPort for RootToolPort {
    async fn execute(&self, req: ToolRequest) -> ToolResponse {
        // 1. Policy validation against the shared session rules.
        let decision = {
            let rules = self.session_rules.read().await;
            validate_tool_call_shared(
                &self.registry,
                &self.policy,
                &rules,
                &req.name,
                &req.arguments,
            )
        };
        match decision {
            Ok(PolicyDecision::Allow) => {}
            Ok(PolicyDecision::Ask(perm)) => {
                // 2. Ask → shared rules / root mode / indefinite bridge.
                if let Err(resp) = self.resolve_ask(&perm).await {
                    return resp;
                }
            }
            Err(e) => {
                return Self::fail(e.code.as_deref().unwrap_or("policy_error"), e.message);
            }
        }

        // 3. Guardian pre-check for shell tools.
        if let Some(resp) = self.guardian_block(&req) {
            return resp;
        }

        // 4. Registry execution with the session-bound workdir.
        let inv_id = req
            .invocation_id
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        // Ensure the turn snapshot exists when the caller supplies a turn id
        // (mirrors the /tools/execute handler; Plan mode skips capture inside
        // maybe_capture_pre_edit via EffectiveMode::Plan).
        if let Some(turn_id) = req.turn_id.as_deref() {
            if let Err(e) = self.registry.checkpoint_manager.begin_turn(turn_id) {
                tracing::warn!(error = %e, turn = %turn_id, "checkpoint begin_turn failed");
            }
        }
        let effective_mode = self.effective_mode.read().map(|m| *m).unwrap_or_default();
        let tool_context = ToolContext {
            agent: &self.agent,
            invocation_id: ToolInvocationId::new(inv_id),
            origin_turn_id: req.turn_id.as_deref(),
            workdir: self.workdir.as_deref(),
            effective_mode,
            checkpoint: Some(self.registry.checkpoint_store.as_ref()),
        };
        match self
            .registry
            .execute_with_context(&tool_context, &req.name, req.arguments)
            .await
        {
            Ok(output) => ToolResponse {
                content: output.content,
                success: true,
            },
            Err(e) => ToolResponse {
                content: format!("Error: {}", e.message),
                success: false,
            },
        }
    }

    fn definitions(&self) -> Vec<ToolDefinition> {
        self.registry
            .list()
            .into_iter()
            .map(|t| ToolDefinition::new(t.name(), t.description(), t.input_schema()))
            .collect()
    }
}

// ── Run registry + run/cancel endpoints (Task 4) ─────────────────────────────

use crate::agent::runtime::{
    run_agent_loop, ApiLlmPort, LoopHooks, LoopTurnState, MutexHistoryStore, RunLoopArgs,
    RuntimeConfig, StreamStyle,
};
use crate::api::{ApiClient, ChatMessage};
use crate::context::memory_session::SessionMessage;
use crate::context::MemorySessionManager;
use crate::prompts::PromptContext;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::sse::{Event, KeepAlive, Sse},
    Json,
};
use serde::Deserialize;
use std::collections::HashMap;
use std::convert::Infallible;
use std::time::Instant;
use tokio::sync::{mpsc, Mutex};
use tokio_stream::wrappers::UnboundedReceiverStream;
use tokio_stream::Stream;
use tokio_util::sync::CancellationToken;

/// One active server-side run for a session (v1: at most one per session).
pub struct SessionRun {
    pub run_id: String,
    pub cancel: CancellationToken,
    pub started_at: std::time::Instant,
}

/// Per-session claim registry enforcing one active run per session.
///
/// std RwLock: critical sections are single HashMap ops, never held across
/// `.await` (same rationale as `SessionWorkdirs` in state.rs).
#[derive(Clone, Default)]
pub struct RunRegistry {
    inner: Arc<std::sync::RwLock<HashMap<String, SessionRun>>>,
}

impl RunRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Claim the session for `run`. Err(409) when a run is already active.
    pub fn claim(&self, session_id: &str, run: SessionRun) -> Result<(), (StatusCode, String)> {
        let mut runs = self.inner.write().expect("session_runs lock poisoned");
        if runs.contains_key(session_id) {
            return Err((
                StatusCode::CONFLICT,
                format!("session {session_id} already has an active run"),
            ));
        }
        runs.insert(session_id.to_string(), run);
        Ok(())
    }

    /// Release the claim — only when the caller still owns it (run_id match),
    /// so a stale task can never release a newer run's claim.
    pub fn finish(&self, session_id: &str, run_id: &str) {
        let mut runs = self.inner.write().expect("session_runs lock poisoned");
        if runs.get(session_id).is_some_and(|r| r.run_id == run_id) {
            if let Some(run) = runs.remove(session_id) {
                tracing::debug!(
                    session_id,
                    run_id,
                    elapsed_ms = run.started_at.elapsed().as_millis(),
                    "session run finished"
                );
            }
        }
    }

    /// Signal cancellation. The claim itself is released by the run task's
    /// `finish` after its final save, so the final save always wins over a
    /// new run. Returns false when no run is active.
    pub fn cancel(&self, session_id: &str) -> bool {
        let runs = self.inner.read().expect("session_runs lock poisoned");
        match runs.get(session_id) {
            Some(run) => {
                run.cancel.cancel();
                true
            }
            None => false,
        }
    }

    /// Whether a run currently holds the session's claim.
    pub fn is_active(&self, session_id: &str) -> bool {
        self.inner
            .read()
            .expect("session_runs lock poisoned")
            .contains_key(session_id)
    }
}

/// Releases the session's run claim on drop. Instantiated at the top of the
/// spawned run task so EVERY exit path — normal return, error, cancel, and
/// panic/unwind — releases the claim; without it a panicking turn would leak
/// the claim (409 on every future run/update until daemon restart). Drop
/// delegates to [`RunRegistry::finish`], so the run_id ownership check still
/// applies (a stale guard can never release a newer run's claim).
struct RunClaimGuard {
    registry: RunRegistry,
    session_id: String,
    run_id: String,
}

impl Drop for RunClaimGuard {
    fn drop(&mut self) {
        self.registry.finish(&self.session_id, &self.run_id);
    }
}

#[derive(Debug, Deserialize)]
pub struct RunRequest {
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct RunResponse {
    pub run_id: String,
    pub session_id: String,
}

/// POST /api/v1/sessions/:id/run — spawn a server-side agent turn.
///
/// 400 empty message / 404 unknown session / 409 run already active; on
/// success 202 with the run id and the turn proceeds in a spawned task.
pub(crate) async fn post_run(
    State(state): State<Arc<DaemonState>>,
    Path(id): Path<String>,
    Json(body): Json<RunRequest>,
) -> Result<(StatusCode, Json<RunResponse>), (StatusCode, String)> {
    if body.message.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "message must not be empty".to_string(),
        ));
    }
    if state.session_manager.get(&id).await.is_none() {
        return Err((StatusCode::NOT_FOUND, format!("no such session: {id}")));
    }

    let run_id = Uuid::new_v4().to_string();
    let cancel = CancellationToken::new();
    let run = SessionRun {
        run_id: run_id.clone(),
        cancel: cancel.clone(),
        started_at: Instant::now(),
    };
    state.session_runs.claim(&id, run)?;

    let task_state = Arc::clone(&state);
    let task_session = id.clone();
    let task_run = run_id.clone();
    tokio::spawn(async move {
        // Drop guard releases the claim on every exit path, including panic.
        let _claim = RunClaimGuard {
            registry: task_state.session_runs.clone(),
            session_id: task_session.clone(),
            run_id: task_run.clone(),
        };
        run_session_turn(&task_state, &task_session, &task_run, body.message, cancel).await;
    });

    Ok((
        StatusCode::ACCEPTED,
        Json(RunResponse {
            run_id,
            session_id: id,
        }),
    ))
}

/// POST /api/v1/sessions/:id/cancel — cancel the session's active run.
/// 204 when a run was signalled, 404 when no run is active.
pub(crate) async fn post_cancel(
    State(state): State<Arc<DaemonState>>,
    Path(id): Path<String>,
) -> StatusCode {
    if state.session_runs.cancel(&id) {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

/// GET /api/v1/sessions/:id/events — SSE stream of the session's live
/// [`SessionEvent`]s, filtered from the single global hub by session id.
///
/// Live-only (v1: no cold replay) — events published before the client
/// connected are not resent; the persisted history (`GET /sessions/:id`)
/// is the catch-up path. A slow subscriber observes `Lagged` (drop-oldest).
/// 404 for an unknown session. Keep-alive comment every 15s.
pub(crate) async fn get_session_events(
    State(state): State<Arc<DaemonState>>,
    Path(id): Path<String>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, (StatusCode, String)> {
    if state.session_manager.get(&id).await.is_none() {
        return Err((StatusCode::NOT_FOUND, format!("no such session: {id}")));
    }

    let (tx, rx) = mpsc::unbounded_channel::<Result<Event, Infallible>>();
    // Subscribe before responding so no event is missed between connect and
    // stream start.
    let mut live = state.session_event_hub.subscribe();

    tokio::spawn(async move {
        loop {
            match live.recv().await {
                Ok(ev) => {
                    if ev.session_id != id {
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
                        "session events SSE subscriber lagged; oldest events dropped for this subscriber"
                    );
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    Ok(Sse::new(UnboundedReceiverStream::new(rx)).keep_alive(KeepAlive::default()))
}

/// Persist `history` as the session's full message list. Runs at turn start
/// and end, and mid-run whenever the loop emits `SaveSession` (via the sink's
/// save bridge): the start save makes the user message durable even if the
/// run dies, the mid-run saves checkpoint compaction/tool-round progress, and
/// the final save records the completed conversation. Tool-call pairing is
/// sanitized so a cancelled/failed run never leaves a dangling assistant
/// `tool_calls` without its `tool` results. Errors are logged, never fatal.
async fn save_session_history(
    sessions: &MemorySessionManager,
    session_id: &str,
    history: &[ChatMessage],
) {
    let mut history = history.to_vec();
    crate::api::types::sanitize_tool_call_pairing(&mut history);
    // ChatMessage -> SessionMessage via serde round-trip (the same conversion
    // update_session relies on from the TUI's PUT body).
    let messages: Vec<SessionMessage> = match serde_json::to_value(&history)
        .and_then(serde_json::from_value)
    {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(error = %e, session_id, "run: history conversion failed, save skipped");
            return;
        }
    };
    let Some(mut session) = sessions.get(session_id).await else {
        tracing::warn!(session_id, "run: session vanished before save");
        return;
    };
    session.messages = messages;
    session.updated_at = chrono::Utc::now();
    // Fully materialised write — clear any lazy index marker (mirrors update_session).
    session.lazy_message_count = None;
    if let Err(e) = sessions.save(&session).await {
        tracing::warn!(error = %e, session_id, "run: session save failed");
    }
}

/// Body of one spawned run: seed history from the persisted session, run the
/// shared agent loop against the run's cancel token, then persist the final
/// history. Terminal events (TurnDone/TurnError) flow through the sink.
async fn run_session_turn(
    state: &Arc<DaemonState>,
    session_id: &str,
    run_id: &str,
    message: String,
    cancel: CancellationToken,
) {
    let mut sink = DaemonEventSink::new(
        session_id.to_string(),
        run_id.to_string(),
        state.session_event_hub.clone(),
    );

    // 1. Seed history from the persisted session (SessionMessage -> ChatMessage
    // serde round-trip) and append the new user message.
    let Some(session) = state.session_manager.get(session_id).await else {
        sink.emit(RuntimeEvent::StreamError(format!(
            "session vanished before run start: {session_id}"
        )));
        return;
    };
    let mut seed: Vec<ChatMessage> =
        match serde_json::to_value(&session.messages).and_then(serde_json::from_value) {
            Ok(h) => h,
            Err(e) => {
                sink.emit(RuntimeEvent::StreamError(format!(
                    "history conversion failed: {e}"
                )));
                return;
            }
        };
    seed.push(ChatMessage::user(&message));

    // 2. Start save: the user message is durable even if the run dies.
    save_session_history(&state.session_manager, session_id, &seed).await;

    // 3. Live settings + LLM port (same wiring as the chat_stream handler).
    let settings = state
        .settings_handle
        .read()
        .expect("lock poisoned: settings")
        .clone();
    let client = ApiClient::with_clients(
        settings.clone(),
        state.http_client.clone(),
        state.http_client_stream.clone(),
    );
    let llm = ApiLlmPort::new(client);

    // 4. Root tool port bound to the session's worktree (None = main working_dir).
    let workdir = state.session_workdir(session_id);
    let tools = RootToolPort::new(state, session_id, workdir.clone());

    // 5. System prompt — same PromptContext chain as the headless runtime,
    // with cwd = the bound worktree or the daemon working dir.
    let cwd = workdir.unwrap_or_else(|| settings.storage.working_dir.clone());
    let prompt_ctx = PromptContext::default()
        .with_cwd(cwd.to_string_lossy().to_string())
        .with_sandbox("workspace-write")
        .with_approval("on-request")
        .with_codegraph_state(crate::mcp::codegraph::probe_install_state(&settings));
    let system_messages =
        crate::prompts::assemble_instructions(&settings, &prompt_ctx).system_messages;

    // 6. Per-run loop config/state; a fresh turn id per run.
    let config = RuntimeConfig {
        max_rounds: settings.agent.max_rounds.unwrap_or(100),
        plan_mode: false,
        subagent_timeout_secs: settings.agent.subagent.timeout_secs,
        context_window: crate::config::resolve_context_window(
            &settings.models.main,
            settings.models.context_window,
        ),
        max_tokens: settings.models.transport.max_tokens,
        session_id: session_id.to_string(),
        turn_id: Some(Uuid::new_v4().to_string()),
        agent_generation: 0,
        stream_max_retries: 2,
    };
    let store = MutexHistoryStore::new(Arc::new(Mutex::new(seed)));
    let history_handle = store.handle();
    // Mid-run persistence bridge (Task 5): from here on, every SaveSession
    // event the loop emits also triggers a spawned snapshot save.
    sink.set_save_bridge(Arc::clone(state), Arc::clone(&history_handle));
    let mut turn_state = LoopTurnState::default();

    // 7. Run the shared loop against the cancel token (subagent pattern):
    // cancellation drops the loop future; the final save below still runs.
    let loop_future = run_agent_loop(RunLoopArgs {
        llm: &llm,
        tools: &tools,
        events: &sink,
        history: &store,
        config: &config,
        state: &mut turn_state,
        stream_style: StreamStyle::default(),
        hooks: LoopHooks::default(),
        system_messages: &system_messages,
    });
    tokio::pin!(loop_future);
    let cancelled = cancel.cancelled();
    tokio::pin!(cancelled);
    tokio::select! {
        biased;
        _ = &mut cancelled => {
            sink.emit(RuntimeEvent::StreamError("run cancelled".to_string()));
        }
        result = &mut loop_future => {
            if let Err(e) = result {
                sink.emit(RuntimeEvent::StreamError(format!("run failed: {e}")));
            }
        }
    }

    // 8. Final save: the full (sanitized) history replaces the session messages.
    let final_history = history_handle.lock().await.clone();
    save_session_history(&state.session_manager, session_id, &final_history).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::runtime::ports::ToolPort;
    use crate::agent::runtime::{EventSink, RuntimeEvent};
    use crate::teams::permission_bridge::PermissionBridge;
    use crate::tools::ToolRegistry;
    use std::collections::HashSet;
    use std::time::Duration;
    use tokio::sync::RwLock;

    #[test]
    fn maps_runtime_events_to_session_events() {
        let (hub, mut rx) = tokio::sync::broadcast::channel(16);
        let sink = DaemonEventSink::new("s1".into(), "r1".into(), hub);
        sink.emit(RuntimeEvent::ContentDelta("hi".into()));
        sink.emit(RuntimeEvent::ReasoningDelta("think".into()));
        sink.emit(RuntimeEvent::ToolStart {
            name: "file_read".into(),
            args: serde_json::json!({"path":"a"}),
        });
        sink.emit(RuntimeEvent::SaveSession);
        let e1 = rx.try_recv().unwrap();
        assert_eq!(e1.kind, SessionEventKind::ContentDelta);
        assert_eq!(e1.data["text"], "hi");
        assert_eq!(e1.seq, 1);
        let e2 = rx.try_recv().unwrap();
        assert_eq!(e2.kind, SessionEventKind::ReasoningDelta);
        let e3 = rx.try_recv().unwrap();
        assert_eq!(e3.kind, SessionEventKind::ToolStart);
        assert_eq!(e3.data["name"], "file_read");
        let e4 = rx.try_recv().unwrap();
        assert_eq!(e4.kind, SessionEventKind::Save);
        // seq 单调递增
        assert_eq!(e4.seq, 4);
    }

    #[test]
    fn unmapped_variants_are_skipped() {
        // Connecting/PreparingTools/StreamDone/CompactionStarted 等不产生事件
        //（v1 不广播连接噪声）；StreamError → TurnError；StreamDone → TurnDone
        let (hub, mut rx) = tokio::sync::broadcast::channel(16);
        let sink = DaemonEventSink::new("s1".into(), "r1".into(), hub);
        sink.emit(RuntimeEvent::Connecting {
            attempt: 1,
            max_retries: 2,
        });
        sink.emit(RuntimeEvent::StreamDone {
            finish_reason: "stop".into(),
        });
        sink.emit(RuntimeEvent::StreamError("boom".into()));
        assert!(rx
            .try_recv()
            .is_ok_and(|e| e.kind == SessionEventKind::TurnDone));
        assert!(rx
            .try_recv()
            .is_ok_and(|e| e.kind == SessionEventKind::TurnError));
        assert!(rx.try_recv().is_err());
    }

    // ── RootToolPort ───────────────────────────────────────────────────────

    fn write_req(path: &str) -> ToolRequest {
        ToolRequest {
            name: "file_write".into(),
            arguments: serde_json::json!({"path": path, "content": "x"}),
            session_id: "test".into(),
            turn_id: None,
            invocation_id: Some("i1".into()),
            parallel: false,
        }
    }

    #[test]
    fn decide_ask_short_circuits_on_rule_or_mode() {
        assert_eq!(decide_ask(true, false), AskDecision::Execute);
        assert_eq!(decide_ask(true, true), AskDecision::Execute);
        assert_eq!(decide_ask(false, true), AskDecision::Execute);
        assert_eq!(decide_ask(false, false), AskDecision::Escalate);
    }

    #[tokio::test]
    async fn executes_in_bound_workdir() {
        let temp = tempfile::tempdir().expect("tempdir");
        let registry = Arc::new(ToolRegistry::with_project_root(temp.path(), 5));
        let rules = Arc::new(RwLock::new(HashSet::new()));
        let bridge = Arc::new(PermissionBridge::new(Duration::from_secs(5)));
        let port =
            RootToolPort::new_for_test(registry, rules, bridge, Some(temp.path().to_path_buf()));

        // file_write with a relative path is inside the workspace (policy
        // Allow) and must land under the session-bound workdir.
        let resp = port.execute(write_req("a.txt")).await;
        assert!(resp.success, "write should succeed: {}", resp.content);
        let written = std::fs::read_to_string(temp.path().join("a.txt")).expect("a.txt exists");
        assert_eq!(written, "x");
    }

    #[tokio::test]
    async fn ask_suspends_until_bridge_resolve() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace).expect("workspace");
        // Absolute path outside the workspace → policy Ask.
        let outside = temp.path().join("outside.txt");

        let registry = Arc::new(ToolRegistry::with_project_root(&workspace, 5));
        let rules = Arc::new(RwLock::new(HashSet::new()));
        let bridge = Arc::new(PermissionBridge::new(Duration::from_secs(5)));
        let port = Arc::new(RootToolPort::new_for_test(
            registry,
            Arc::clone(&rules),
            Arc::clone(&bridge),
            Some(workspace),
        ));

        let outside_str = outside.to_str().expect("utf8 path").to_string();
        let executing = {
            let port = Arc::clone(&port);
            tokio::spawn(async move { port.execute(write_req(&outside_str)).await })
        };

        // The execute call must suspend on a pending bridge approval.
        let mut request_id = None;
        for _ in 0..100 {
            let pending = bridge.pending().await;
            if let Some(p) = pending.first() {
                assert_eq!(p.tool, "file_write");
                assert!(p.session_rule.starts_with("path:"));
                request_id = Some(p.request_id.clone());
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let request_id = request_id.expect("approval request never appeared");
        assert!(
            !executing.is_finished(),
            "execute must stay suspended while the approval is pending"
        );
        assert!(!outside.exists(), "no side effect before approval");

        assert!(bridge.resolve(&request_id, true).await);
        let resp = executing.await.expect("execute join");
        assert!(resp.success, "approved write should run: {}", resp.content);
        assert_eq!(
            std::fs::read_to_string(&outside).expect("outside.txt written"),
            "x"
        );

        // The approved rule is persisted into the shared session rules so
        // /tools/execute and later server runs stay coherent.
        let approved_rules = rules.read().await;
        assert!(
            approved_rules.iter().any(|r| r.starts_with("path:")),
            "approved rule must be recorded: {approved_rules:?}"
        );
    }

    // ── RunRegistry ──────────────────────────────────────────────────────

    fn test_run(run_id: &str) -> SessionRun {
        SessionRun {
            run_id: run_id.to_string(),
            cancel: CancellationToken::new(),
            started_at: Instant::now(),
        }
    }

    #[test]
    fn claim_rejects_second_run() {
        let registry = RunRegistry::new();
        assert!(registry.claim("s1", test_run("r1")).is_ok());
        let (status, _) = registry.claim("s1", test_run("r2")).unwrap_err();
        assert_eq!(status, StatusCode::CONFLICT);
        // A different session is unaffected.
        assert!(registry.claim("s2", test_run("r3")).is_ok());
    }

    #[test]
    fn finish_releases_claim() {
        let registry = RunRegistry::new();
        registry.claim("s1", test_run("r1")).unwrap();
        // A stale run_id must not release the current owner's claim.
        registry.finish("s1", "other");
        assert!(registry.is_active("s1"));
        registry.finish("s1", "r1");
        assert!(!registry.is_active("s1"));
        assert!(registry.claim("s1", test_run("r2")).is_ok());
    }

    #[test]
    fn cancel_signals_token_and_reports_activity() {
        let registry = RunRegistry::new();
        assert!(!registry.cancel("nobody"));
        let run = test_run("r1");
        let token = run.cancel.clone();
        registry.claim("s1", run).unwrap();
        assert!(registry.cancel("s1"));
        assert!(token.is_cancelled());
        // The claim itself stays until the run task calls finish (final save
        // must complete before a new run can start).
        assert!(registry.is_active("s1"));
    }

    #[tokio::test]
    async fn panic_releases_claim_via_drop_guard() {
        let registry = RunRegistry::new();
        registry.claim("s1", test_run("r1")).unwrap();

        // Simulate a run task that panics mid-turn (e.g. a poisoned-lock
        // expect in the turn body).
        let guard_registry = registry.clone();
        let handle = tokio::spawn(async move {
            let _claim = RunClaimGuard {
                registry: guard_registry,
                session_id: "s1".to_string(),
                run_id: "r1".to_string(),
            };
            panic!("simulated turn panic");
        });
        let err = handle.await.expect_err("task must panic");
        assert!(err.is_panic());

        // The guard released the claim during unwinding: no 409 leak.
        assert!(!registry.is_active("s1"));
        assert!(registry.claim("s1", test_run("r2")).is_ok());
    }

    // ── Persistence bridge (Task 5) ───────────────────────────────────────

    #[tokio::test]
    async fn save_session_history_preserves_tool_call_pairing() {
        let temp = tempfile::tempdir().expect("tempdir");
        let sessions = MemorySessionManager::with_project_root(temp.path().to_path_buf());
        let session = sessions.create(Some("pairing")).await.expect("create");

        // History shaped like a completed tool round: assistant tool_calls
        // followed by the matching tool result.
        let tool_call = crate::api::ToolCall {
            id: "call_1".into(),
            r#type: "function".into(),
            function: crate::api::ToolCallFunction {
                name: "file_write".into(),
                arguments: r#"{"path":"a.txt","content":"x"}"#.into(),
            },
        };
        let history = vec![
            ChatMessage::user("write a.txt"),
            ChatMessage::assistant_with_tools(vec![tool_call]),
            ChatMessage::tool("call_1", r#"{"success":true}"#),
            ChatMessage::assistant("done"),
        ];
        save_session_history(&sessions, &session.id, &history).await;

        let persisted = sessions.get(&session.id).await.expect("session persists");
        assert_eq!(persisted.messages.len(), 4);
        let calls = persisted.messages[1]
            .tool_calls
            .as_ref()
            .expect("assistant tool_calls persisted");
        assert_eq!(calls[0].id, "call_1");
        assert_eq!(calls[0].function.name, "file_write");
        assert_eq!(persisted.messages[2].role, "tool");
        assert_eq!(
            persisted.messages[2].tool_call_id.as_deref(),
            Some("call_1")
        );

        // Reload from disk through a fresh manager: the pairing must survive
        // the JSON round-trip, not just the in-memory index.
        let reloaded = MemorySessionManager::with_project_root(temp.path().to_path_buf());
        let disk = reloaded
            .load(&session.id)
            .await
            .expect("load")
            .expect("session file on disk");
        assert_eq!(
            disk.messages[1]
                .tool_calls
                .as_ref()
                .expect("tool_calls on disk")[0]
                .id,
            "call_1"
        );
        assert_eq!(disk.messages[2].tool_call_id.as_deref(), Some("call_1"));
    }

    #[tokio::test]
    async fn save_session_history_repairs_dangling_tool_calls() {
        // A cancelled run can snapshot a history whose assistant tool_calls
        // have no results yet; the save must inject interrupted results so
        // the restored history stays API-compliant.
        let temp = tempfile::tempdir().expect("tempdir");
        let sessions = MemorySessionManager::with_project_root(temp.path().to_path_buf());
        let session = sessions.create(Some("repair")).await.expect("create");

        let history = vec![
            ChatMessage::user("hi"),
            ChatMessage::assistant_with_tools(vec![crate::api::ToolCall {
                id: "call_orphan".into(),
                r#type: "function".into(),
                function: crate::api::ToolCallFunction {
                    name: "file_read".into(),
                    arguments: r#"{"path":"a.txt"}"#.into(),
                },
            }]),
        ];
        save_session_history(&sessions, &session.id, &history).await;

        let persisted = sessions.get(&session.id).await.expect("session persists");
        assert_eq!(persisted.messages.len(), 3, "synthetic tool result added");
        assert_eq!(persisted.messages[2].role, "tool");
        assert_eq!(
            persisted.messages[2].tool_call_id.as_deref(),
            Some("call_orphan")
        );
    }
}
