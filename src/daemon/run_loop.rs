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
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

/// A single event in a daemon-run session, envelope for SSE fan-out.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEvent {
    /// Monotonically increasing sequence number within one session (across
    /// runs), starting at 1 — clients dedup/order by seq on reconnect.
    pub seq: u64,
    pub session_id: String,
    pub run_id: String,
    pub kind: SessionEventKind,
    /// Kind-specific payload (see [`DaemonEventSink::emit`] mapping).
    pub data: serde_json::Value,
}

/// The subset of runtime events worth broadcasting to clients (v1).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionEventKind {
    ContentDelta,
    ReasoningDelta,
    ToolStart,
    ToolResult,
    TurnDone,
    TurnError,
    /// Real-time context occupancy: prompt tokens of the most recent LLM call,
    /// pushed after every API response (and after auto-compaction) so web
    /// clients can render the context bar live during a turn.
    UsageUpdate,
    Save,
    PermissionRequired,
    AskUser,
    SyncLost,
    /// Turn context snapshot for inspector panels: system prompt layers,
    /// recalled memories, new messages, hook reminder, and token usage.
    /// Emitted once per run, after the loop exits and before the final save.
    TurnContext,
}

/// Broadcast channel over which one daemon run publishes [`SessionEvent`]s.
/// Subscribers use `subscribe()`; lagging receivers see `RecvError::Lagged`.
pub type SessionEventHub = tokio::sync::broadcast::Sender<SessionEvent>;

/// Per-session fixed-capacity replay buffer. Lives only for the daemon
/// process lifetime — after a restart it is empty and `after=` resumes
/// answer `SyncLost` (correctness never depends on the buffer; clients
/// fall back to `GET /sessions/:id`). Capacity comes from
/// `daemon.event_buffer_capacity` (default 1024, aligned with TRACE_HUB).
///
/// The type is `pub` so integration tests can drive `after=` replay through
/// the real HTTP/SSE stack; mutation stays crate-internal (`push` is called
/// only by the dual-write publishers).
pub struct SessionEventBuffer {
    events: VecDeque<SessionEvent>,
    capacity: usize,
}

impl SessionEventBuffer {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            // Cap only the upfront allocation (a huge configured capacity
            // must not pre-allocate megabytes); the stored capacity stays
            // authoritative. Floor at 1: with 0, `push` would never evict
            // and the ring would grow unbounded.
            events: VecDeque::with_capacity(capacity.clamp(1, 4096)),
            capacity: capacity.max(1),
        }
    }

    /// Push an event; evicts the oldest when full. Must be called at the same
    /// point the event is published to the hub so buffer and broadcast agree.
    pub(crate) fn push(&mut self, ev: SessionEvent) {
        if self.events.len() == self.capacity {
            self.events.pop_front();
        }
        self.events.push_back(ev);
    }

    // Read-side accessors: consumed by Task 3's `after=` replay path
    // (`plan_catch_up`) and by tests.
    pub(crate) fn oldest_seq(&self) -> Option<u64> {
        self.events.front().map(|e| e.seq)
    }

    pub(crate) fn latest_seq(&self) -> Option<u64> {
        self.events.back().map(|e| e.seq)
    }

    pub(crate) fn events_after(&self, after: u64) -> impl Iterator<Item = &SessionEvent> {
        self.events.iter().filter(move |e| e.seq > after)
    }
}

/// [`EventSink`] that translates runtime events into [`SessionEvent`]s and
/// broadcasts them on a per-run hub. Connection noise (Connecting,
/// PreparingTools, CompactionStarted, …) is intentionally dropped in v1.
///
/// `next_seq` is the session's shared counter (from
/// [`DaemonState::session_seq_counter`]), not per-sink: seq must stay
/// monotonic across runs of one session for client dedup/resume.
pub struct DaemonEventSink {
    session_id: String,
    run_id: String,
    hub: SessionEventHub,
    next_seq: Arc<AtomicU64>,
    /// Per-session replay buffer (from [`DaemonState::session_buffer`]):
    /// every published event is dual-written here so `after=` resume
    /// (Task 3) can replay what a reconnecting client missed.
    buffer: Arc<std::sync::RwLock<SessionEventBuffer>>,
    /// Set once the terminal TurnDone is published, so the loop's duplicate
    /// StreamDone at turn completion doesn't produce a second TurnDone.
    turn_done_published: AtomicBool,
    /// Mid-run persistence bridge (Task 5); `None` until the run task wires
    /// the live history handle in via [`DaemonEventSink::set_save_bridge`].
    save_bridge: Option<SaveBridge>,
    /// Per-project todo bridge; `None` until the run task wires it in via
    /// [`DaemonEventSink::set_todo_bridge`]. When set, `PlanUpdate` events
    /// update the session's project todo state and broadcast globally.
    todo_bridge: Option<TodoBridge>,
}

/// Handles the todo-update side of [`RuntimeEvent::PlanUpdate`]: the owning
/// project's todo state. `emit` is sync, so the update is always spawned.
struct TodoBridge {
    state: Arc<DaemonState>,
}

/// Handles the persistence side of [`RuntimeEvent::SaveSession`]: the owning
/// project's session store plus the live history handle. `emit` is sync, so
/// the save itself is always spawned, never run inline.
///
/// `save_gen` / `save_lock` implement the stale-overwrite guard: every save
/// (mid-run or turn-end) claims a generation and re-checks it under the write
/// lock, so a task holding an older snapshot can never overwrite a newer one.
struct SaveBridge {
    /// The session store that owns this session (multi-project routing);
    /// captured at run start so an unregistered-project mid-run still saves.
    sessions: MemorySessionManager,
    history: Arc<tokio::sync::Mutex<Vec<ChatMessage>>>,
    save_gen: Arc<AtomicU64>,
    save_lock: Arc<tokio::sync::Mutex<()>>,
}

impl DaemonEventSink {
    pub fn new(
        session_id: String,
        run_id: String,
        hub: SessionEventHub,
        next_seq: Arc<AtomicU64>,
        buffer: Arc<std::sync::RwLock<SessionEventBuffer>>,
    ) -> Self {
        Self {
            session_id,
            run_id,
            hub,
            next_seq,
            buffer,
            turn_done_published: AtomicBool::new(false),
            save_bridge: None,
            todo_bridge: None,
        }
    }

    /// Attach the mid-run persistence bridge: subsequent `SaveSession` events
    /// spawn a history snapshot save in addition to broadcasting `Save`.
    /// `save_gen` / `save_lock` are shared with the run task so the turn-end
    /// final save participates in the same generation sequence.
    fn set_save_bridge(
        &mut self,
        sessions: MemorySessionManager,
        history: Arc<tokio::sync::Mutex<Vec<ChatMessage>>>,
        save_gen: Arc<AtomicU64>,
        save_lock: Arc<tokio::sync::Mutex<()>>,
    ) {
        self.save_bridge = Some(SaveBridge {
            sessions,
            history,
            save_gen,
            save_lock,
        });
    }

    /// Attach the per-project todo bridge: subsequent `PlanUpdate` events
    /// spawn a todo state update + global broadcast routed to the session's
    /// project, so multi-project sessions never cross-contaminate.
    fn set_todo_bridge(&mut self, state: Arc<DaemonState>) {
        self.todo_bridge = Some(TodoBridge { state });
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
        let _ = self.hub.send(event.clone());
        // Dual-write into the replay buffer at the same point, so buffer and
        // broadcast never disagree on which seqs exist (Task 3 replays from
        // here on `after=` resume). std RwLock: single push, never held
        // across `.await`.
        self.buffer
            .write()
            .expect("session buffer lock poisoned")
            .push(event);
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
                // StreamDone fires at the end of EVERY LLM round (stream.rs forwards the
                // provider's finish_reason inline). A round that ends in tool calls is a
                // round boundary, not turn end: publish TurnDone{tool_calls} so web/desktop
                // clients can split per-round bubbles (their SSE loop treats tool_calls as
                // a boundary, not turn completion). This does NOT consume
                // `turn_done_published` - the terminal round still publishes its TurnDone
                // exactly once. Anthropic `tool_use` is normalized to "tool_calls" in
                // src/api, so this holds across providers.
                tracing::info!(finish_reason = %finish_reason, "StreamDone received");
                if finish_reason == "tool_calls" {
                    tracing::info!("round-boundary: publishing TurnDone(tool_calls)");
                    self.publish(
                        SessionEventKind::TurnDone,
                        serde_json::json!({ "finish_reason": finish_reason }),
                    );
                    return;
                }
                // The loop re-emits StreamDone at turn completion (loop_.rs), so a
                // terminal reason is seen twice; publish TurnDone exactly once per run.
                if self.turn_done_published.swap(true, Ordering::Relaxed) {
                    return;
                }
                tracing::info!(finish_reason = %finish_reason, "turn end: publishing TurnDone");
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
            RuntimeEvent::UsageUpdate { prompt_tokens } => {
                self.publish(
                    SessionEventKind::UsageUpdate,
                    serde_json::json!({ "prompt_tokens": prompt_tokens }),
                );
            }
            RuntimeEvent::SaveSession => {
                self.publish(SessionEventKind::Save, serde_json::json!({}));
                // Mid-run persistence bridge (Task 5): snapshot the live
                // history and persist it in a spawned task — `emit` is sync
                // and must never block the run loop on disk I/O. The
                // generation is claimed here (event order) so a stale task
                // self-skips when a newer save was claimed meanwhile.
                if let Some(bridge) = &self.save_bridge {
                    let gen = claim_save_generation(&bridge.save_gen);
                    let sessions = bridge.sessions.clone();
                    let history = Arc::clone(&bridge.history);
                    let save_gen = Arc::clone(&bridge.save_gen);
                    let save_lock = Arc::clone(&bridge.save_lock);
                    let session_id = self.session_id.clone();
                    tokio::spawn(async move {
                        let snapshot = history.lock().await.clone();
                        guarded_save(
                            &sessions,
                            &session_id,
                            &snapshot,
                            gen,
                            &save_gen,
                            &save_lock,
                        )
                        .await;
                    });
                }
            }
            RuntimeEvent::PlanUpdate(args) => {
                if let Some(bridge) = &self.todo_bridge {
                    let items = parse_plan_update(&args);
                    let state = Arc::clone(&bridge.state);
                    let session_id = self.session_id.clone();
                    tokio::spawn(async move {
                        state.apply_todos_update(&session_id, items).await;
                    });
                }
            }
            // v1: connection noise and UI-only signals are not broadcast.
            _ => {}
        }
    }
}

// ── RootToolPort (Task 3) ────────────────────────────────────────────────────

use crate::agent::runtime::ports::{InteractionPort, ToolPort, ToolRequest, ToolResponse};
#[cfg(test)]
use crate::agent::SessionId;
use crate::agent::{AgentExecutionContext, ToolContext, ToolInvocationId};
use crate::api::ToolDefinition;
use crate::daemon::state::{DaemonState, MESSAGE_QUEUE_MAX_DEPTH};
use crate::permissions::policy::{PolicyDecision, ToolPermissionPolicy};
use crate::teams::permission_bridge::{PermissionBridge, StructuredApproval};
use crate::tools::executor::validate_tool_call_shared;
use crate::tools::{Tool, ToolRegistry};
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
    interaction_bridge: Arc<crate::daemon::interaction_bridge::InteractionBridge>,
    permission_modes: crate::permissions::PermissionModeStore,
    agent: AgentExecutionContext,
    /// The session's effective working root (bound worktree > project root >
    /// main working_dir). Injected into every [`ToolContext`] so relative
    /// tool paths resolve there, and used as the permission-policy root so
    /// validation and execution never diverge.
    root: PathBuf,
    /// Per-project checkpoint handles (snapshots live under the project the
    /// session belongs to, not the daemon's main project).
    checkpoint_manager: Arc<crate::tools::CheckpointManager>,
    checkpoint_store: Arc<crate::tools::CheckpointStore>,
    /// Per-project task routing: `task_management` calls are intercepted and
    /// delegated to the session's project task manager instead of the
    /// registry's global default, mirroring `MemoryRouter` for tasks.
    task_router: Arc<crate::tasks::TaskRouter>,
    session_id: String,
    /// Session event hub: pushes PermissionRequired/AskUser events to SSE
    /// clients so they can prompt without polling pending-permissions.
    hub: SessionEventHub,
    run_id: String,
    next_seq: Arc<AtomicU64>,
    /// Per-session replay buffer (from [`DaemonState::session_buffer`]);
    /// `publish_event` dual-writes into it, same contract as
    /// [`DaemonEventSink::publish`].
    buffer: Arc<std::sync::RwLock<SessionEventBuffer>>,
}

impl RootToolPort {
    /// Build the port for a server-side root run. `root` is the session's
    /// effective working root (see [`DaemonState::effective_session_root`]):
    /// tool path resolution, the permission-policy boundary, and checkpoint
    /// storage all follow it.
    pub fn new(
        state: &DaemonState,
        session_id: &str,
        run_id: &str,
        root: PathBuf,
        agent: AgentExecutionContext,
    ) -> Self {
        let (checkpoint_manager, checkpoint_store) = state.checkpoints_for_project(&root);
        Self {
            registry: Arc::clone(&state.tool_registry),
            policy: ToolPermissionPolicy::new(root.clone()),
            session_rules: state.tool_executor.session_rules_handle(),
            bridge: Arc::clone(&state.permission_bridge),
            interaction_bridge: Arc::clone(&state.interaction_bridge),
            agent,
            permission_modes: state.permission_modes.clone(),
            root,
            checkpoint_manager,
            checkpoint_store,
            task_router: Arc::clone(&state.task_router),
            session_id: session_id.to_string(),
            hub: state.session_event_hub.clone(),
            run_id: run_id.to_string(),
            next_seq: state.session_seq_counter(session_id),
            buffer: state.session_buffer(session_id),
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
            checkpoint_manager: registry.checkpoint_manager.clone(),
            checkpoint_store: registry.checkpoint_store.clone(),
            registry,
            policy: ToolPermissionPolicy::new(policy_root.clone()),
            session_rules,
            bridge,
            interaction_bridge: Arc::new(
                crate::daemon::interaction_bridge::InteractionBridge::new(),
            ),
            permission_modes: crate::permissions::PermissionModeStore::new(),
            agent: AgentExecutionContext::root(SessionId::new("test")),
            root: policy_root.clone(),
            task_router: Arc::new(crate::tasks::TaskRouter::new(
                Arc::new(crate::tasks::TaskManagementTool::new()),
                policy_root,
            )),
            session_id: "test".to_string(),
            hub: tokio::sync::broadcast::channel(16).0,
            run_id: "test".to_string(),
            next_seq: Arc::new(AtomicU64::new(1)),
            buffer: Arc::new(std::sync::RwLock::new(SessionEventBuffer::new(16))),
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

    /// Publish a SessionEvent to the hub (permission/ask notifications).
    fn publish_event(&self, kind: SessionEventKind, data: serde_json::Value) {
        let event = SessionEvent {
            seq: self.next_seq.fetch_add(1, Ordering::Relaxed),
            session_id: self.session_id.clone(),
            run_id: self.run_id.clone(),
            kind,
            data,
        };
        let _ = self.hub.send(event.clone());
        // Dual-write into the replay buffer, same contract as
        // `DaemonEventSink::publish` (buffer and broadcast must agree).
        self.buffer
            .write()
            .expect("session buffer lock poisoned")
            .push(event);
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
            .permission_modes
            .get(&self.root)
            .root_mode
            .auto_approves(&perm.tool_name);
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

        // Push the permission request to SSE clients so they can prompt
        // without polling GET /pending-permissions.
        self.publish_event(
            SessionEventKind::PermissionRequired,
            serde_json::json!({
                "request_id": &approval.request_id,
                "tool": &perm.tool_name,
                "reason": &perm.reason,
                "rule": &perm.session_rule,
            }),
        );

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

        // 3b. Per-project task routing: delegate `task_management` to the
        // session's project task manager instead of the registry's global
        // default, so multi-project sessions never cross-contaminate.
        if req.name == "task_management" {
            let task_mgr = self.task_router.for_project(&self.root).await;
            match task_mgr.execute(req.arguments.clone()).await {
                Ok(output) => {
                    return ToolResponse {
                        content: output.content,
                        success: true,
                    }
                }
                Err(e) => {
                    return Self::fail(e.code.as_deref().unwrap_or("task_error"), e.message);
                }
            }
        }

        // 4. Registry execution with the session's effective root as workdir.
        let inv_id = req
            .invocation_id
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        // Ensure the turn snapshot exists when the caller supplies a turn id
        // (mirrors the /tools/execute handler; Plan mode skips capture inside
        // maybe_capture_pre_edit via EffectiveMode::Plan).
        if let Some(turn_id) = req.turn_id.as_deref() {
            if let Err(e) = self.checkpoint_manager.begin_turn(turn_id) {
                tracing::warn!(error = %e, turn = %turn_id, "checkpoint begin_turn failed");
            }
        }
        let effective_mode = self.permission_modes.get(&self.root).effective_mode;
        let tool_context = ToolContext {
            agent: &self.agent,
            invocation_id: ToolInvocationId::new(inv_id),
            origin_turn_id: req.turn_id.as_deref(),
            workdir: Some(self.root.as_path()),
            effective_mode,
            checkpoint: Some(self.checkpoint_store.as_ref()),
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

#[async_trait]
impl InteractionPort for RootToolPort {
    /// Route `ask_user_question` through the InteractionBridge: mint a payload
    /// from the tool args, block until a frontend resolves it, return the
    /// answer string. The loop dispatches here (not ToolPort::execute) for
    /// interaction-class tools (loop_.rs dispatch_ask).
    async fn ask_user_question(&self, args: &serde_json::Value) -> String {
        let payload =
            crate::daemon::interaction_bridge::QuestionPayload::from_args(args, &self.session_id);
        self.publish_event(
            SessionEventKind::AskUser,
            serde_json::json!({
                "request_id": &payload.request_id,
                "question": &payload.question,
                "options": &payload.options,
                "multi_select": payload.multi_select,
            }),
        );
        self.interaction_bridge.request(payload).await
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
    extract::{Path, Query, State},
    http::StatusCode,
    response::sse::{Event, KeepAlive, Sse},
    Json,
};
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

    /// Atomically replace the finishing owner's claim with a continuation.
    /// A stale completion can never replace a newer run.
    fn handoff(
        &self,
        session_id: &str,
        finished_run_id: &str,
        next: SessionRun,
    ) -> Result<(), SessionRun> {
        let mut runs = self.inner.write().expect("session_runs lock poisoned");
        if runs
            .get(session_id)
            .is_some_and(|run| run.run_id == finished_run_id)
        {
            runs.insert(session_id.to_string(), next);
            Ok(())
        } else {
            Err(next)
        }
    }

    /// Signal cancellation. The claim itself is released or handed off by the
    /// scheduler after the run task's final save, so that save always wins
    /// over a new run. Returns false when no run is active.
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

    #[cfg(test)]
    fn active_run_id(&self, session_id: &str) -> Option<String> {
        self.inner
            .read()
            .expect("session_runs lock poisoned")
            .get(session_id)
            .map(|run| run.run_id.clone())
    }
}

/// Serialized inputs to the daemon-owned continuation scheduler.
#[derive(Debug)]
pub(crate) enum BackgroundSchedulerEvent {
    ResultReady {
        session_id: String,
    },
    RunFinished {
        session_id: String,
        run_id: String,
        final_save_succeeded: bool,
    },
    /// A task group transitioned to all-results-recorded (pinged by the
    /// coordinator). Hints only — readiness is re-verified by claiming.
    TaskGroupReady {
        session_id: String,
    },
}

impl BackgroundSchedulerEvent {
    fn session_id(&self) -> &str {
        match self {
            Self::ResultReady { session_id }
            | Self::RunFinished { session_id, .. }
            | Self::TaskGroupReady { session_id } => session_id,
        }
    }
}

struct BackgroundContinuation {
    session_id: String,
    run_id: String,
    cancel: CancellationToken,
    message: String,
    plan_mode: bool,
}

/// Completes the session's run claim on drop. With a live scheduler it queues
/// `RunFinished` together with the final-persistence outcome; only a successful
/// save permits an atomic handoff to pending background results. Without a
/// scheduler it releases directly through [`RunRegistry::finish`]. Every exit
/// path — normal return, error, cancel, and panic/unwind — therefore avoids
/// leaking the claim, and the run-id ownership check prevents a stale guard
/// from releasing or replacing a newer run.
struct RunClaimGuard {
    registry: RunRegistry,
    session_id: String,
    run_id: String,
    completion_tx: Option<mpsc::UnboundedSender<BackgroundSchedulerEvent>>,
    final_save_succeeded: bool,
}

impl Drop for RunClaimGuard {
    fn drop(&mut self) {
        if self.completion_tx.as_ref().is_some_and(|tx| {
            tx.send(BackgroundSchedulerEvent::RunFinished {
                session_id: self.session_id.clone(),
                run_id: self.run_id.clone(),
                final_save_succeeded: self.final_save_succeeded,
            })
            .is_ok()
        }) {
            return;
        }
        self.registry.finish(&self.session_id, &self.run_id);
    }
}

async fn run_under_claim<F>(mut claim: RunClaimGuard, turn: F)
where
    F: std::future::Future<Output = bool>,
{
    claim.final_save_succeeded = turn.await;
}

fn structured_background_continuation(
    results: &[crate::tools::execution::background::BackgroundResult],
) -> String {
    serde_json::json!({
        "type": "background_task_results",
        "results": results,
    })
    .to_string()
}

/// User message carrying a claimed task-group delivery into a synthesis turn.
/// Mirrors the TUI's `process_continuation` format so the model sees the same
/// contract regardless of which client (or the daemon) initiated the turn.
fn structured_task_group_continuation(results: &[crate::agent::ChildResult]) -> String {
    let batch = serde_json::to_string(results)
        .unwrap_or_else(|e| format!("{{\"serialize_error\":\"{e}\"}}"));
    format!(
        "<child-results>\n{batch}\n</child-results>\n\n\
         A background subagent group completed. Synthesize these results into your current \
         work and continue; do not repeat the subagent's steps.\n\n\
         Continue from the delivered subagent results."
    )
}

/// Try to turn one ready root task group into a daemon-owned synthesis run.
/// This is the server-side counterpart of the TUI's `poll_ready_task_groups`:
/// without it, a completed subagent group strands in the coordinator whenever
/// no polling client (TUI) is attached — e.g. web sessions.
///
/// Returns `None` (leaving the group ready for a later attempt) when the
/// session is busy, blocked by a failed-save barrier, or has no ready group.
async fn prepare_task_group_continuation(
    state: &Arc<DaemonState>,
    session_id: &str,
) -> Option<BackgroundContinuation> {
    // Same durability barrier as background-result continuations: a failed
    // final save must be retried by an explicit run before any synthesized
    // history is layered on top.
    if state.background_continuation_is_blocked(session_id).await {
        return None;
    }
    state.resolve_session(session_id).await.as_ref()?;

    // Serialize with user-driven runs: if a turn is active the group stays
    // ready and is re-attempted from the `RunFinished` re-check.
    let run_id = Uuid::new_v4().to_string();
    let cancel = CancellationToken::new();
    let next_run = SessionRun {
        run_id: run_id.clone(),
        cancel: cancel.clone(),
        started_at: Instant::now(),
    };
    if state.session_runs.claim(session_id, next_run).is_err() {
        return None;
    }

    let root = match state.root_context(session_id).await {
        Ok(root) => root,
        Err(error) => {
            tracing::warn!(%session_id, %error, "task-group continuation: no root context");
            state.session_runs.finish(session_id, &run_id);
            return None;
        }
    };
    let generation = state.coordinator.current_generation(&root.session_id).await;
    let delivery = match state
        .coordinator
        .claim_ready_root_group(&root, generation)
        .await
    {
        Ok(delivery) => delivery,
        Err(error) => {
            tracing::warn!(%session_id, %error, "task-group continuation: claim failed");
            state.session_runs.finish(session_id, &run_id);
            return None;
        }
    };
    let Some(delivery) = delivery else {
        state.session_runs.finish(session_id, &run_id);
        return None;
    };

    // Mirror the client-driven claim endpoint's broadcast so listening UIs
    // refresh their subagent panels regardless of who claimed.
    let project = state.effective_session_root(session_id).await;
    state.broadcast_global(
        crate::daemon::global_events::GlobalEventKind::TaskGroupResult,
        serde_json::json!({
            "task_group_id": delivery.group_id.as_str(),
            "session_id": session_id,
            "generation": delivery.generation,
            "project": project,
            "result_count": delivery.results.len(),
            "statuses": delivery.results.iter().map(|r| r.status).collect::<Vec<_>>(),
        }),
    );

    tracing::info!(
        %session_id,
        group_id = %delivery.group_id.as_str(),
        results = delivery.results.len(),
        "daemon claimed ready task group; spawning synthesis continuation"
    );
    Some(BackgroundContinuation {
        session_id: session_id.to_string(),
        run_id,
        cancel,
        message: structured_task_group_continuation(&delivery.results),
        plan_mode: false,
    })
}

/// Prepare one scheduler event. Results move to a recoverable claim here and
/// are acknowledged only after `run_session_turn` persists its start message.
/// `RunFinished` is emitted after the turn returns, so normal handoff still
/// occurs only after final save; an unacknowledged claim is requeued first.
async fn prepare_background_continuation(
    state: &Arc<DaemonState>,
    event: BackgroundSchedulerEvent,
) -> Option<BackgroundContinuation> {
    let (session_id, finished_run) = match event {
        BackgroundSchedulerEvent::ResultReady { session_id } => (session_id, None),
        BackgroundSchedulerEvent::RunFinished {
            session_id,
            run_id,
            final_save_succeeded,
        } => (session_id, Some((run_id, final_save_succeeded))),
        // The scheduler loop routes this variant to
        // `prepare_task_group_continuation` instead; handled here only for
        // exhaustiveness.
        BackgroundSchedulerEvent::TaskGroupReady { session_id } => (session_id, None),
    };

    // A run that exited before acknowledging its start-save never consumed
    // the claim. Put that batch back ahead of newer pending results before
    // deciding whether to hand off the registry claim.
    if let Some((run_id, _)) = finished_run.as_ref() {
        state
            .requeue_background_result_claim(&session_id, run_id.as_str())
            .await;
    }

    // A failed final save is not a successful completion seam. Release only
    // the matching failed owner, leave all pending/requeued results intact,
    // and block result-ready events until an explicit retry establishes newly
    // durable history. A successful completion clears that retry barrier.
    if let Some((run_id, false)) = finished_run.as_ref() {
        state.block_background_continuation(&session_id).await;
        state.session_runs.finish(&session_id, run_id);
        return None;
    }
    if finished_run.is_some() {
        state.allow_background_continuation(&session_id).await;
    } else if state.background_continuation_is_blocked(&session_id).await {
        return None;
    }
    let finished_run_id = finished_run.map(|(run_id, _)| run_id);

    // A forged/stale session id is retained for legacy recovery but cannot
    // acquire a run claim or consume model history.
    if state.resolve_session(&session_id).await.is_none() {
        if let Some(run_id) = finished_run_id {
            state.session_runs.finish(&session_id, &run_id);
        }
        return None;
    }

    if !state.has_pending_background_results(&session_id).await {
        if let Some(run_id) = finished_run_id {
            state.session_runs.finish(&session_id, &run_id);
        }
        return None;
    }

    let run_id = Uuid::new_v4().to_string();
    let cancel = CancellationToken::new();
    let next_run = SessionRun {
        run_id: run_id.clone(),
        cancel: cancel.clone(),
        started_at: Instant::now(),
    };
    let claimed = match finished_run_id {
        Some(finished_run_id) => state
            .session_runs
            .handoff(&session_id, &finished_run_id, next_run)
            .is_ok(),
        None => state.session_runs.claim(&session_id, next_run).is_ok(),
    };
    if !claimed {
        return None;
    }

    let results = state
        .claim_background_results_for_session(&session_id, &run_id)
        .await;
    if results.is_empty() {
        state.session_runs.finish(&session_id, &run_id);
        return None;
    }
    Some(BackgroundContinuation {
        session_id,
        run_id,
        cancel,
        message: structured_background_continuation(&results),
        plan_mode: false,
    })
}

/// Start the session's next queued user message (submitted while the run slot
/// was busy) as a daemon-owned turn. Last priority: only reached when a
/// finished run leaves no pending background results and no ready task group,
/// so old-turn continuations always settle before a queued new turn starts.
///
/// The failed-save barrier applies equally here — queued messages must not
/// layer new history on top of an unpersisted run; an explicit run (which
/// re-persists) clears the barrier.
async fn prepare_message_continuation(
    state: &Arc<DaemonState>,
    session_id: &str,
) -> Option<BackgroundContinuation> {
    if state.background_continuation_is_blocked(session_id).await {
        return None;
    }
    state.resolve_session(session_id).await.as_ref()?;
    if !state.has_queued_messages(session_id).await {
        return None;
    }

    let run_id = Uuid::new_v4().to_string();
    let cancel = CancellationToken::new();
    let next_run = SessionRun {
        run_id: run_id.clone(),
        cancel: cancel.clone(),
        started_at: Instant::now(),
    };
    // Plain claim: every earlier path that returned None either finished the
    // previous owner's registry claim or never owned it. A lost race simply
    // fails here and the queue waits for the next RunFinished.
    state.session_runs.claim(session_id, next_run).ok()?;

    let queued = match state.claim_queued_message(session_id, &run_id).await {
        Some(queued) => queued,
        None => {
            state.session_runs.finish(session_id, &run_id);
            return None;
        }
    };
    let remaining = state.message_queue_depth(session_id).await;
    tracing::debug!(
        session_id,
        run_id = %run_id,
        message_id = %queued.message_id,
        remaining,
        "scheduler starting queued user message"
    );
    Some(BackgroundContinuation {
        session_id: session_id.to_string(),
        run_id,
        cancel,
        message: queued.message,
        plan_mode: queued.plan_mode,
    })
}

fn spawn_claimed_session_turn(
    state: Arc<DaemonState>,
    session_id: String,
    run_id: String,
    message: String,
    plan_mode: bool,
    cancel: CancellationToken,
) {
    tokio::spawn(async move {
        let claim = RunClaimGuard {
            registry: state.session_runs.clone(),
            session_id: session_id.clone(),
            run_id: run_id.clone(),
            completion_tx: state.background_scheduler_sender(),
            final_save_succeeded: false,
        };
        run_under_claim(
            claim,
            run_session_turn(&state, &session_id, &run_id, message, plan_mode, cancel),
        )
        .await;
    });
}

pub(crate) fn spawn_background_continuation_scheduler(
    state: std::sync::Weak<DaemonState>,
    mut events: mpsc::UnboundedReceiver<BackgroundSchedulerEvent>,
) {
    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            let Some(state) = state.upgrade() else {
                break;
            };
            let session_id = event.session_id().to_string();
            let continuation = match event {
                BackgroundSchedulerEvent::TaskGroupReady { .. } => {
                    // An idle session may also hold queued user messages:
                    // drain them whenever the group was not ready (or the
                    // claim was lost to a user run).
                    match prepare_task_group_continuation(&state, &session_id).await {
                        Some(continuation) => Some(continuation),
                        None => prepare_message_continuation(&state, &session_id).await,
                    }
                }
                event => match prepare_background_continuation(&state, event).await {
                    Some(continuation) => Some(continuation),
                    // No background-result continuation happened (nothing
                    // pending, blocked, or the claim moved on). The session may
                    // be idle now with a ready task group waiting — e.g. the
                    // group completed while this run was still active. Last,
                    // drain any user message queued while the run was active.
                    None => match prepare_task_group_continuation(&state, &session_id).await {
                        Some(continuation) => Some(continuation),
                        None => prepare_message_continuation(&state, &session_id).await,
                    },
                },
            };
            if let Some(continuation) = continuation {
                spawn_claimed_session_turn(
                    state,
                    continuation.session_id,
                    continuation.run_id,
                    continuation.message,
                    continuation.plan_mode,
                    continuation.cancel,
                );
            }
        }
    });
}

#[derive(Debug, Deserialize)]
pub struct RunRequest {
    pub message: String,
    /// Forward the frontend's Plan mode so the server-side loop omits tool
    /// definitions and runs the planner (mirrors `RuntimeConfig::plan_mode`).
    /// Absent (e.g. the web client has no plan toggle) defaults to `false`.
    #[serde(default)]
    pub plan_mode: bool,
    /// When a run is already active: `true` (default) parks the message in
    /// the session's FIFO queue — the scheduler starts it after the current
    /// run settles (pending background results and ready task groups first).
    /// `false` preserves the legacy immediate-409 contract.
    #[serde(default = "queue_default_true")]
    pub queue: bool,
}

const fn queue_default_true() -> bool {
    true
}

#[derive(Debug, Serialize)]
pub struct RunResponse {
    /// Empty string when the turn was queued instead of started.
    pub run_id: String,
    pub session_id: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub queued: bool,
    /// 1-based queue position, present only when queued.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queue_position: Option<usize>,
}

/// POST /api/v1/sessions/:id/run — spawn a server-side agent turn.
///
/// 400 empty message / 404 unknown session. On success 202: either the turn
/// starts immediately (`run_id` set), or — when a run is already active and
/// `queue != false` — the message joins the session FIFO (`queued: true`,
/// empty `run_id`; the scheduler assigns the run later and clients adopt the
/// run id from the first SSE event). 409 only when `queue=false` collides
/// with an active run; 429 when the queue is at `MESSAGE_QUEUE_MAX_DEPTH`.
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
    if state.resolve_session(&id).await.is_none() {
        return Err((StatusCode::NOT_FOUND, format!("no such session: {id}")));
    }

    let run_id = Uuid::new_v4().to_string();
    let cancel = CancellationToken::new();
    let run = SessionRun {
        run_id: run_id.clone(),
        cancel: cancel.clone(),
        started_at: Instant::now(),
    };
    if state.session_runs.claim(&id, run).is_err() {
        if !body.queue {
            return Err((
                StatusCode::CONFLICT,
                format!("session {id} already has an active run"),
            ));
        }
        let queued = state
            .enqueue_message(&id, body.message, body.plan_mode)
            .await
            .map_err(|_| {
                (
                    StatusCode::TOO_MANY_REQUESTS,
                    format!("session {id} message queue full (max {MESSAGE_QUEUE_MAX_DEPTH})"),
                )
            })?;
        let depth = state.message_queue_depth(&id).await;
        tracing::debug!(
            session_id = %id,
            message_id = %queued.message_id,
            depth,
            "run busy; user message queued"
        );
        return Ok((
            StatusCode::ACCEPTED,
            Json(RunResponse {
                run_id: String::new(),
                session_id: id,
                queued: true,
                queue_position: Some(depth),
            }),
        ));
    }

    spawn_claimed_session_turn(
        Arc::clone(&state),
        id.clone(),
        run_id.clone(),
        body.message,
        body.plan_mode,
        cancel,
    );

    Ok((
        StatusCode::ACCEPTED,
        Json(RunResponse {
            run_id,
            session_id: id,
            queued: false,
            queue_position: None,
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

#[derive(Debug, Serialize)]
pub struct QueueResponse {
    pub session_id: String,
    pub messages: Vec<crate::daemon::state::QueuedMessage>,
}

/// GET /api/v1/sessions/:id/queue — list the session's queued user messages
/// (pending only; the claimed head is already being persisted by its run).
pub(crate) async fn get_queue(
    State(state): State<Arc<DaemonState>>,
    Path(id): Path<String>,
) -> Json<QueueResponse> {
    Json(QueueResponse {
        session_id: id.clone(),
        messages: state.queued_messages_snapshot(&id).await,
    })
}

/// DELETE /api/v1/sessions/:id/queue — drop every still-pending message.
/// 404 unknown session; 200 with the dropped count otherwise.
pub(crate) async fn delete_queue(
    State(state): State<Arc<DaemonState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if state.resolve_session(&id).await.is_none() {
        return Err(StatusCode::NOT_FOUND);
    }
    let dropped = state.clear_queued_messages(&id).await;
    Ok(Json(serde_json::json!({ "dropped": dropped })))
}

/// DELETE /api/v1/sessions/:id/queue/:message_id — retract one queued
/// message. 404 unknown session or already-started/gone message.
pub(crate) async fn delete_queued_message(
    State(state): State<Arc<DaemonState>>,
    Path((id, message_id)): Path<(String, String)>,
) -> StatusCode {
    if state.resolve_session(&id).await.is_none() {
        return StatusCode::NOT_FOUND;
    }
    if state.remove_queued_message(&id, &message_id).await {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

/// Query parameters for `GET /sessions/:id/events`.
#[derive(Debug, Deserialize)]
pub struct SessionEventsQuery {
    pub after: Option<u64>,
}

/// Replay/attach decision for `after=` subscriptions (design §2.2).
#[derive(Debug)]
pub(crate) enum CatchUp {
    /// No `after` — live-only (existing default).
    LiveOnly,
    /// `after >= latest` — nothing missed, attach live directly.
    UpToDate,
    /// Buffer covers `after + 1` — replay these, then attach live (dedup by seq).
    Replay(Vec<SessionEvent>),
    /// `after + 1 < oldest` (or buffer empty) — send SyncLost with latest seq.
    SyncLost { latest_seq: u64 },
}

pub(crate) fn plan_catch_up(after: Option<u64>, buf: &SessionEventBuffer) -> CatchUp {
    let Some(after) = after else {
        return CatchUp::LiveOnly;
    };
    match (buf.oldest_seq(), buf.latest_seq()) {
        (_, Some(latest)) if after >= latest => CatchUp::UpToDate,
        (Some(oldest), Some(_)) if after + 1 >= oldest => {
            CatchUp::Replay(buf.events_after(after).cloned().collect())
        }
        // Buffer empty (daemon restarted) or requested seq evicted.
        _ => CatchUp::SyncLost {
            latest_seq: buf.latest_seq().unwrap_or(0),
        },
    }
}

/// Build the out-of-band `sync_lost` event for one connection (design §2.3).
/// `seq: 0` and empty `run_id` mark it as control-plane, not run output.
pub(crate) fn sync_lost_event(session_id: &str, reason: &str, latest_seq: u64) -> SessionEvent {
    SessionEvent {
        seq: 0,
        session_id: session_id.to_string(),
        run_id: String::new(),
        kind: SessionEventKind::SyncLost,
        data: serde_json::json!({ "reason": reason, "latest_seq": latest_seq }),
    }
}

/// A live session-event subscription opened by [`subscribe_session_events`]:
/// the `after=` replay/attach decision, the initial dedup seam, and the live
/// broadcast receiver.
///
/// Seam invariant: the receiver is subscribed to the hub *before* the caller
/// replays [`CatchUp::Replay`] events, so no seam event can be lost; the
/// caller dedups by dropping live events with `seq <= seam_seq`.
pub(crate) struct SessionEventSubscription {
    /// Replay/attach decision for the `after` cursor (design §2.2).
    pub catch_up: CatchUp,
    /// First live seq to forward — everything `<= seam_seq` was already
    /// replayed or is skipped by the cursor.
    pub seam_seq: u64,
    /// Live receiver for the session hub.
    pub live: tokio::sync::broadcast::Receiver<SessionEvent>,
}

/// Subscribe a caller to a session's event stream with an optional `after`
/// cursor. Mirrors the `GET /sessions/:id/events` replay/attach semantics
/// (design §2.2): compute [`CatchUp`], subscribe to the hub BEFORE the replay
/// snapshot is consumed, and derive the initial `seam_seq`.
pub(crate) fn subscribe_session_events(
    after: Option<u64>,
    buffer: &Arc<std::sync::RwLock<SessionEventBuffer>>,
    hub: &SessionEventHub,
) -> SessionEventSubscription {
    let catch_up = {
        let buf = buffer.read().expect("session buffer lock poisoned");
        plan_catch_up(after, &buf)
    };
    // Subscribe BEFORE replaying so seam events can't be lost; dedup below.
    let live = hub.subscribe();
    let seam_seq = match &catch_up {
        CatchUp::Replay(evs) => evs.last().map(|e| e.seq).unwrap_or(0),
        CatchUp::SyncLost { latest_seq } => *latest_seq,
        CatchUp::LiveOnly | CatchUp::UpToDate => after.unwrap_or(0),
    };
    SessionEventSubscription {
        catch_up,
        seam_seq,
        live,
    }
}

/// Handle a `Lagged` error on a live session-event subscription (design §2.3):
/// jump the seam to the latest buffered seq and build the out-of-band
/// `sync_lost` event (`reason: "lagged"`) for that one subscriber. Other
/// subscribers are unaffected. Shared by the SSE handler and the upcoming
/// WebSocket endpoint.
pub(crate) fn plan_lagged_resync(
    session_id: &str,
    seam_seq: &mut u64,
    buffer: &Arc<std::sync::RwLock<SessionEventBuffer>>,
) -> SessionEvent {
    let latest = buffer
        .read()
        .expect("session buffer lock poisoned")
        .latest_seq()
        .unwrap_or(*seam_seq);
    let ev = sync_lost_event(session_id, "lagged", latest);
    *seam_seq = latest; // skip everything up to latest; client resyncs
    ev
}

/// GET /api/v1/sessions/:id/events — SSE stream of the session's events.
///
/// Without `after`: live-only (default, unchanged). With `after=<seq>`:
/// replay buffered events with `seq > after` in order, then attach live;
/// live events with `seq <= ` the last replayed seq are dropped (dedup at
/// the seam). If `after` fell out of the buffer window the client receives a
/// `sync_lost` event (`{ reason, latest_seq }`) — recovery contract: do a
/// full `GET /sessions/:id`, realign on the latest turn state, then
/// re-subscribe (without `after`, or with the newest seq). If a live
/// subscriber falls behind the broadcast window (`Lagged`), that connection
/// alone receives a `sync_lost` event (`reason: "lagged"`) and the stream
/// stays open; other subscribers are unaffected. 404 for unknown
/// session. Keep-alive comment every 15s.
pub(crate) async fn get_session_events(
    State(state): State<Arc<DaemonState>>,
    Path(id): Path<String>,
    Query(query): Query<SessionEventsQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, (StatusCode, String)> {
    if state.resolve_session(&id).await.is_none() {
        return Err((StatusCode::NOT_FOUND, format!("no such session: {id}")));
    }

    let buffer = state.session_buffer(&id);
    let SessionEventSubscription {
        catch_up,
        mut seam_seq,
        mut live,
    } = subscribe_session_events(query.after, &buffer, &state.session_event_hub);

    let (tx, rx) = mpsc::unbounded_channel::<Result<Event, Infallible>>();

    // Replay the catch-up decision into the SSE channel. The hub was already
    // subscribed by `subscribe_session_events`, so seam events can't be lost.
    match &catch_up {
        CatchUp::Replay(evs) => {
            for ev in evs {
                let data = serde_json::to_string(ev).unwrap_or_default();
                if tx.send(Ok(Event::default().data(data))).is_err() {
                    return Ok(
                        Sse::new(UnboundedReceiverStream::new(rx)).keep_alive(KeepAlive::default())
                    );
                }
            }
        }
        CatchUp::SyncLost { latest_seq } => {
            let ev = sync_lost_event(&id, "evicted", *latest_seq);
            let data = serde_json::to_string(&ev).unwrap_or_default();
            let _ = tx.send(Ok(Event::default().data(data)));
            // Keep the stream open; the client decides to resubscribe (§2.3).
        }
        CatchUp::LiveOnly | CatchUp::UpToDate => {}
    }

    tokio::spawn(async move {
        loop {
            match live.recv().await {
                Ok(ev) => {
                    if ev.session_id != id || ev.seq <= seam_seq {
                        continue; // other session, or already replayed
                    }
                    seam_seq = ev.seq;
                    let data = serde_json::to_string(&ev).unwrap_or_default();
                    if tx.send(Ok(Event::default().data(data))).is_err() {
                        return; // client disconnected
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(
                        target: "wgenty::daemon",
                        lagged = n,
                        "session events SSE subscriber lagged; sending sync_lost to this subscriber"
                    );
                    let ev = plan_lagged_resync(&id, &mut seam_seq, &buffer);
                    let data = serde_json::to_string(&ev).unwrap_or_default();
                    if tx.send(Ok(Event::default().data(data))).is_err() {
                        return;
                    }
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    Ok(Sse::new(UnboundedReceiverStream::new(rx)).keep_alive(KeepAlive::default()))
}

/// Parse `update_plan` tool args (`{ "plan": [{ "step": "...", "status": "..." }] }`)
/// into [`TodoItem`]s. Malformed items are silently skipped (the TUI applies
/// the same lenient parsing in `plan_panel.rs`).
fn parse_plan_update(args: &serde_json::Value) -> Vec<crate::tasks::TodoItem> {
    args.get("plan")
        .and_then(|p| p.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    let content = item.get("step")?.as_str()?.to_string();
                    let status = item.get("status")?.as_str()?.to_string();
                    Some(crate::tasks::TodoItem {
                        content,
                        status,
                        active_form: String::new(),
                        subagent: None,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Claim the next save generation for a run. Monotonic per run; claimed at
/// event/save time so generation order matches the order snapshots logically
/// supersede each other.
fn claim_save_generation(save_gen: &AtomicU64) -> u64 {
    save_gen.fetch_add(1, Ordering::SeqCst) + 1
}

/// Persist `snapshot` unless a newer save was claimed after `gen`. The
/// re-check happens under the per-run write lock, closing the check-then-write
/// race: the write that lands last always belongs to the latest claimed
/// generation, so a stale mid-run snapshot can never overwrite a newer save
/// (in particular the turn-end final save).
async fn guarded_save(
    sessions: &MemorySessionManager,
    session_id: &str,
    snapshot: &[ChatMessage],
    gen: u64,
    save_gen: &AtomicU64,
    save_lock: &tokio::sync::Mutex<()>,
) -> bool {
    let _write = save_lock.lock().await;
    if save_gen.load(Ordering::SeqCst) != gen {
        // A newer snapshot was claimed meanwhile; its save carries newer
        // state — writing ours would be a stale overwrite.
        return false;
    }
    save_session_history(sessions, session_id, snapshot).await
}

/// Persist `history` as the session's full message list. Runs at turn start
/// and end, and mid-run whenever the loop emits `SaveSession` (via the sink's
/// save bridge): the start save makes the user message durable even if the
/// run dies, the mid-run saves checkpoint compaction/tool-round progress, and
/// the final save records the completed conversation. Tool-call pairing is
/// sanitized so a cancelled/failed run never leaves a dangling assistant
/// `tool_calls` without its `tool` results. Returns whether persistence
/// completed; errors are logged for callers to handle at their lifecycle seam.
async fn save_session_history(
    sessions: &MemorySessionManager,
    session_id: &str,
    history: &[ChatMessage],
) -> bool {
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
            return false;
        }
    };
    let Some(mut session) = sessions.get(session_id).await else {
        tracing::warn!(session_id, "run: session vanished before save");
        return false;
    };
    session.messages = messages;
    session.updated_at = chrono::Utc::now();
    // Run saves participate in the same version sequence as PUT overwrites.
    session.version += 1;
    // Fully materialised write — clear any lazy index marker (mirrors update_session).
    session.lazy_message_count = None;
    if let Err(e) = sessions.save(&session).await {
        tracing::warn!(error = %e, session_id, "run: session save failed");
        return false;
    }
    true
}

/// Derive a session title from the first user message: collapse whitespace,
/// cap at 48 chars, ellipsis when truncated. `None` when the seed has no
/// user message (or it is empty) — the session keeps its previous name.
fn title_from_first_user_message(seed: &[ChatMessage]) -> Option<String> {
    let first = seed.iter().find(|m| m.role == "user")?;
    let collapsed = first
        .content
        .as_deref()?
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if collapsed.is_empty() {
        return None;
    }
    const MAX_TITLE_CHARS: usize = 48;
    if collapsed.chars().count() > MAX_TITLE_CHARS {
        Some(format!(
            "{}…",
            collapsed.chars().take(MAX_TITLE_CHARS).collect::<String>()
        ))
    } else {
        Some(collapsed)
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
    plan_mode: bool,
    cancel: CancellationToken,
) -> bool {
    let mut sink = DaemonEventSink::new(
        session_id.to_string(),
        run_id.to_string(),
        state.session_event_hub.clone(),
        state.session_seq_counter(session_id),
        state.session_buffer(session_id),
    );

    // 1. Resolve the owning session store + persisted session (multi-project
    // routing), seed history from it (SessionMessage -> ChatMessage serde
    // round-trip), and append the new user message. The manager is captured
    // for every later save so an unregistered-project mid-run still persists.
    let Some((sessions, session)) = state.resolve_session(session_id).await else {
        sink.emit(RuntimeEvent::StreamError(format!(
            "session vanished before run start: {session_id}"
        )));
        return false;
    };
    let mut seed: Vec<ChatMessage> =
        match serde_json::to_value(&session.messages).and_then(serde_json::from_value) {
            Ok(h) => h,
            Err(e) => {
                sink.emit(RuntimeEvent::StreamError(format!(
                    "history conversion failed: {e}"
                )));
                return false;
            }
        };
    seed.push(ChatMessage::user(&message));
    let seed_len_before_run = seed.len(); // for TurnContext new_messages slice

    // 2. Start save: the user message is durable even if the run dies. A
    // background batch and any queued-message claim remain claimed (and
    // snapshot-visible) until this succeeds; failure requeues them for a
    // later continuation attempt.
    if !save_session_history(&sessions, session_id, &seed).await {
        state
            .requeue_background_result_claim(session_id, run_id)
            .await;
        state.requeue_message_claim(session_id, run_id).await;
        sink.emit(RuntimeEvent::StreamError(
            "run start history save failed".to_string(),
        ));
        return false;
    }
    state.ack_background_result_claim(session_id, run_id).await;
    state.ack_message_claim(session_id, run_id).await;

    // 2b. Auto-title unnamed sessions from the first user message. Sessions
    // created without an explicit name default `name` to the session id (a
    // UUID) — the web rail and the TUI resume list would otherwise show raw
    // UUIDs. Explicitly named sessions (`name != id`) are never touched. The
    // reload after the seed save keeps `messages` current so the rename
    // cannot clobber the just-saved history.
    if let Ok(Some(mut session)) = sessions.load(session_id).await {
        if session.name == session_id {
            if let Some(title) = title_from_first_user_message(&seed) {
                session.name = title;
                if let Err(e) = sessions.save(&session).await {
                    tracing::warn!(error = %e, session = session_id, "auto-title save failed");
                }
            }
        }
    }

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

    // 4. Root tool port bound to the session's effective root (bound worktree
    // > the session's project > main working_dir). Tool path resolution, the
    // permission-policy boundary, and checkpoint storage all follow it.
    let root = state.effective_session_root(session_id).await;
    let tools = {
        let agent = match state.root_context(session_id).await {
            Ok(ctx) => ctx,
            Err(e) => {
                sink.emit(RuntimeEvent::StreamError(format!("root context: {e}")));
                return false;
            }
        };
        RootToolPort::new(state, session_id, run_id, root.clone(), agent)
    };

    // 5. System prompt — same PromptContext chain as the headless runtime,
    // with cwd = the session's effective root. Retain `layers` for the
    // TurnContext inspector event (previously discarded).
    let cwd = root;
    let mut prompt_ctx = PromptContext::default()
        .with_cwd(cwd.to_string_lossy().to_string())
        .with_sandbox("workspace-write")
        .with_approval("on-request")
        .with_codegraph_state(crate::mcp::codegraph::probe_install_state_for(
            &cwd, &settings,
        ));

    // 5a. Per-turn memory recall (mirrors TUI turn.rs:229). Daemon previously
    // skipped this — a functional gap. Now recalls cross-session memories and
    // injects them into the prompt context + retains metadata for inspector.
    let recalled = crate::context::inject::MemoryContextInjector::recall(
        &message,
        &state.memory_manager,
        settings.storage.memory.recall_top_n,
        settings.storage.memory.recall_min_effective_importance as f64,
        None,
    )
    .await;
    if !recalled.text.is_empty() {
        prompt_ctx.memories = recalled
            .text
            .lines()
            .filter(|l| l.starts_with("- ["))
            .map(|l| l.to_string())
            .collect();
    }

    // 5b. Hook reminder: fire UserPromptSubmit hooks, collect injection
    // fragments, build a <system-reminder> block (mirrors TUI agent/mod.rs
    // 280-326). The daemon's HookManager (runtime/hooks) already handles
    // PreToolUse/PostToolUse for tool execution; this adds the turn-level
    // prompt-submit path that was previously daemon-only absent.
    let hook_ctx = crate::runtime::hooks::types::HookContext {
        event: "UserPromptSubmit".to_string(),
        tool_name: None,
        tool_input: Some(serde_json::Value::String(message.clone())),
        tool_result: None,
        session_id: Some(session_id.to_string()),
        working_directory: cwd.to_string_lossy().to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        comet_phase: None,
        workflow_state: None,
        variables: Default::default(),
    };
    let outcomes = match tokio::time::timeout(
        std::time::Duration::from_secs(10),
        state.hook_manager.fire(
            &crate::runtime::hooks::HookEvent::UserPromptSubmit,
            &hook_ctx,
            None,
            None,
        ),
    )
    .await
    {
        Ok(v) => v,
        Err(_) => {
            tracing::warn!("UserPromptSubmit hook timed out (10s), skipping");
            Vec::new()
        }
    };
    let injections = crate::runtime::hooks::collect_injections(&outcomes);
    let reminder = crate::prompts::build_user_turn_reminder(&prompt_ctx, &injections);

    // Prepend reminder to the user message in seed history (same as TUI).
    if let Some(ref r) = reminder {
        if let Some(user_msg) = seed.iter_mut().rev().find(|m| m.role == "user") {
            let original = user_msg.content.take().unwrap_or_default();
            user_msg.content = Some(format!("{}\n\n{}", r.to_model, original));
        }
    }

    let assembled = crate::prompts::assemble_instructions(&settings, &prompt_ctx);
    let system_messages = assembled.system_messages;
    let prompt_layers = assembled.layers; // retained for TurnContext

    // 6. Per-run loop config/state; a fresh turn id per run.
    let config = RuntimeConfig {
        max_rounds: settings.agent.max_rounds.unwrap_or(100),
        plan_mode,
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
    // event the loop emits also triggers a spawned snapshot save. The save
    // generation counter + write lock are shared with the sink bridge so the
    // turn-end final save participates in the same sequence (a stale in-flight
    // snapshot save can never overwrite a newer one).
    let save_gen = Arc::new(AtomicU64::new(0));
    let save_lock = Arc::new(Mutex::new(()));
    sink.set_save_bridge(
        sessions.clone(),
        Arc::clone(&history_handle),
        Arc::clone(&save_gen),
        Arc::clone(&save_lock),
    );
    // Per-project todo bridge: `PlanUpdate` events route to the session's
    // project todo state via `TodoRouter`, mirroring the save bridge pattern.
    sink.set_todo_bridge(Arc::clone(state));
    let mut turn_state = LoopTurnState::default();
    let token_counter = crate::api::token_counter::TokenCounter::new();

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
        hooks: LoopHooks {
            interaction: Some(&tools),
            token_counter: Some(&token_counter),
            ..LoopHooks::default()
        },
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
    // Claims the newest generation, so any still-in-flight mid-run save
    // self-skips instead of overwriting this final state with a stale snapshot.
    let final_history = history_handle.lock().await.clone();
    let gen = claim_save_generation(&save_gen);
    let final_save_succeeded = guarded_save(
        &sessions,
        session_id,
        &final_history,
        gen,
        &save_gen,
        &save_lock,
    )
    .await;
    if !final_save_succeeded {
        sink.emit(RuntimeEvent::StreamError(
            "run final history save failed; retry the turn".to_string(),
        ));
    }

    // 9. TurnContext: broadcast inspector data (layers, recalled memories,
    // new messages, reminder, token usage). Emitted once per run after the
    // loop exits and the final save completes (clients keep their session
    // subscription briefly open past TurnDone to receive it).
    let new_messages: Vec<_> = final_history
        .iter()
        .skip(seed_len_before_run)
        .map(|m| {
            serde_json::json!({
                "role": m.role,
                "content": m.content.as_deref().unwrap_or("").chars().take(500).collect::<String>(),
            })
        })
        .collect();
    let layers_json: Vec<_> = prompt_layers
        .iter()
        .map(|l| {
            serde_json::json!({
                "label": l.label,
                "source": format!("{:?}", l.source),
                "char_count": l.char_count,
            })
        })
        .collect();
    let memories_json: Vec<_> = recalled
        .memories
        .iter()
        .map(|m| {
            serde_json::json!({
                "importance": m.importance,
                "memory_type": m.memory_type,
                "content_preview": m.content_preview,
            })
        })
        .collect();
    let usage_json = serde_json::json!({
        "prompt_tokens": token_counter.turn_input_tokens(),
        "completion_tokens": token_counter.turn_output_tokens(),
        "total_tokens": token_counter.used_tokens(),
        // Context-window occupancy at the last API call — the same measure
        // the TUI context bar renders (drops after auto-compaction). Unlike
        // prompt_tokens (a per-turn sum over rounds) this is not additive.
        "context_tokens": token_counter.last_prompt_tokens(),
    });
    sink.publish(
        SessionEventKind::TurnContext,
        serde_json::json!({
            "layers": layers_json,
            "recalled_memories": memories_json,
            "new_messages": new_messages,
            "reminder": reminder.as_ref().map(|r| serde_json::json!({
                "to_model": r.to_model,
                "to_transcript": r.to_transcript,
            })),
            "usage": usage_json,
        }),
    );
    final_save_succeeded
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

    fn ev(seq: u64) -> SessionEvent {
        SessionEvent {
            seq,
            session_id: "s".into(),
            run_id: "r".into(),
            kind: SessionEventKind::Save,
            data: serde_json::json!({}),
        }
    }

    #[test]
    fn catch_up_matrix() {
        let mut buf = SessionEventBuffer::new(4);
        for seq in 3..=6 {
            buf.push(ev(seq));
        }
        assert!(matches!(plan_catch_up(None, &buf), CatchUp::LiveOnly));
        assert!(matches!(plan_catch_up(Some(6), &buf), CatchUp::UpToDate));
        match plan_catch_up(Some(4), &buf) {
            CatchUp::Replay(evs) => {
                assert_eq!(evs.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![5, 6])
            }
            other => panic!("expected Replay, got {other:?}"),
        }
        assert!(matches!(
            plan_catch_up(Some(1), &buf),
            CatchUp::SyncLost { latest_seq: 6 }
        ));
        let empty = SessionEventBuffer::new(4);
        assert!(matches!(
            plan_catch_up(Some(9), &empty),
            CatchUp::SyncLost { latest_seq: 0 }
        ));
    }

    /// The extracted subscription primitive must compute the same `after=`
    /// catch-up decision and seam as the SSE handler (design §2.2), and
    /// subscribe to the hub before the caller replays (seam invariant).
    #[test]
    fn subscribe_session_events_plans_catch_up_and_seam() {
        let buffer = Arc::new(std::sync::RwLock::new(SessionEventBuffer::new(4)));
        {
            let mut buf = buffer.write().expect("buffer lock poisoned");
            for seq in 3..=6 {
                buf.push(ev(seq));
            }
        }
        let (hub, _keep) = tokio::sync::broadcast::channel(8);

        // No cursor → live-only, seam starts at 0.
        let sub = subscribe_session_events(None, &buffer, &hub);
        assert!(matches!(&sub.catch_up, CatchUp::LiveOnly));
        assert_eq!(sub.seam_seq, 0);

        // Buffer covers after+1 → replay [5,6], seam = last replayed seq.
        let sub = subscribe_session_events(Some(4), &buffer, &hub);
        match &sub.catch_up {
            CatchUp::Replay(evs) => {
                assert_eq!(evs.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![5, 6])
            }
            other => panic!("expected Replay, got {other:?}"),
        }
        assert_eq!(sub.seam_seq, 6);

        // Cursor fell out of the buffer → SyncLost, seam = latest buffered seq.
        let sub = subscribe_session_events(Some(1), &buffer, &hub);
        assert!(matches!(&sub.catch_up, CatchUp::SyncLost { latest_seq: 6 }));
        assert_eq!(sub.seam_seq, 6);

        // Cursor already at/after latest → nothing missed, seam = cursor.
        let sub = subscribe_session_events(Some(6), &buffer, &hub);
        assert!(matches!(&sub.catch_up, CatchUp::UpToDate));
        assert_eq!(sub.seam_seq, 6);
    }

    /// The lagged-resync helper advances the seam past the latest buffered
    /// seq and returns the `sync_lost` event — the behavior both the SSE
    /// handler and the upcoming WebSocket endpoint need (design §2.3).
    #[test]
    fn lagged_resync_advances_seam_and_builds_sync_lost() {
        let buffer = Arc::new(std::sync::RwLock::new(SessionEventBuffer::new(4)));
        {
            let mut buf = buffer.write().expect("buffer lock poisoned");
            for seq in 1..=10 {
                buf.push(ev(seq));
            }
        }
        let mut seam = 8;
        let sync = plan_lagged_resync("s", &mut seam, &buffer);
        assert_eq!(sync.kind, SessionEventKind::SyncLost);
        assert_eq!(sync.data["reason"], "lagged");
        assert_eq!(sync.data["latest_seq"], 10);
        assert_eq!(seam, 10);
    }

    /// Cursor resume (`after=3`): replay covers `seq > after` only, then the
    /// live tail continues from `seam_seq` with no duplicate and no gap.
    #[test]
    fn subscribe_cursor_replay_seams_into_live_without_gap_or_dup() {
        let buffer = Arc::new(std::sync::RwLock::new(SessionEventBuffer::new(8)));
        {
            let mut buf = buffer.write().expect("buffer lock poisoned");
            for seq in 3..=6 {
                buf.push(ev(seq));
            }
        }
        let (hub, _keep) = tokio::sync::broadcast::channel(16);

        let mut sub = subscribe_session_events(Some(3), &buffer, &hub);
        let replayed: Vec<u64> = match &sub.catch_up {
            CatchUp::Replay(events) => events.iter().map(|e| e.seq).collect(),
            other => panic!("expected Replay after cursor resume, got {other:?}"),
        };
        assert_eq!(replayed, vec![4, 5, 6]);
        assert_eq!(sub.seam_seq, 6);

        // Live publishes after subscription. A duplicate of the seam event
        // must be filtered out; events after the seam must flow through.
        hub.send(ev(6)).expect("hub send");
        hub.send(ev(7)).expect("hub send");
        hub.send(ev(8)).expect("hub send");

        let mut live = Vec::new();
        while let Ok(event) = sub.live.try_recv() {
            if event.seq > sub.seam_seq {
                live.push(event.seq);
            }
        }
        assert_eq!(live, vec![7, 8]);

        // Seam invariant: replay ∪ live is exactly 4..=8 — contiguous,
        // nothing duplicated, nothing dropped.
        let mut combined = replayed;
        combined.extend(live);
        assert_eq!(combined, vec![4, 5, 6, 7, 8]);
    }

    /// Cursor slipped out of the replay window → `SyncLost{latest_seq}` with
    /// the seam advanced to `latest_seq`; an empty buffer (fresh daemon)
    /// reports `latest_seq: 0`.
    #[test]
    fn subscribe_sync_lost_on_stale_cursor_and_empty_buffer() {
        let buffer = Arc::new(std::sync::RwLock::new(SessionEventBuffer::new(4)));
        {
            let mut buf = buffer.write().expect("buffer lock poisoned");
            for seq in 3..=6 {
                buf.push(ev(seq));
            }
        }
        let (hub, _keep) = tokio::sync::broadcast::channel(16);

        // `after = 1`: 1 + 1 < oldest_seq(3) → the cursor is gone.
        let sub = subscribe_session_events(Some(1), &buffer, &hub);
        assert!(matches!(sub.catch_up, CatchUp::SyncLost { latest_seq: 6 }));
        assert_eq!(sub.seam_seq, 6);

        // Empty buffer → nothing to resume from.
        let empty = Arc::new(std::sync::RwLock::new(SessionEventBuffer::new(4)));
        let sub = subscribe_session_events(Some(9), &empty, &hub);
        assert!(matches!(sub.catch_up, CatchUp::SyncLost { latest_seq: 0 }));
        assert_eq!(sub.seam_seq, 0);
    }

    /// Two subscribers on the same hub see the identical live sequence and
    /// draining one never consumes the other's events.
    #[test]
    fn multiple_subscribers_receive_identical_independent_sequence() {
        let buffer = Arc::new(std::sync::RwLock::new(SessionEventBuffer::new(8)));
        let (hub, _keep) = tokio::sync::broadcast::channel(16);

        let mut sub_a = subscribe_session_events(None, &buffer, &hub);
        let mut sub_b = subscribe_session_events(None, &buffer, &hub);
        assert!(matches!(sub_a.catch_up, CatchUp::LiveOnly));
        assert!(matches!(sub_b.catch_up, CatchUp::LiveOnly));

        for seq in 1..=5 {
            hub.send(ev(seq)).expect("hub send");
        }

        let mut a_seqs = Vec::new();
        while let Ok(event) = sub_a.live.try_recv() {
            a_seqs.push(event.seq);
        }
        let mut b_seqs = Vec::new();
        while let Ok(event) = sub_b.live.try_recv() {
            b_seqs.push(event.seq);
        }

        assert_eq!(a_seqs, vec![1, 2, 3, 4, 5]);
        assert_eq!(b_seqs, vec![1, 2, 3, 4, 5]);
    }

    /// On an empty buffer `plan_lagged_resync` has no newer cursor to jump to,
    /// so it keeps the current seam and reports that as `latest_seq`.
    #[test]
    fn lagged_resync_on_empty_buffer_keeps_seam() {
        let buffer = Arc::new(std::sync::RwLock::new(SessionEventBuffer::new(4)));
        let mut seam = 7;
        let sync = plan_lagged_resync("s", &mut seam, &buffer);
        assert_eq!(sync.kind, SessionEventKind::SyncLost);
        assert_eq!(sync.data["reason"], "lagged");
        assert_eq!(sync.data["latest_seq"], 7);
        assert_eq!(seam, 7);
    }

    #[test]
    fn buffer_evicts_oldest_when_full() {
        let mut buf = SessionEventBuffer::new(3);
        for seq in 1..=5 {
            buf.push(ev(seq));
        }
        assert_eq!(buf.oldest_seq(), Some(3));
        assert_eq!(buf.latest_seq(), Some(5));
        let after: Vec<u64> = buf.events_after(3).map(|e| e.seq).collect();
        assert_eq!(after, vec![4, 5]);
    }

    #[test]
    fn buffer_empty_bounds() {
        let buf = SessionEventBuffer::new(3);
        assert_eq!(buf.oldest_seq(), None);
        assert_eq!(buf.latest_seq(), None);
        assert_eq!(buf.events_after(0).count(), 0);
    }

    #[tokio::test]
    async fn lagged_subscriber_receives_sync_lost() {
        // 小容量 hub + 不消费的 receiver，制造 Lagged。
        let (hub, _keep) = tokio::sync::broadcast::channel(2);
        let mut slow = hub.subscribe();
        for seq in 1..=10u64 {
            let _ = hub.send(ev(seq));
        }
        // 模拟 get_session_events 的 Lagged 分支逻辑：
        let n = match slow.recv().await {
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => n,
            other => panic!("expected Lagged, got {other:?}"),
        };
        assert!(n > 0);
        let sync = sync_lost_event("s", "lagged", ev(10).seq);
        assert_eq!(sync.kind, SessionEventKind::SyncLost);
        assert_eq!(sync.data["reason"], "lagged");
        assert_eq!(sync.data["latest_seq"], 10);
    }

    /// Read one SSE `data:` frame from a response body stream and parse the
    /// [`SessionEvent`] it carries.
    async fn next_sse_event<E>(
        body: &mut (impl Stream<Item = Result<axum::body::Bytes, E>> + Unpin),
    ) -> SessionEvent {
        use futures::StreamExt as _;
        let frame = tokio::time::timeout(Duration::from_secs(5), body.next())
            .await
            .expect("SSE frame within timeout")
            .expect("stream still open")
            .ok()
            .expect("frame bytes ok");
        let text = String::from_utf8(frame.to_vec()).expect("utf8 frame");
        let line = text.lines().next().expect("non-empty SSE frame");
        let json = line.trim_start_matches("data:").trim_start();
        serde_json::from_str(json).expect("SessionEvent JSON")
    }

    /// Handler-level: a live subscriber that falls behind (broadcast `Lagged`)
    /// receives an out-of-band `sync_lost` on ITS connection only, and the
    /// stream stays open for subsequent live events (design §2.3).
    #[tokio::test]
    async fn sse_handler_sends_sync_lost_to_lagged_subscriber() {
        use axum::response::IntoResponse;

        let temp = tempfile::tempdir().expect("tempdir");
        let mut settings = crate::config::Settings::default();
        settings.storage.working_dir = temp.path().to_path_buf();
        let mut state = DaemonState::new(crate::state::AppState::new(settings)).await;
        // Isolate the registry from the developer's real projects.json.
        state.projects = crate::daemon::projects::ProjectRegistry::load(
            temp.path().to_path_buf(),
            temp.path().join("projects.json"),
        );
        // Tiny hub so a paused subscriber lags after a few publishes.
        state.session_event_hub = tokio::sync::broadcast::channel(2).0;
        let state = Arc::new(state);

        let sid = "s".to_string();
        state
            .session_manager
            .save(&crate::context::memory_session::Session::with_id(
                sid.clone(),
                None,
            ))
            .await
            .expect("save session");

        let sse = get_session_events(
            State(state.clone()),
            Path(sid.clone()),
            Query(SessionEventsQuery { after: None }),
        )
        .await
        .expect("handler must accept the subscription");
        let mut body = sse.into_response().into_body().into_data_stream();

        // Publish more events than the hub holds before the handler's spawned
        // task gets scheduled (current-thread runtime, no await in between),
        // so its receiver is guaranteed to lag.
        let sink = DaemonEventSink::new(
            sid.clone(),
            "r".into(),
            state.session_event_hub.clone(),
            state.session_seq_counter(&sid),
            state.session_buffer(&sid),
        );
        for _ in 0..10 {
            sink.publish(SessionEventKind::Save, serde_json::json!({}));
        }

        // First frame must be the out-of-band sync_lost for THIS connection.
        let lost = next_sse_event(&mut body).await;
        assert_eq!(lost.kind, SessionEventKind::SyncLost);
        assert_eq!(lost.seq, 0, "sync_lost is control-plane, not run output");
        assert_eq!(lost.session_id, "s");
        assert_eq!(lost.data["reason"], "lagged");
        assert_eq!(lost.data["latest_seq"], 10);

        // The stream stays open: the next live event (past the new seam) is
        // still delivered on the same connection.
        sink.publish(SessionEventKind::Save, serde_json::json!({}));
        let live = next_sse_event(&mut body).await;
        assert_eq!(live.kind, SessionEventKind::Save);
        assert_eq!(live.seq, 11);
    }

    #[tokio::test]
    async fn publish_writes_to_hub_and_buffer() {
        let (hub, mut rx) = tokio::sync::broadcast::channel(16);
        let buffer = Arc::new(std::sync::RwLock::new(SessionEventBuffer::new(8)));
        let sink = DaemonEventSink::new(
            "s1".into(),
            "r1".into(),
            hub,
            Arc::new(AtomicU64::new(1)),
            buffer.clone(),
        );
        sink.publish(SessionEventKind::Save, serde_json::json!({}));
        let got = rx.recv().await.expect("hub event");
        assert_eq!(got.seq, 1);
        let buf = buffer.read().expect("buffer lock poisoned");
        assert_eq!(buf.latest_seq(), Some(1));
    }

    #[test]
    fn maps_runtime_events_to_session_events() {
        let (hub, mut rx) = tokio::sync::broadcast::channel(16);
        let sink = DaemonEventSink::new(
            "s1".into(),
            "r1".into(),
            hub,
            Arc::new(AtomicU64::new(1)),
            Arc::new(std::sync::RwLock::new(SessionEventBuffer::new(16))),
        );
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
        let sink = DaemonEventSink::new(
            "s1".into(),
            "r1".into(),
            hub,
            Arc::new(AtomicU64::new(1)),
            Arc::new(std::sync::RwLock::new(SessionEventBuffer::new(16))),
        );
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

    #[test]
    fn tool_calls_round_end_publishes_boundary_then_terminal() {
        // StreamDone with finish_reason "tool_calls" ends an LLM ROUND, not the
        // turn - it now publishes TurnDone{tool_calls} as a round-boundary
        // signal so web/desktop clients can split per-round bubbles (their SSE
        // loop treats tool_calls as a boundary, not turn completion). The
        // terminal StreamDone (non-tool_calls) is still published exactly once
        // via `turn_done_published` (the loop re-emits it at turn completion).
        let (hub, mut rx) = tokio::sync::broadcast::channel(16);
        let sink = DaemonEventSink::new(
            "s1".into(),
            "r1".into(),
            hub,
            Arc::new(AtomicU64::new(1)),
            Arc::new(std::sync::RwLock::new(SessionEventBuffer::new(16))),
        );
        sink.emit(RuntimeEvent::StreamDone {
            finish_reason: "tool_calls".into(),
        });
        sink.emit(RuntimeEvent::ToolStart {
            name: "file_write".into(),
            args: serde_json::json!({}),
        });
        sink.emit(RuntimeEvent::StreamDone {
            finish_reason: "stop".into(),
        });
        // The loop re-emits StreamDone at turn completion — TurnDone must be
        // published exactly once.
        sink.emit(RuntimeEvent::StreamDone {
            finish_reason: "stop".into(),
        });
        // Round-boundary TurnDone{tool_calls} fires before ToolStart.
        assert!(rx.try_recv().is_ok_and(|e| {
            e.kind == SessionEventKind::TurnDone
                && e.data.get("finish_reason").and_then(|v| v.as_str()) == Some("tool_calls")
        }));
        assert!(rx
            .try_recv()
            .is_ok_and(|e| e.kind == SessionEventKind::ToolStart));
        // Terminal TurnDone{stop} exactly once.
        assert!(rx.try_recv().is_ok_and(|e| {
            e.kind == SessionEventKind::TurnDone
                && e.data.get("finish_reason").and_then(|v| v.as_str()) == Some("stop")
        }));
        assert!(rx.try_recv().is_err(), "no second terminal TurnDone");
    }

    #[test]
    fn seq_is_monotonic_per_session_across_runs() {
        // Two sinks sharing one session counter (two runs of the same
        // session): interleaved events keep a strictly increasing seq, so
        // clients can dedup/order by seq alone on reconnect. A different
        // session's counter starts at 1 independently.
        let (hub, mut rx) = tokio::sync::broadcast::channel(16);
        let counter = Arc::new(AtomicU64::new(1)); // DaemonState::session_seq_counter
                                                   // Runs of one session share the session's replay buffer (production
                                                   // gets it from DaemonState::session_buffer); other sessions get their
                                                   // own, mirroring the per-session counter.
        let s1_buffer = Arc::new(std::sync::RwLock::new(SessionEventBuffer::new(16)));
        let sink_run1 = DaemonEventSink::new(
            "s1".into(),
            "r1".into(),
            hub.clone(),
            Arc::clone(&counter),
            Arc::clone(&s1_buffer),
        );
        let sink_run2 = DaemonEventSink::new(
            "s1".into(),
            "r2".into(),
            hub.clone(),
            Arc::clone(&counter),
            Arc::clone(&s1_buffer),
        );
        let other_session = DaemonEventSink::new(
            "s2".into(),
            "r3".into(),
            hub,
            Arc::new(AtomicU64::new(1)),
            Arc::new(std::sync::RwLock::new(SessionEventBuffer::new(16))),
        );

        sink_run1.emit(RuntimeEvent::ContentDelta("a".into()));
        sink_run2.emit(RuntimeEvent::ContentDelta("b".into()));
        sink_run1.emit(RuntimeEvent::ContentDelta("c".into()));
        other_session.emit(RuntimeEvent::ContentDelta("x".into()));

        let events: Vec<SessionEvent> = (0..4).filter_map(|_| rx.try_recv().ok()).collect();
        let s1_seqs: Vec<u64> = events
            .iter()
            .filter(|e| e.session_id == "s1")
            .map(|e| e.seq)
            .collect();
        assert_eq!(s1_seqs, vec![1, 2, 3], "seq must be per-session monotonic");
        let s2_seqs: Vec<u64> = events
            .iter()
            .filter(|e| e.session_id == "s2")
            .map(|e| e.seq)
            .collect();
        assert_eq!(s2_seqs, vec![1], "other sessions count independently");
    }

    // ── RootToolPort ───────────────────────────────────────────────────────

    #[test]
    fn run_request_plan_mode_defaults_false_and_reads_true() {
        // Absent plan_mode (e.g. the web client, which has no plan toggle)
        // defaults to false via #[serde(default)].
        let no_field: RunRequest =
            serde_json::from_str(r#"{"message":"hi"}"#).expect("deserialize");
        assert_eq!(no_field.message, "hi");
        assert!(!no_field.plan_mode);
        // The TUI forwards plan_mode = true when in Plan mode.
        let with_field: RunRequest =
            serde_json::from_str(r#"{"message":"hi","plan_mode":true}"#).expect("deserialize");
        assert!(with_field.plan_mode);
    }

    #[test]
    fn run_request_queue_defaults_true_and_reads_false() {
        // Absent queue field defaults to true: a busy session parks the
        // message instead of answering 409.
        let no_field: RunRequest =
            serde_json::from_str(r#"{"message":"hi"}"#).expect("deserialize");
        assert!(no_field.queue);
        // An explicit false preserves the legacy immediate-409 contract.
        let legacy: RunRequest =
            serde_json::from_str(r#"{"message":"hi","queue":false}"#).expect("deserialize");
        assert!(!legacy.queue);
    }

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

    #[test]
    fn title_from_first_user_message_basic() {
        let seed = vec![
            ChatMessage::assistant("earlier"),
            ChatMessage::user("  fix the   login bug  "),
        ];
        assert_eq!(
            title_from_first_user_message(&seed).as_deref(),
            Some("fix the login bug")
        );
    }

    #[test]
    fn title_from_first_user_message_truncates() {
        let long = "x".repeat(60);
        let title = title_from_first_user_message(&[ChatMessage::user(long)]).unwrap();
        assert_eq!(title.chars().count(), 49); // 48 chars + ellipsis
        assert!(title.ends_with('…'));
    }

    #[test]
    fn title_from_first_user_message_none_cases() {
        assert_eq!(title_from_first_user_message(&[]), None);
        assert_eq!(
            title_from_first_user_message(&[ChatMessage::assistant("hi")]),
            None
        );
        assert_eq!(
            title_from_first_user_message(&[ChatMessage::user("   ")]),
            None
        );
    }

    #[tokio::test]
    async fn executes_in_bound_workdir() {
        let temp = tempfile::tempdir().expect("tempdir");
        let registry = Arc::new(ToolRegistry::with_project_root(temp.path(), 5));
        let rules = Arc::new(RwLock::new(HashSet::new()));
        let bridge = Arc::new(PermissionBridge::new(Duration::from_secs(5)));
        // Writes always Ask (see validate_write_paths); pre-approve the path
        // rule so this test exercises the workdir binding, not the approval
        // flow — otherwise execute() parks on the bridge forever.
        let rule_key = format!(
            "path:{}",
            temp.path()
                .canonicalize()
                .expect("canonical tempdir")
                .join("a.txt")
                .display()
        );
        rules.write().await.insert(rule_key);
        let port =
            RootToolPort::new_for_test(registry, rules, bridge, Some(temp.path().to_path_buf()));
        // Since 975e46ab the policy layer answers `Ask` for every file_write
        // (auto-approve is decided downstream by the root permission mode).
        // This test verifies workdir binding, not the approval flow, so give
        // the root an auto-approving mode — otherwise `resolve_ask` waits on
        // the bridge forever under the default Normal mode.
        port.permission_modes.set(
            temp.path().to_path_buf(),
            crate::config::RootPermissionMode::AcceptEdits,
            crate::sandbox::EffectiveMode::AcceptEdits,
        );

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

        assert!(matches!(
            bridge.resolve(&request_id, true).await,
            crate::teams::PermissionResolveOutcome::Resolved
        ));
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

    fn background_result(
        task_id: &str,
        session_id: &str,
    ) -> crate::tools::execution::background::BackgroundResult {
        crate::tools::execution::background::BackgroundResult {
            task_id: task_id.to_string(),
            session_id: Some(session_id.to_string()),
            result_type: "command".to_string(),
            command: format!("run {task_id}"),
            stdout: format!("output {task_id}"),
            stderr: String::new(),
            exit_code: Some(0),
            success: true,
            sandbox_bypassed: false,
            permission_mode: None,
            sandbox_level: None,
        }
    }

    async fn continuation_test_state() -> Arc<DaemonState> {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut settings = crate::config::Settings::default();
        settings.storage.working_dir = temp.keep();
        let state = Arc::new(DaemonState::new(crate::state::AppState::new(settings)).await);
        state
            .session_manager
            .save(&crate::context::memory_session::Session::with_id(
                "session-a".to_string(),
                None,
            ))
            .await
            .expect("save test session");
        state
    }

    /// Drive a session-a root task group to readiness: one child, finished.
    async fn make_ready_root_group(state: &Arc<DaemonState>) {
        let root = state.root_context("session-a").await.expect("root context");
        let group = state
            .coordinator
            .current_or_create_root_group(&root, "turn-1")
            .await
            .expect("root group");
        let child = state
            .coordinator
            .reserve_child_in_group(&root, crate::agent::SpawnChildRequest::new("work"), group)
            .await
            .expect("child reservation");
        state
            .coordinator
            .finish_child(
                &child.context,
                crate::agent::ChildTerminal::completed("subagent done"),
            )
            .await
            .expect("finish child");
    }

    #[tokio::test]
    async fn ready_task_group_yields_synthesis_continuation() {
        let state = continuation_test_state().await;
        make_ready_root_group(&state).await;

        let continuation = prepare_task_group_continuation(&state, "session-a")
            .await
            .expect("ready root group should produce a continuation");
        assert!(continuation.message.contains("<child-results>"));
        assert!(continuation.message.contains("subagent done"));
        // The continuation owns the session run.
        assert!(state.session_runs.is_active("session-a"));

        // The group was claimed exactly once: after the continuation's run
        // releases, a second attempt finds nothing ready and must release the
        // run claim it took while checking.
        state.session_runs.finish("session-a", &continuation.run_id);
        assert!(prepare_task_group_continuation(&state, "session-a")
            .await
            .is_none());
        assert!(!state.session_runs.is_active("session-a"));
    }

    #[tokio::test]
    async fn task_group_continuation_waits_for_an_idle_session() {
        let state = continuation_test_state().await;
        make_ready_root_group(&state).await;

        // A user-driven run owns the session: the group stays ready, no
        // continuation is prepared.
        state
            .session_runs
            .claim("session-a", test_run("user-run"))
            .expect("claim user run");
        assert!(prepare_task_group_continuation(&state, "session-a")
            .await
            .is_none());

        // Once the run finishes, the ready group is delivered.
        state.session_runs.finish("session-a", "user-run");
        let continuation = prepare_task_group_continuation(&state, "session-a")
            .await
            .expect("idle session with a ready group should continue");
        assert!(continuation.message.contains("subagent done"));
    }

    #[tokio::test]
    async fn task_group_continuation_respects_the_failed_save_barrier() {
        let state = continuation_test_state().await;
        make_ready_root_group(&state).await;
        state.block_background_continuation("session-a").await;
        assert!(prepare_task_group_continuation(&state, "session-a")
            .await
            .is_none());
        assert!(!state.session_runs.is_active("session-a"));
    }

    // ── Queued user messages ─────────────────────────────────────────────

    #[tokio::test]
    async fn queued_message_waits_for_an_idle_session() {
        let state = continuation_test_state().await;
        state
            .enqueue_message("session-a", "follow-up".to_string(), true)
            .await
            .expect("enqueue");

        // A user-driven run owns the session: the queued message stays put.
        state
            .session_runs
            .claim("session-a", test_run("user-run"))
            .expect("claim user run");
        assert!(prepare_message_continuation(&state, "session-a")
            .await
            .is_none());
        assert_eq!(state.message_queue_depth("session-a").await, 1);

        // Once the run finishes, the queued message starts as the next run
        // with its plan_mode preserved.
        state.session_runs.finish("session-a", "user-run");
        let continuation = prepare_message_continuation(&state, "session-a")
            .await
            .expect("idle session with a queued message should continue");
        assert_eq!(continuation.message, "follow-up");
        assert!(continuation.plan_mode);
        assert_eq!(
            state.session_runs.active_run_id("session-a").as_deref(),
            Some(continuation.run_id.as_str())
        );
        // The head is claimed (not dropped) until the run's start save acks.
        assert_eq!(state.message_queue_depth("session-a").await, 0);
    }

    #[tokio::test]
    async fn message_continuation_respects_the_failed_save_barrier() {
        let state = continuation_test_state().await;
        state
            .enqueue_message("session-a", "follow-up".to_string(), false)
            .await
            .expect("enqueue");
        state.block_background_continuation("session-a").await;
        assert!(prepare_message_continuation(&state, "session-a")
            .await
            .is_none());
        assert!(!state.session_runs.is_active("session-a"));
        assert_eq!(state.message_queue_depth("session-a").await, 1);
    }

    #[tokio::test]
    async fn message_continuation_claim_requeues_on_failed_start_save() {
        let state = continuation_test_state().await;
        state
            .enqueue_message("session-a", "follow-up".to_string(), false)
            .await
            .expect("enqueue");
        let continuation = prepare_message_continuation(&state, "session-a")
            .await
            .expect("continuation");

        // Simulate a failed start save: the claim returns to the queue head
        // and the run slot is free for the next attempt.
        assert!(
            state
                .requeue_message_claim("session-a", &continuation.run_id)
                .await
        );
        state.session_runs.finish("session-a", &continuation.run_id);
        assert_eq!(state.message_queue_depth("session-a").await, 1);

        let retry = prepare_message_continuation(&state, "session-a")
            .await
            .expect("requeued message continues again");
        assert_eq!(retry.message, "follow-up");

        // A stale run id must not ack or requeue another run's claim.
        assert!(!state.ack_message_claim("session-a", "stale-run").await);
        assert!(!state.requeue_message_claim("session-a", "stale-run").await);

        // After the start save succeeds the claim is acked for good.
        assert!(state.ack_message_claim("session-a", &retry.run_id).await);
        assert_eq!(state.message_queue_depth("session-a").await, 0);
    }

    #[tokio::test]
    async fn queued_messages_are_fifo_and_retractable() {
        let state = continuation_test_state().await;
        let first = state
            .enqueue_message("session-a", "one".to_string(), false)
            .await
            .expect("enqueue one");
        state
            .enqueue_message("session-a", "two".to_string(), false)
            .await
            .expect("enqueue two");

        let snapshot = state.queued_messages_snapshot("session-a").await;
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot[0].message, "one");
        assert_eq!(snapshot[1].message, "two");

        assert!(
            state
                .remove_queued_message("session-a", &first.message_id)
                .await
        );
        assert!(
            !state
                .remove_queued_message("session-a", &first.message_id)
                .await
        );
        let snapshot = state.queued_messages_snapshot("session-a").await;
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].message, "two");

        assert_eq!(state.clear_queued_messages("session-a").await, 1);
        assert!(!state.has_queued_messages("session-a").await);
    }

    #[tokio::test]
    async fn message_queue_enforces_max_depth() {
        let state = continuation_test_state().await;
        for i in 0..MESSAGE_QUEUE_MAX_DEPTH {
            state
                .enqueue_message("session-a", format!("msg {i}"), false)
                .await
                .expect("within capacity");
        }
        assert!(state
            .enqueue_message("session-a", "overflow".to_string(), false)
            .await
            .is_err());
        // A different session is unaffected.
        assert!(state
            .enqueue_message("session-b", "ok".to_string(), false)
            .await
            .is_ok());
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
    async fn idle_result_claims_batch_without_removing_it_before_ack() {
        let state = continuation_test_state().await;
        state
            .record_background_result(background_result("bg_1", "session-a"))
            .await;
        state
            .record_background_result(background_result("bg_2", "session-a"))
            .await;

        let continuation = prepare_background_continuation(
            &state,
            BackgroundSchedulerEvent::ResultReady {
                session_id: "session-a".to_string(),
            },
        )
        .await
        .expect("idle session should claim a continuation");

        assert!(state.session_runs.is_active("session-a"));
        assert_eq!(
            state
                .background_results_snapshot_for_session("session-a")
                .await
                .len(),
            2,
            "claimed results remain recoverable until the start save is acknowledged"
        );
        let payload: serde_json::Value =
            serde_json::from_str(&continuation.message).expect("structured continuation JSON");
        assert_eq!(payload["type"], "background_task_results");
        assert_eq!(payload["results"][0]["task_id"], "bg_1");
        assert_eq!(payload["results"][1]["task_id"], "bg_2");

        assert!(
            state
                .ack_background_result_claim("session-a", &continuation.run_id)
                .await
        );
        assert!(state
            .background_results_snapshot_for_session("session-a")
            .await
            .is_empty());
    }

    #[tokio::test]
    async fn acknowledged_task_id_rejects_late_duplicate_without_sse() {
        let state = continuation_test_state().await;
        let mut events = state.global_event_hub.subscribe();
        let result = background_result("bg_1", "session-a");
        assert!(state.record_background_result(result.clone()).await);
        events.recv().await.expect("first result SSE");
        let continuation = prepare_background_continuation(
            &state,
            BackgroundSchedulerEvent::ResultReady {
                session_id: "session-a".to_string(),
            },
        )
        .await
        .expect("claim continuation");
        assert!(
            state
                .ack_background_result_claim("session-a", &continuation.run_id)
                .await
        );

        assert!(!state.record_background_result(result).await);
        assert!(matches!(
            events.try_recv(),
            Err(tokio::sync::broadcast::error::TryRecvError::Empty)
        ));
        assert!(state
            .background_results_snapshot_for_session("session-a")
            .await
            .is_empty());
    }

    #[tokio::test]
    async fn unacknowledged_claim_requeues_without_handoff_after_startup_crash() {
        let state = continuation_test_state().await;
        state
            .record_background_result(background_result("bg_1", "session-a"))
            .await;
        let first = prepare_background_continuation(
            &state,
            BackgroundSchedulerEvent::ResultReady {
                session_id: "session-a".to_string(),
            },
        )
        .await
        .expect("first continuation claim");

        let retry = prepare_background_continuation(
            &state,
            BackgroundSchedulerEvent::RunFinished {
                session_id: "session-a".to_string(),
                run_id: first.run_id.clone(),
                final_save_succeeded: false,
            },
        )
        .await;

        assert!(retry.is_none(), "failed run must not hand off as success");
        assert!(!state.session_runs.is_active("session-a"));
        assert_eq!(
            state
                .background_results_snapshot_for_session("session-a")
                .await
                .len(),
            1,
            "failed run's unacknowledged claim remains recoverable"
        );
    }

    #[tokio::test]
    async fn busy_result_waits_until_run_finished_before_handoff() {
        let state = continuation_test_state().await;
        state
            .session_runs
            .claim("session-a", test_run("foreground"))
            .expect("foreground claim");
        state
            .record_background_result(background_result("bg_1", "session-a"))
            .await;

        let while_busy = prepare_background_continuation(
            &state,
            BackgroundSchedulerEvent::ResultReady {
                session_id: "session-a".to_string(),
            },
        )
        .await;
        assert!(
            while_busy.is_none(),
            "busy session must leave inbox pending"
        );
        assert_eq!(
            state
                .background_results_snapshot_for_session("session-a")
                .await
                .len(),
            1
        );

        let continuation = prepare_background_continuation(
            &state,
            BackgroundSchedulerEvent::RunFinished {
                session_id: "session-a".to_string(),
                run_id: "foreground".to_string(),
                final_save_succeeded: true,
            },
        )
        .await
        .expect("finishing owner should hand off to pending continuation");
        assert_ne!(continuation.run_id, "foreground");
        assert_eq!(
            state.session_runs.active_run_id("session-a").as_deref(),
            Some(continuation.run_id.as_str())
        );
    }

    #[tokio::test]
    async fn failed_final_save_releases_without_handoff_and_preserves_pending_results() {
        let state = continuation_test_state().await;
        state
            .session_runs
            .claim("session-a", test_run("foreground"))
            .expect("foreground claim");
        state
            .record_background_result(background_result("bg_1", "session-a"))
            .await;

        let continuation = prepare_background_continuation(
            &state,
            BackgroundSchedulerEvent::RunFinished {
                session_id: "session-a".to_string(),
                run_id: "foreground".to_string(),
                final_save_succeeded: false,
            },
        )
        .await;

        assert!(
            continuation.is_none(),
            "failed final persistence must not hand off to a continuation"
        );
        assert!(!state.session_runs.is_active("session-a"));
        assert_eq!(
            state
                .background_results_snapshot_for_session("session-a")
                .await
                .iter()
                .map(|result| result.task_id.as_str())
                .collect::<Vec<_>>(),
            vec!["bg_1"]
        );

        state
            .record_background_result(background_result("bg_2", "session-a"))
            .await;
        let while_blocked = prepare_background_continuation(
            &state,
            BackgroundSchedulerEvent::ResultReady {
                session_id: "session-a".to_string(),
            },
        )
        .await;
        assert!(
            while_blocked.is_none(),
            "new result-ready events must not bypass a failed-save retry barrier"
        );

        state
            .session_runs
            .claim("session-a", test_run("foreground-retry"))
            .expect("explicit user retry can claim the released session");
        let continuation = prepare_background_continuation(
            &state,
            BackgroundSchedulerEvent::RunFinished {
                session_id: "session-a".to_string(),
                run_id: "foreground-retry".to_string(),
                final_save_succeeded: true,
            },
        )
        .await
        .expect("successful retry clears the barrier and hands off pending results");
        let payload: serde_json::Value =
            serde_json::from_str(&continuation.message).expect("continuation JSON");
        assert_eq!(payload["results"][0]["task_id"], "bg_1");
        assert_eq!(payload["results"][1]["task_id"], "bg_2");
    }

    #[tokio::test]
    async fn cancelled_run_hands_off_with_a_fresh_cancellation_token() {
        let state = continuation_test_state().await;
        let foreground = test_run("foreground");
        let foreground_cancel = foreground.cancel.clone();
        state
            .session_runs
            .claim("session-a", foreground)
            .expect("foreground claim");
        state
            .record_background_result(background_result("bg_1", "session-a"))
            .await;

        assert!(state.session_runs.cancel("session-a"));
        assert!(foreground_cancel.is_cancelled());
        let continuation = prepare_background_continuation(
            &state,
            BackgroundSchedulerEvent::RunFinished {
                session_id: "session-a".to_string(),
                run_id: "foreground".to_string(),
                final_save_succeeded: true,
            },
        )
        .await
        .expect("cancelled run should schedule retained results after completion");

        assert!(!continuation.cancel.is_cancelled());
        assert_eq!(
            state.session_runs.active_run_id("session-a").as_deref(),
            Some(continuation.run_id.as_str())
        );
    }

    #[tokio::test]
    async fn failed_guarded_save_is_propagated_to_run_finished() {
        let registry = RunRegistry::new();
        registry
            .claim("session-a", test_run("foreground"))
            .expect("foreground claim");
        let (completion_tx, mut completion_rx) = mpsc::unbounded_channel();
        let temp = tempfile::tempdir().expect("tempdir");
        let sessions = MemorySessionManager::with_project_root(temp.path().to_path_buf());
        let save_gen = AtomicU64::new(1);
        let save_lock = tokio::sync::Mutex::new(());
        let guard = RunClaimGuard {
            registry,
            session_id: "session-a".to_string(),
            run_id: "foreground".to_string(),
            completion_tx: Some(completion_tx),
            final_save_succeeded: false,
        };

        run_under_claim(
            guard,
            guarded_save(
                &sessions,
                "missing-session",
                &[ChatMessage::user("durable")],
                1,
                &save_gen,
                &save_lock,
            ),
        )
        .await;

        let event = completion_rx.recv().await.expect("run-finished event");
        assert!(matches!(
            event,
            BackgroundSchedulerEvent::RunFinished {
                run_id,
                final_save_succeeded: false,
                ..
            } if run_id == "foreground"
        ));
    }

    #[tokio::test]
    async fn guarded_save_reports_missing_session_as_failure() {
        let temp = tempfile::tempdir().expect("tempdir");
        let sessions = MemorySessionManager::with_project_root(temp.path().to_path_buf());
        let save_gen = AtomicU64::new(1);
        let save_lock = tokio::sync::Mutex::new(());

        let saved = guarded_save(
            &sessions,
            "missing-session",
            &[ChatMessage::user("durable")],
            1,
            &save_gen,
            &save_lock,
        )
        .await;

        assert!(!saved);
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
                completion_tx: None,
                final_save_succeeded: false,
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

    #[tokio::test]
    async fn guarded_save_skips_stale_snapshot() {
        // Regression: a spawned save holding an older snapshot must never
        // overwrite a newer save (e.g. the turn-end final save). Two
        // generations are claimed stale-then-new; the stale task reaches the
        // write first and must self-skip, leaving the newer snapshot to win.
        let temp = tempfile::tempdir().expect("tempdir");
        let sessions = MemorySessionManager::with_project_root(temp.path().to_path_buf());
        let session = sessions.create(Some("gen")).await.expect("create");
        let save_gen = Arc::new(AtomicU64::new(0));
        let save_lock = Arc::new(Mutex::new(()));

        let stale_gen = claim_save_generation(&save_gen);
        let new_gen = claim_save_generation(&save_gen);

        // Stale writer arrives first: self-skips because a newer generation
        // was claimed meanwhile.
        guarded_save(
            &sessions,
            &session.id,
            &[ChatMessage::user("stale")],
            stale_gen,
            &save_gen,
            &save_lock,
        )
        .await;
        assert!(
            sessions
                .get(&session.id)
                .await
                .expect("session")
                .messages
                .is_empty(),
            "stale snapshot must not be persisted"
        );

        // The newer snapshot is still the latest claim: it persists.
        guarded_save(
            &sessions,
            &session.id,
            &[ChatMessage::user("new")],
            new_gen,
            &save_gen,
            &save_lock,
        )
        .await;
        let persisted = sessions.get(&session.id).await.expect("session persists");
        assert_eq!(persisted.messages.len(), 1);
        assert_eq!(persisted.messages[0].content, "new");

        // A late stale retry (older snapshot finishing after the newer write,
        // the interleaving from the review finding) still cannot overwrite.
        guarded_save(
            &sessions,
            &session.id,
            &[ChatMessage::user("stale")],
            stale_gen,
            &save_gen,
            &save_lock,
        )
        .await;
        let persisted = sessions.get(&session.id).await.expect("session persists");
        assert_eq!(persisted.messages.len(), 1);
        assert_eq!(persisted.messages[0].content, "new");
    }
}
