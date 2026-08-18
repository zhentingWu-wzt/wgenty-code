//! DaemonState -- shared state for the HTTP API server.

use crate::config::RootPermissionMode;
use crate::context::memory_session::SessionManager as MemorySessionManager;
use crate::knowledge::loader::SkillLoader;
use crate::permissions::PermissionModeStore;
use crate::permissions::ToolPermissionPolicy;
use crate::runtime::hooks::HookManager;
use crate::state::AppState;
use crate::tasks::{TaskManagementTool, TodoState};
use crate::teams::mailbox::TeamManager;
use crate::teams::permission_bridge::PermissionBridge;
use crate::tools::execution::background::{BackgroundManager, BackgroundResult, BackgroundTool};
use crate::tools::meta::team_message::TeamMessageTool;
use crate::tools::{CheckpointManager, CheckpointStore, ToolExecutor, ToolRegistry};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

/// Idle timeout (seconds): the daemon initiates a graceful shutdown when no
/// thin client is connected AND no authenticated API request has arrived for
/// this long. Counts from daemon start, so a daemon that never serves any
/// client also exits instead of lingering forever.
pub const THIN_CLIENT_IDLE_TIMEOUT_SECS: u64 = 300;

/// Grace window for a client-bound daemon (spawned with `--spawned-by`):
/// once the LAST client (WS push or heartbeat SSE) disconnects, the daemon
/// exits after this window. Short enough to follow its owner down promptly,
/// long enough to absorb page refreshes and reconnect jitter.
pub const CLIENT_BOUND_GRACE_SECS: u64 = 30;

/// Tracks active thin-client connections and the last API activity time;
/// signals the daemon to exit when it has been idle for
/// [`THIN_CLIENT_IDLE_TIMEOUT_SECS`].
pub struct ActiveClientTracker {
    count: AtomicUsize,
    /// Unix epoch milliseconds of the last observed activity (client
    /// connect/disconnect or any authenticated API request). Seeded with the
    /// tracker's creation time so the idle window starts at daemon boot.
    last_activity_ms: AtomicU64,
    zero_notify: tokio::sync::Notify,
    shutting_down: AtomicUsize,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

impl ActiveClientTracker {
    pub fn new() -> Self {
        Self {
            count: AtomicUsize::new(0),
            last_activity_ms: AtomicU64::new(now_ms()),
            zero_notify: tokio::sync::Notify::new(),
            shutting_down: AtomicUsize::new(0),
        }
    }

    /// Record activity: any authenticated request or client connect/disconnect
    /// pushes the idle shutdown deadline out.
    pub fn touch(&self) {
        self.last_activity_ms.store(now_ms(), Ordering::Release);
    }

    /// Duration since the last observed activity.
    pub fn idle_for(&self) -> std::time::Duration {
        std::time::Duration::from_millis(
            now_ms().saturating_sub(self.last_activity_ms.load(Ordering::Acquire)),
        )
    }

    pub fn register_client(&self) -> bool {
        if self.shutting_down.load(Ordering::Acquire) != 0 {
            return false;
        }
        self.touch();
        self.count.fetch_add(1, Ordering::Release);
        true
    }

    pub fn unregister_client(&self) {
        self.touch();
        let prev = self.count.fetch_sub(1, Ordering::Release);
        if prev == 1 {
            self.zero_notify.notify_one();
        }
    }

    pub fn client_count(&self) -> usize {
        self.count.load(Ordering::Acquire)
    }

    /// Resolves when the client count drops to zero or shutdown is initiated.
    /// Lets the idle-shutdown monitor re-check promptly instead of waiting
    /// out its current sleep.
    pub async fn clients_changed(&self) {
        self.zero_notify.notified().await;
    }

    pub fn initiate_shutdown(&self) {
        self.shutting_down.store(1, Ordering::Release);
        self.zero_notify.notify_one();
    }
}

impl Default for ActiveClientTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Maximum retained background results per session.
///
/// Retention accepts eviction at extreme volume. Task IDs remain deduplicated
/// within their originating session.
pub const BACKGROUND_RESULTS_CAPACITY: usize = 256;

/// Maximum live entries for one session during recovery. A normal pending
/// queue and one claimed batch are each capped at
/// [`BACKGROUND_RESULTS_CAPACITY`]; requeue may temporarily combine them.
pub const BACKGROUND_RESULTS_MAX_LIVE_PER_SESSION: usize = BACKGROUND_RESULTS_CAPACITY * 2;

/// Number of recently acknowledged task IDs retained independently for each
/// session. Once a session's oldest tombstone is evicted, an exceptionally old
/// duplicate for that session may be accepted again; traffic from other
/// sessions cannot shorten its replay window.
pub const BACKGROUND_RESULT_TOMBSTONE_CAPACITY: usize = 1024;

struct BackgroundResultClaim {
    run_id: String,
    task_ids: Vec<String>,
}

#[derive(Default)]
struct SessionBackgroundResults {
    by_task_id: HashMap<String, BackgroundResult>,
    /// All live results, pending and claimed, in completion order.
    task_order: VecDeque<String>,
    /// Ordinary results eligible for the next continuation claim. A claimed
    /// batch is removed from this queue but remains in
    /// `by_task_id` / `task_order`.
    pending_order: VecDeque<String>,
    /// An unacknowledged batch restored after a failed run. Claims always take
    /// this queue before ordinary pending entries, and capacity eviction never
    /// removes it.
    recovery_order: VecDeque<String>,
    /// At most one continuation can own a batch because `RunRegistry` permits
    /// only one active run per session.
    claim: Option<BackgroundResultClaim>,
    consumed: HashSet<String>,
    consumed_order: VecDeque<String>,
}

/// Daemon-owned background-result lifecycle, keyed by session and task ID.
///
/// Entries move from `pending_order` to one claimed batch without leaving the
/// live maps, so snapshots can recover them until the continuation's start
/// message is persisted. A successful ack removes the live entries and records
/// bounded composite-key tombstones; an unacknowledged run requeues its claim.
#[derive(Default)]
struct BackgroundResultInbox {
    by_session: HashMap<String, SessionBackgroundResults>,
    arrival_order: VecDeque<(String, String)>,
    /// Sessions whose last run failed final persistence. Automatic
    /// continuations remain blocked until a later explicit run reports a
    /// successful final save.
    continuation_blocked: HashSet<String>,
}

impl BackgroundResultInbox {
    /// Returns `true` only when a new, owned task result was retained.
    fn enqueue(&mut self, result: BackgroundResult) -> bool {
        let Some(session_id) = result
            .session_id
            .as_deref()
            .filter(|session_id| !session_id.is_empty())
            .map(str::to_owned)
        else {
            return false;
        };
        let task_id = result.task_id.clone();
        let session = self.by_session.entry(session_id.clone()).or_default();
        if session.consumed.contains(&task_id) || session.by_task_id.contains_key(&task_id) {
            return false;
        }

        if session.pending_order.len() >= BACKGROUND_RESULTS_CAPACITY {
            if let Some(evicted_task_id) = session.pending_order.pop_front() {
                session.by_task_id.remove(&evicted_task_id);
                session
                    .task_order
                    .retain(|retained_task_id| retained_task_id != &evicted_task_id);
                self.arrival_order
                    .retain(|(retained_session_id, retained_task_id)| {
                        retained_session_id != &session_id || retained_task_id != &evicted_task_id
                    });
            }
        }

        session.task_order.push_back(task_id.clone());
        session.pending_order.push_back(task_id.clone());
        session.by_task_id.insert(task_id.clone(), result);
        self.arrival_order.push_back((session_id, task_id));
        true
    }

    fn snapshot_for_session(&self, session_id: &str) -> Vec<BackgroundResult> {
        let Some(session) = self.by_session.get(session_id) else {
            return Vec::new();
        };
        session
            .task_order
            .iter()
            .filter_map(|task_id| session.by_task_id.get(task_id).cloned())
            .collect()
    }

    fn snapshot_all(&self) -> Vec<BackgroundResult> {
        self.arrival_order
            .iter()
            .filter_map(|(session_id, task_id)| {
                self.by_session
                    .get(session_id)
                    .and_then(|session| session.by_task_id.get(task_id))
                    .cloned()
            })
            .collect()
    }

    fn has_pending_for_session(&self, session_id: &str) -> bool {
        self.by_session.get(session_id).is_some_and(|session| {
            !session.recovery_order.is_empty() || !session.pending_order.is_empty()
        })
    }

    fn continuation_is_blocked(&self, session_id: &str) -> bool {
        self.continuation_blocked.contains(session_id)
    }

    fn block_continuation(&mut self, session_id: &str) {
        self.continuation_blocked.insert(session_id.to_string());
    }

    fn allow_continuation(&mut self, session_id: &str) {
        self.continuation_blocked.remove(session_id);
    }

    fn claim_for_session(&mut self, session_id: &str, run_id: &str) -> Vec<BackgroundResult> {
        let Some(session) = self.by_session.get_mut(session_id) else {
            return Vec::new();
        };
        if session.claim.is_some()
            || (session.recovery_order.is_empty() && session.pending_order.is_empty())
        {
            return Vec::new();
        }
        let recovery_len = session
            .recovery_order
            .len()
            .min(BACKGROUND_RESULTS_CAPACITY);
        let mut task_ids: Vec<String> = session.recovery_order.drain(..recovery_len).collect();
        let remaining = BACKGROUND_RESULTS_CAPACITY - task_ids.len();
        task_ids.extend(
            session
                .pending_order
                .drain(..session.pending_order.len().min(remaining)),
        );
        let results = task_ids
            .iter()
            .filter_map(|task_id| session.by_task_id.get(task_id).cloned())
            .collect();
        session.claim = Some(BackgroundResultClaim {
            run_id: run_id.to_string(),
            task_ids,
        });
        results
    }

    fn ack_claim(&mut self, session_id: &str, run_id: &str) -> bool {
        let task_ids = {
            let Some(session) = self.by_session.get_mut(session_id) else {
                return false;
            };
            let Some(claim) = session.claim.take() else {
                return false;
            };
            if claim.run_id != run_id {
                session.claim = Some(claim);
                return false;
            }
            for task_id in &claim.task_ids {
                session.by_task_id.remove(task_id);
                session
                    .task_order
                    .retain(|retained_task_id| retained_task_id != task_id);
            }
            for task_id in &claim.task_ids {
                if session.consumed.insert(task_id.clone()) {
                    session.consumed_order.push_back(task_id.clone());
                }
            }
            while session.consumed_order.len() > BACKGROUND_RESULT_TOMBSTONE_CAPACITY {
                if let Some(expired) = session.consumed_order.pop_front() {
                    session.consumed.remove(&expired);
                }
            }
            claim.task_ids
        };

        self.arrival_order
            .retain(|(retained_session_id, retained_task_id)| {
                retained_session_id != session_id || !task_ids.contains(retained_task_id)
            });
        true
    }

    fn requeue_claim(&mut self, session_id: &str, run_id: &str) -> bool {
        let Some(session) = self.by_session.get_mut(session_id) else {
            return false;
        };
        let Some(claim) = session.claim.take() else {
            return false;
        };
        if claim.run_id != run_id {
            session.claim = Some(claim);
            return false;
        }
        let mut recovery = VecDeque::from(claim.task_ids);
        recovery.append(&mut session.recovery_order);
        session.recovery_order = recovery;
        true
    }
}

/// Maximum queued user messages per session. `POST /sessions/:id/run` answers
/// `429` beyond this depth so a runaway client cannot balloon memory; the
/// queue itself is intentionally short (typing ahead a few turns).
pub const MESSAGE_QUEUE_MAX_DEPTH: usize = 8;

/// One user message parked while the session's single run slot is busy.
#[derive(Debug, Clone, serde::Serialize, PartialEq)]
pub struct QueuedMessage {
    pub message_id: String,
    pub message: String,
    /// Preserved from the submitting request so a queued Plan-mode turn still
    /// runs as Plan mode when the scheduler finally starts it.
    pub plan_mode: bool,
}

#[derive(Default)]
struct SessionMessageQueue {
    /// FIFO of not-yet-started messages.
    pending: VecDeque<QueuedMessage>,
    /// The single head claimed by the active continuation run. At most one
    /// because `RunRegistry` permits one run per session. Retained (not
    /// removed) until the run's start save succeeds, so a crashed start can
    /// requeue it — mirrors `BackgroundResultClaim`.
    claim: Option<(String, QueuedMessage)>,
}

/// Per-session FIFO of user messages submitted while a run was active.
///
/// Lifecycle mirrors [`BackgroundResultInbox`]: a claimed head stays visible
/// until the continuation run persists its start save (`ack_claim`); a failed
/// start requeues it ahead of the rest (`requeue_claim`). Unlike background
/// results there is no dedup/tombstone layer — messages are unique by id.
#[derive(Default)]
struct MessageInbox {
    by_session: HashMap<String, SessionMessageQueue>,
}

/// Error returned by [`MessageInbox::enqueue`] when the session's queue is at
/// [`MESSAGE_QUEUE_MAX_DEPTH`].
#[derive(Debug)]
pub struct MessageQueueFull;

impl MessageInbox {
    fn enqueue(
        &mut self,
        session_id: &str,
        message: String,
        plan_mode: bool,
    ) -> Result<QueuedMessage, MessageQueueFull> {
        let session = self.by_session.entry(session_id.to_string()).or_default();
        if session.pending.len() >= MESSAGE_QUEUE_MAX_DEPTH {
            return Err(MessageQueueFull);
        }
        let queued = QueuedMessage {
            message_id: uuid::Uuid::new_v4().to_string(),
            message,
            plan_mode,
        };
        session.pending.push_back(queued.clone());
        Ok(queued)
    }

    fn snapshot_for_session(&self, session_id: &str) -> Vec<QueuedMessage> {
        self.by_session
            .get(session_id)
            .map(|session| session.pending.iter().cloned().collect())
            .unwrap_or_default()
    }

    fn depth(&self, session_id: &str) -> usize {
        self.by_session
            .get(session_id)
            .map_or(0, |session| session.pending.len())
    }

    fn has_pending(&self, session_id: &str) -> bool {
        self.depth(session_id) > 0
    }

    /// Move the head message into an exclusive claim by `run_id`.
    fn claim_head(&mut self, session_id: &str, run_id: &str) -> Option<QueuedMessage> {
        let session = self.by_session.get_mut(session_id)?;
        if session.claim.is_some() {
            return None;
        }
        let head = session.pending.pop_front()?;
        session.claim = Some((run_id.to_string(), head.clone()));
        Some(head)
    }

    /// Drop the claim after the run's start save succeeded. The message is
    /// now durable session history.
    fn ack_claim(&mut self, session_id: &str, run_id: &str) -> bool {
        let Some(session) = self.by_session.get_mut(session_id) else {
            return false;
        };
        match session.claim.take() {
            Some((claim_run_id, _)) if claim_run_id == run_id => true,
            other => {
                session.claim = other;
                false
            }
        }
    }

    /// Restore a failed run's claim to the queue head.
    fn requeue_claim(&mut self, session_id: &str, run_id: &str) -> bool {
        let Some(session) = self.by_session.get_mut(session_id) else {
            return false;
        };
        match session.claim.take() {
            Some((claim_run_id, message)) if claim_run_id == run_id => {
                session.pending.push_front(message);
                true
            }
            other => {
                session.claim = other;
                false
            }
        }
    }

    /// Remove one still-pending message (user retracting a queued turn).
    /// Claimed heads cannot be removed — their run may already be saving.
    fn remove(&mut self, session_id: &str, message_id: &str) -> bool {
        let Some(session) = self.by_session.get_mut(session_id) else {
            return false;
        };
        let before = session.pending.len();
        session.pending.retain(|m| m.message_id != message_id);
        before != session.pending.len()
    }

    /// Drop every pending message for the session. Returns how many.
    fn clear(&mut self, session_id: &str) -> usize {
        self.by_session
            .get_mut(session_id)
            .map(|session| {
                let n = session.pending.len();
                session.pending.clear();
                n
            })
            .unwrap_or(0)
    }
}

/// Per-session permission rules.
struct SessionRules {
    approved: HashSet<String>,
}

/// A project's checkpoint handle pair (manager + store), cached per project
/// root in [`DaemonState::project_checkpoints`].
type ProjectCheckpointHandles = (Arc<CheckpointManager>, Arc<CheckpointStore>);

impl SessionRules {
    fn new() -> Self {
        Self {
            approved: HashSet::new(),
        }
    }
}

/// Shared state for all daemon HTTP handlers.
pub struct DaemonState {
    pub app_state: AppState,
    /// Live, runtime-mutable settings handle. Seeded from `app_state.settings`
    /// at startup, then updated in place by the `/model` switch endpoint.
    /// Per-request handlers (e.g. `chat_stream`) read a fresh clone from here
    /// so a model switch takes effect on the next turn without a daemon
    /// restart. The startup-only derived resources below (tool registry,
    /// checkpoint store, etc.) keep reading the frozen `app_state.settings`
    /// snapshot since they don't depend on the active model identity.
    pub settings_handle: crate::config::watcher::SettingsHandle,
    /// Shared pooled HTTP clients for LLM API calls. Built once at startup
    /// from `app_state.settings` so the reqwest keep-alive pool + TLS session
    /// cache are reused across every per-request `ApiClient` - avoids a fresh
    /// TCP + TLS handshake on each chat turn. Model identity (name/base_url/
    /// api_key/provider) is resolved per request from [`Self::settings_handle`],
    /// so only the connect/read timeouts baked in here are fixed for the
    /// daemon lifetime.
    pub http_client: Arc<reqwest::Client>,
    pub http_client_stream: Arc<reqwest::Client>,
    pub tool_registry: Arc<ToolRegistry>,
    pub tool_executor: ToolExecutor,
    pub checkpoint_manager: Arc<CheckpointManager>,
    pub checkpoint_store: Arc<CheckpointStore>,
    /// Per-session static Work-Graph runtimes shared by daemon tool calls.
    pub work_graph_runtime_store: Arc<crate::exec_session::ExecutionSessionRuntimeStore>,
    pub task_manager: Arc<TaskManagementTool>,
    pub todo_state: Arc<RwLock<TodoState>>,
    pub task_router: Arc<crate::tasks::TaskRouter>,
    pub todo_router: Arc<crate::tasks::TodoRouter>,
    pub skill_loader: Arc<SkillLoader>,
    pub background_manager: Arc<BackgroundManager>,
    /// Session-scoped, task-ID-deduplicated background result inbox. Fed by
    /// the tool-layer manager hook and retained before any SSE notification.
    background_result_inbox: Arc<RwLock<BackgroundResultInbox>>,
    /// Session-scoped FIFO of user messages submitted while the single run
    /// slot was busy. Drained by the continuation scheduler after a run
    /// finishes (and after pending background results / ready task groups).
    message_inbox: Arc<RwLock<MessageInbox>>,
    /// Scheduler mailbox. Result-ready and run-finished notifications share
    /// one consumer so their relative ordering cannot strand a busy session's
    /// pending inbox entries.
    background_scheduler_tx:
        tokio::sync::mpsc::UnboundedSender<crate::daemon::run_loop::BackgroundSchedulerEvent>,
    background_scheduler_rx: std::sync::Mutex<
        Option<
            tokio::sync::mpsc::UnboundedReceiver<crate::daemon::run_loop::BackgroundSchedulerEvent>,
        >,
    >,
    background_scheduler_started: std::sync::atomic::AtomicBool,
    pub team_manager: Option<Arc<TeamManager>>,
    pub session_manager: MemorySessionManager,
    /// Shared MemoryManager backing the `memory_add` tool and AutoDream (D1).
    pub memory_manager: Arc<crate::context::MemoryManager>,
    /// Hook manager for lifecycle events (PreToolUse/PostToolUse/UserPromptSubmit).
    /// Used by run_session_turn to fire UserPromptSubmit hooks and collect
    /// injection fragments for the prompt reminder + inspector TurnContext.
    pub hook_manager: Arc<HookManager>,
    /// Long-lived external MCP sessions and their status.
    pub mcp_manager: Arc<crate::mcp::McpManager>,
    sessions: Arc<RwLock<std::collections::HashMap<String, SessionRules>>>,
    /// Subagent progress store, scoped by session_id → node_id.
    pub subagent_progress:
        Arc<RwLock<HashMap<String, HashMap<String, crate::agent::progress::SubagentProgress>>>>,
    /// Last poll timestamp per session, used for TTL-based eviction.
    pub subagent_poll_times: Arc<RwLock<HashMap<String, Instant>>>,
    /// Exclusive owner of agent spawning, concurrency, and lifecycle. Scoped
    /// agent APIs read through it; identity is never taken from request JSON.
    pub coordinator: Arc<crate::agent::AgentCoordinator>,
    /// Viewer-bound capability service for trusted UI navigation.
    pub capability_service: Arc<crate::agent::capability::CapabilityService>,
    /// Viewer bearer-token digests: HMAC-SHA256(daemon_viewer_secret, token)
    /// -> ViewerId. The raw token is never stored.
    viewer_tokens: Arc<RwLock<HashMap<String, crate::agent::capability::ViewerId>>>,
    /// Root execution context per session, created via `ensure_root`.
    root_contexts: Arc<RwLock<HashMap<String, crate::agent::AgentExecutionContext>>>,
    /// Session → bound worktree path (project v1: project → worktree → session,
    /// N:1). Sessions without an entry run in the main working_dir.
    pub session_workdirs: SessionWorkdirs,
    /// Multi-project registry. The daemon `working_dir` is the implicit main
    /// project; registered projects are arbitrary directories (git optional)
    /// persisted at `~/.wgenty-code/projects.json`.
    pub projects: crate::daemon::projects::ProjectRegistry,
    /// Per-project memory routing (`memory_add` tool, memory HTTP handlers,
    /// AutoDream fan-out). `memory_manager` above remains the main project's
    /// manager.
    pub memory_router: Arc<crate::daemon::memory_router::MemoryRouter>,
    /// Per-project session managers (lazy). The main project's manager is
    /// `session_manager`; each registered project gets one cached instance.
    /// tokio RwLock: get-or-create runs `load_index` (async I/O).
    project_session_managers: Arc<RwLock<HashMap<PathBuf, MemorySessionManager>>>,
    /// Per-project checkpoint handles (lazy). The main project uses the tool
    /// registry's `checkpoint_manager`/`checkpoint_store`; each registered
    /// project gets a store under `<project>/.wgenty-code/checkpoints/`.
    /// std RwLock: construction is pure path joins (no I/O), never held
    /// across `.await`.
    project_checkpoints: Arc<std::sync::RwLock<HashMap<PathBuf, ProjectCheckpointHandles>>>,
    /// Secret used to digest viewer bearer tokens.
    daemon_viewer_secret: [u8; 32],
    /// Shared subagent policy-Ask bridge (TUI/daemon drains pending approvals).
    pub permission_bridge: Arc<PermissionBridge>,
    /// Shared ask_user_question bridge (server-side loop blocks until a client
    /// resolves the prompt via POST /interactions/:id/resolve).
    pub interaction_bridge: Arc<crate::daemon::interaction_bridge::InteractionBridge>,
    /// Per-project permission modes (root + effective). Each project (by
    /// canonical working dir) owns an independent entry; defaults to Normal.
    pub permission_modes: PermissionModeStore,
    /// Shared sandbox effective mode lock (includes Plan). Read by
    /// `root_context` when building the trusted `ToolContext`.
    pub effective_mode: Arc<std::sync::RwLock<crate::sandbox::EffectiveMode>>,
    /// Shared read connection to the global subagent transcript store, used by
    /// the SSE trace endpoint for cold-start replay. `None` when the store
    /// failed to open at startup (SSE then streams live-only). See design D5.
    pub transcript_store: Option<Arc<crate::transcript::SubagentTranscriptStore>>,
    /// Broadcast hub for daemon-run session events (`SessionEvent` envelope).
    /// One hub per daemon; events carry session_id/run_id for filtering.
    pub session_event_hub: crate::daemon::run_loop::SessionEventHub,
    /// Current bearer token as seen by in-handler auth (the WebSocket push
    /// endpoint authenticates from a query param because browser WebSocket
    /// APIs cannot set headers; design §3.1). Empty until `set_api_token`
    /// runs at startup — the ws handler rejects ALL credentials while empty
    /// so the uninitialized window cannot be bypassed with an empty token.
    /// RwLock so a future in-process rotation can close live ws connections
    /// with code 4001 (pump tick guard).
    api_token: std::sync::RwLock<String>,
    /// Port the HTTP listener actually bound to (design §2). Set once after
    /// `TcpListener::bind` succeeds in `run()`; read by the bootstrap
    /// endpoint's same-origin predicate to build the allowed host:port set
    /// (`127.0.0.1:<port>` / `localhost:<port>`). `None` until set — the
    /// bootstrap endpoint fail-closes while unset. Same RwLock shape as
    /// `api_token` for consistency.
    /// `(port, lan_exposed)` of the bound listener (`None` until
    /// [`DaemonState::set_bind`]). `lan_exposed` marks a non-loopback bind
    /// (`--host 0.0.0.0`), which widens the web-UI bootstrap same-origin
    /// allowlist to private-IP hosts.
    bind: std::sync::RwLock<Option<(u16, bool)>>,
    /// Broadcast hub for daemon-wide (cross-project) global events
    /// (`GlobalEvent` envelope). Independent from `session_event_hub` so
    /// high-frequency session deltas can't starve global events (design §3.1).
    pub global_event_hub: crate::daemon::global_events::GlobalEventHub,
    /// Global event sequence counter, monotonic across the daemon process
    /// (starts at 1 on each start; not resumable after a restart). Kept
    /// separate from `session_seq_counters` — global events live in their own
    /// seq space.
    pub global_seq_counter: Arc<AtomicU64>,
    /// One active server-side run per session (claim registry). Enforces the
    /// 409 on `POST /sessions/:id/run` and the update_session run lock.
    pub session_runs: crate::daemon::run_loop::RunRegistry,
    /// Per-session event sequence counters. `SessionEvent.seq` must be
    /// monotonic per session across runs (client reconnect dedup/resume
    /// contract), so the counter outlives any single run's `DaemonEventSink`.
    /// std RwLock: critical sections are single HashMap ops, never held across
    /// `.await` (same rationale as `SessionWorkdirs`).
    session_seq_counters: Arc<std::sync::RwLock<HashMap<String, Arc<AtomicU64>>>>,
    /// Per-session replay buffers (fixed-capacity ring, see
    /// `run_loop::SessionEventBuffer`). Lazily created alongside the seq
    /// counter; std RwLock for the same single-HashMap-op rationale.
    session_buffers: Arc<
        std::sync::RwLock<
            HashMap<String, Arc<std::sync::RwLock<crate::daemon::run_loop::SessionEventBuffer>>>,
        >,
    >,
    /// Serializes `PUT /sessions/:id` (load → `expected_version` check →
    /// save) so concurrent writers can't interleave the check-and-set: two
    /// racing PUTs with the same `expected_version` must yield exactly one
    /// success and one 409, never two "successful" writes at the same
    /// version. A single global lock is enough — session saves are small,
    /// infrequent disk writes on a loopback daemon.
    pub session_update_lock: Arc<tokio::sync::Mutex<()>>,
    /// Tracks connected thin clients and triggers graceful shutdown
    /// when the last client disconnects.
    pub active_clients: Arc<ActiveClientTracker>,
    /// Signalled by `POST /api/v1/shutdown` (`wgenty-code daemon stop`) to
    /// request a graceful shutdown; the server's shutdown future listens on
    /// this alongside Ctrl-C and the thin-client idle timeout.
    pub shutdown_notify: Arc<tokio::sync::Notify>,
}

impl DaemonState {
    /// Record the startup-generated bearer token for in-handler auth
    /// (WebSocket query-token path). Called once after `generate_api_token`.
    pub fn set_api_token(&self, token: String) {
        *self.api_token.write().expect("api token lock poisoned") = token;
    }

    /// Current bearer token (empty until [`DaemonState::set_api_token`]).
    pub fn current_api_token(&self) -> String {
        self.api_token
            .read()
            .expect("api token lock poisoned")
            .clone()
    }

    /// Record `(port, lan_exposed)` of the listener. Called once after
    /// `TcpListener::bind` succeeds in `run()` (near `set_api_token`).
    pub fn set_bind(&self, port: u16, lan_exposed: bool) {
        *self.bind.write().expect("bind lock poisoned") = Some((port, lan_exposed));
    }

    /// `(port, lan_exposed)` the listener bound to (`None` until
    /// [`DaemonState::set_bind`]).
    pub fn current_bind(&self) -> Option<(u16, bool)> {
        *self.bind.read().expect("bind lock poisoned")
    }

    pub async fn new(app_state: AppState) -> Self {
        let task_manager = Arc::new(TaskManagementTool::new());
        let todo_state = Arc::new(RwLock::new(TodoState::default()));
        let policy = ToolPermissionPolicy::from_settings(&app_state.settings);

        // Initialize background manager (shares OS sandbox with shell tools)
        let bg_sandbox = Arc::new(crate::sandbox::SandboxManager::new());
        let bg_manager = Arc::new(BackgroundManager::new().with_sandbox(bg_sandbox));

        // Session inbox + global event bus handles. The tool-layer manager
        // hook diverts completed results here instead of its internal drain
        // queue, which remains the CLI path. Only owned, newly inserted
        // results are published.
        let background_result_inbox = Arc::new(RwLock::new(BackgroundResultInbox::default()));
        let global_event_hub = crate::daemon::global_events::new_global_event_hub();
        let global_seq_counter = Arc::new(AtomicU64::new(1));
        let (background_scheduler_tx, background_scheduler_rx) =
            tokio::sync::mpsc::unbounded_channel();
        {
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<BackgroundResult>();
            bg_manager.set_result_hook(move |result| {
                // The receiver lives for the daemon's lifetime; a failed send
                // means the state is shutting down and the result is safe to drop.
                let _ = tx.send(result);
            });
            let inbox = background_result_inbox.clone();
            let hub = global_event_hub.clone();
            let seq = global_seq_counter.clone();
            let scheduler = background_scheduler_tx.clone();
            tokio::spawn(async move {
                // Single consumer: completion order is preserved in the inbox.
                while let Some(result) = rx.recv().await {
                    let session_id = result.session_id.clone();
                    if retain_and_broadcast_background_result(&inbox, &hub, &seq, result).await {
                        if let Some(session_id) = session_id {
                            let _ = scheduler.send(
                                crate::daemon::run_loop::BackgroundSchedulerEvent::ResultReady {
                                    session_id,
                                },
                            );
                        }
                    }
                }
            });
        }

        // Load team manager if .team/config.json exists
        let team_manager = {
            let root = &app_state.settings.storage.working_dir;
            TeamManager::load(root).map(Arc::new)
        };
        crate::utils::startup_timing::mark("daemon state: team manager loaded");

        // Initialize skill loader (needed before registry so TaskTool can use it).
        let skill_loader = {
            let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
            let base_dirs = vec![
                home.join(".wgenty-code"),
                app_state.settings.storage.working_dir.clone(),
            ];
            let loader = SkillLoader::load_from_dirs(&base_dirs);
            Arc::new(loader)
        };
        crate::utils::startup_timing::mark("daemon state: skill loader ready");

        let progress_store: Arc<
            RwLock<HashMap<String, HashMap<String, crate::agent::progress::SubagentProgress>>>,
        > = Arc::new(RwLock::new(HashMap::new()));

        let mcp_manager = Arc::new(crate::mcp::McpManager::new());

        // Single shared coordinator owning all agent spawning, concurrency,
        // and lifecycle for this daemon. Derived from trusted subagent
        // settings; identity is never taken from model JSON. Constructed
        // outside the registry's Arc::new_cyclic so DaemonState can hold it.
        let registry = Arc::new(crate::org_graph::NodeRegistry::builtin(
            &app_state.settings.agent.subagent,
        ));
        let coordinator = Arc::new(
            crate::agent::AgentCoordinator::new(
                app_state.settings.agent.subagent.max_concurrent,
                app_state.settings.agent.subagent.max_depth,
            )
            .with_node_registry(registry),
        );
        // Viewer-bound capability service + viewer-token secret. The secret is
        // random per daemon start; viewer tokens do not survive restart.
        let daemon_viewer_secret = {
            use rand::RngCore;
            let mut bytes = [0u8; 32];
            rand::rngs::OsRng.fill_bytes(&mut bytes);
            bytes
        };
        let capability_service = Arc::new(crate::agent::capability::CapabilityService::new(
            daemon_viewer_secret,
        ));

        // Reserved built-in tool names (extracted from the real registry after
        // construction below) so MCP external tools can avoid name collisions.
        // The MCP connection itself is deferred to a background task so it never
        // blocks the first rendered frame.

        let approval_timeout = app_state.settings.agent.subagent.approval_timeout_secs;
        let permission_bridge = Arc::new(PermissionBridge::with_timeout_secs(approval_timeout));
        let interaction_bridge =
            Arc::new(crate::daemon::interaction_bridge::InteractionBridge::new());
        let shared_session_rules = Arc::new(RwLock::new(HashSet::<String>::new()));
        let root_mode = Arc::new(std::sync::RwLock::new(RootPermissionMode::Normal));
        let effective_mode = Arc::new(std::sync::RwLock::new(
            crate::sandbox::EffectiveMode::Normal,
        ));
        // TaskTool is built inside Arc::new_cyclic, before the per-session
        // Work-Graph store exists. Keep a late-bound handle so it can bind a
        // RootCause child before the child future is spawned.
        let root_cause_runtime_handle = crate::exec_session::root_cause_runtime_handle();
        let permission_modes = PermissionModeStore::new();
        // ── Shared MemoryManager (D1): backs memory_add tool + AutoDream ──
        let memory_manager = Arc::new(crate::context::MemoryManager::with_settings(
            &app_state.settings,
            app_state.settings.storage.working_dir.clone(),
        ));

        // Multi-project registry + per-project memory routing. Created before
        // the tool registry so `memory_add` can route by invocation workdir.
        let projects = crate::daemon::projects::ProjectRegistry::load_default(
            app_state.settings.storage.working_dir.clone(),
        );
        let memory_router = Arc::new(crate::daemon::memory_router::MemoryRouter::new(
            app_state.settings.clone(),
            projects.clone(),
            memory_manager.clone(),
        ));

        // Per-project task/todo routing (mirrors MemoryRouter). The main
        // project reuses the instances built above; registered projects get
        // lazy-initialized instances on first access.
        let task_router = Arc::new(crate::tasks::TaskRouter::new(
            task_manager.clone(),
            app_state.settings.storage.working_dir.clone(),
        ));
        let todo_router = Arc::new(crate::tasks::TodoRouter::new(
            todo_state.clone(),
            app_state.settings.storage.working_dir.clone(),
        ));

        // Shared read connection to the global transcript db for the SSE trace
        // endpoint's cold-start replay (Task 3.4). `None` on failure -> the SSE
        // stream degrades to live-only. Built at the body level (not inside the
        // Arc::new_cyclic closure) so DaemonState can hold it directly.
        let sse_transcript_store = {
            let db_path = std::path::PathBuf::from(&app_state.settings.storage.transcript.db_path);
            match crate::transcript::SubagentTranscriptStore::open(&db_path) {
                Ok(store) => Some(std::sync::Arc::new(store)),
                Err(e) => {
                    tracing::warn!(
                        "Failed to open SSE transcript store at {}: {}. \
                         Trace SSE will stream live-only.",
                        db_path.display(),
                        e
                    );
                    None
                }
            }
        };

        // Use Arc::new_cyclic so the TaskTool holds a valid Weak<ToolRegistry>
        // that points to the *final* Arc allocation — not a temporary one that
        // gets dropped (which would leave a dangling weak reference).
        let tool_registry = Arc::new_cyclic(|weak_reg| {
            let registry = ToolRegistry::with_project_root(
                app_state.settings.storage.working_dir.clone(),
                app_state.settings.agent.checkpoint.keep_n,
            )
            .with_settings(&app_state.settings);
            registry.register(Box::new(BackgroundTool::new(bg_manager.clone())));

            // Team messaging (s09): always available; writes peer mailboxes directly.
            registry.register(Box::new(TeamMessageTool::new()));
            registry.register(Box::new(
                crate::tools::meta::request_approval::RequestApprovalTool::new(),
            ));

            // D1: memory_add tool (routes to the invocation's project pool)
            registry.register(Box::new(crate::tools::meta::MemoryAddTool::with_resolver(
                memory_manager.clone(),
                memory_router.clone(),
            )));

            // Register load_skill tool if skills exist
            if !skill_loader.is_empty() {
                registry.register(Box::new(
                    crate::tools::meta::load_skill::LoadSkillTool::new(skill_loader.clone()),
                ));
            }

            // TaskTool gets a Weak<ToolRegistry> that is valid for the lifetime
            // of this Arc (created by Arc::new_cyclic).
            // Initialize optional transcript store for subagent persistence.
            let transcript_store = {
                let db_path =
                    std::path::PathBuf::from(&app_state.settings.storage.transcript.db_path);
                match crate::transcript::SubagentTranscriptStore::open(&db_path) {
                    Ok(store) => Some(std::sync::Arc::new(store)),
                    Err(e) => {
                        tracing::warn!("Failed to open transcript store at {}: {}. Running without persistence.", db_path.display(), e);
                        None
                    }
                }
            };
            let task_tool = crate::tools::meta::task::TaskTool::new(
                app_state.settings.clone(),
                weak_reg.clone(),
                coordinator.clone(),
                progress_store.clone(),
                transcript_store.clone(),
            )
            .with_permission_bridge(permission_bridge.clone())
            .with_session_rules(shared_session_rules.clone())
            .with_root_mode(root_mode.clone())
            .with_effective_mode(effective_mode.clone())
            .with_root_cause_runtime(root_cause_runtime_handle.clone())
            .with_permission_modes(permission_modes.clone());
            registry.register(Box::new(task_tool));

            // Register subagent trace tool (read-only visualization for subagent transcripts)
            let trace_tool = crate::tools::meta::subagent_trace::SubagentTraceTool::new(
                transcript_store.clone(),
                coordinator.clone(),
            );
            registry.register(Box::new(trace_tool));

            if app_state.settings.agent.rlm.enabled && app_state.settings.agent.rlm.delegate_tool {
                let rlm_tool = crate::tools::meta::rlm::RlmDelegateTool::new(
                    app_state.settings.clone(),
                    weak_reg.clone(),
                    coordinator.clone(),
                    progress_store.clone(),
                    transcript_store.clone(),
                );
                registry.register(Box::new(rlm_tool));
            }

            #[cfg(feature = "scripting")]
            {
                let run_script_tool = crate::tools::meta::run_script::RunScriptTool::new(
                    app_state.settings.clone(),
                    weak_reg.clone(),
                    coordinator.clone(),
                    transcript_store.clone(),
                );
                registry.register(Box::new(run_script_tool));
            }

            // Wire external skill registry into the skill tool so the model can
            // invoke external skills via the `skill` tool (fixes C1).
            let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
            let project_root = app_state.settings.storage.working_dir.clone();
            let external_registry_roots =
                crate::knowledge::SkillRootResolver::roots_with(&home, &project_root);
            if let Ok(external_registry) =
                crate::knowledge::ExternalSkillRegistry::discover(external_registry_roots)
            {
                if !external_registry.list().is_empty() {
                    registry.wire_skill_registry(std::sync::Arc::new(external_registry));
                }
            }

            registry
        });
        crate::utils::startup_timing::mark("daemon state: tool registry built");
        let checkpoint_manager = tool_registry.checkpoint_manager.clone();
        let checkpoint_store = tool_registry.checkpoint_store.clone();
        let work_graph_runtime_store =
            Arc::new(crate::exec_session::ExecutionSessionRuntimeStore::new(
                app_state.settings.storage.working_dir.clone(),
                checkpoint_store.clone(),
                2,
            ));
        *root_cause_runtime_handle
            .write()
            .expect("lock poisoned: root-cause runtime handle") =
            Some(work_graph_runtime_store.clone());
        tool_registry.register_exec_session_tools(work_graph_runtime_store.clone());
        tool_registry.enable_static_root_cause_route(work_graph_runtime_store.clone());
        tool_registry
            .register_specialist_report_tool(work_graph_runtime_store.clone(), coordinator.clone());

        // ── D1: AutoDream startup check (fire-and-forget) ────────────────
        // Replaces the old TUI app-side AutoDream spawn (removed in Task 4).
        // Runs once per registered project so each project's pool is
        // consolidated independently.
        {
            let router = Arc::clone(&memory_router);
            tokio::spawn(async move {
                for mgr in router.all().await {
                    let autodream = crate::services::AutoDreamService::new(None, Some(mgr));
                    match autodream.check_and_run().await {
                        Ok(true) => tracing::info!("AutoDream: consolidation triggered"),
                        Ok(false) => tracing::debug!("AutoDream: gate not met, skipped"),
                        Err(e) => tracing::warn!(error = %e, "AutoDream check_and_run failed"),
                    }
                }
            });
        }

        // Extract reserved tool names from the real registry (no throwaway
        // construction needed - avoids a second ToolRegistry::new() which
        // re-creates all built-in tool instances).
        let reserved_tool_names: HashSet<String> = tool_registry
            .list()
            .into_iter()
            .map(|tool| tool.name().to_string())
            .collect();

        // ── Background MCP tool connection (non-blocking) ────────────────
        // Connect to configured MCP servers in the background so the daemon
        // starts (and the TUI renders its first frame) without waiting for
        // subprocess spawns + initialize/tools/list handshakes. External tools
        // are registered into the live registry via register(&self) once each
        // server handshake completes. If the user submits a prompt before MCP
        // tools are ready, the request proceeds with built-in tools only - the
        // model never sees MCP tools until they are registered.
        {
            let mcp_manager = Arc::clone(&mcp_manager);
            let tool_registry = Arc::clone(&tool_registry);
            let settings = app_state.settings.clone();
            let mut reserved = reserved_tool_names;
            tokio::spawn(async move {
                let external_tools = mcp_manager
                    .connect_configured_tools(&settings, &mut reserved)
                    .await;
                crate::utils::startup_timing::mark(
                    "daemon state: mcp tools connected (background)",
                );
                let count = external_tools.len();
                for tool in external_tools {
                    tool_registry.register(tool);
                }
                crate::utils::startup_timing::mark("daemon state: mcp tools registered");
                tracing::info!(
                    registered = count,
                    "background MCP tool connection complete"
                );
            });
        }

        // Initialize HookManager from settings hooks configuration
        let hooks_config = app_state
            .settings
            .integrations
            .hooks
            .as_ref()
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let hook_manager = Arc::new(HookManager::from_settings(&hooks_config));
        // Project-local sessions: `<working_dir>/.wgenty-code/sessions/`
        // (falls back to ~/.wgenty-code/sessions if the project dir is unwritable).
        // Do not use SessionManager::new() here — that always writes to the global
        // home directory and diverges from WGENTY.md / historical project sessions.
        let session_manager =
            MemorySessionManager::with_project_root(app_state.settings.storage.working_dir.clone());
        let tool_executor = ToolExecutor::new(tool_registry.clone(), policy)
            .with_hooks(hook_manager.clone())
            .with_shared_session_rules(shared_session_rules);

        // Shared pooled HTTP clients for LLM API calls. Built once so every
        // per-request `ApiClient` reuses the keep-alive pool + TLS session
        // cache instead of re-handshaking TLS on every chat turn.
        let (http_client, http_client_stream) =
            crate::api::ApiClient::build_clients(&app_state.settings);

        // Attach a tier-2 LLM for ambiguous-relation review (P2-B). Uses the
        // shared pooled clients so review calls reuse the keep-alive pool.
        // Model identity comes from the startup snapshot; `/model` switches do
        // not retroactively re-bind the reviewer, which is acceptable since
        // review is low-frequency and model-quality-insensitive (one-word
        // verdict). If construction fails, review degrades to the legacy
        // merge+tag path — never blocks daemon startup.
        {
            let api_client = crate::api::ApiClient::with_clients(
                app_state.settings.clone(),
                http_client.clone(),
                http_client_stream.clone(),
            );
            let llm: Arc<dyn crate::agent::runtime::LlmPort> =
                Arc::new(crate::agent::runtime::ApiLlmPort::new(api_client));
            let review = Arc::new(crate::agent::runtime::adapters::MemoryReviewAdapter::new(
                llm,
            ));
            memory_router.set_review_llm(Some(review)).await;
        }

        // Live settings handle seeded from the startup snapshot. Runtime
        // handlers clone from here per request so `/model` switches take
        // effect on the next turn without a restart.
        let settings_handle: crate::config::watcher::SettingsHandle =
            Arc::new(std::sync::RwLock::new(app_state.settings.clone()));

        Self {
            app_state,
            settings_handle,
            tool_executor,
            tool_registry,
            checkpoint_manager,
            checkpoint_store,
            work_graph_runtime_store,
            task_manager,
            todo_state,
            task_router,
            todo_router,
            skill_loader,
            background_manager: bg_manager,
            background_result_inbox,
            message_inbox: Arc::new(RwLock::new(MessageInbox::default())),
            background_scheduler_tx,
            background_scheduler_rx: std::sync::Mutex::new(Some(background_scheduler_rx)),
            background_scheduler_started: std::sync::atomic::AtomicBool::new(false),
            team_manager,
            session_manager,
            memory_manager,
            hook_manager,
            mcp_manager,
            sessions: Arc::new(RwLock::new(std::collections::HashMap::new())),
            subagent_progress: progress_store,
            subagent_poll_times: Arc::new(RwLock::new(HashMap::new())),
            coordinator,
            capability_service,
            viewer_tokens: Arc::new(RwLock::new(HashMap::new())),
            root_contexts: Arc::new(RwLock::new(HashMap::new())),
            session_workdirs: Arc::new(std::sync::RwLock::new(HashMap::new())),
            projects,
            memory_router,
            project_session_managers: Arc::new(RwLock::new(HashMap::new())),
            project_checkpoints: Arc::new(std::sync::RwLock::new(HashMap::new())),
            daemon_viewer_secret,
            permission_bridge,
            interaction_bridge,
            permission_modes,
            effective_mode,
            transcript_store: sse_transcript_store,
            session_event_hub: tokio::sync::broadcast::channel(1024).0,
            global_event_hub,
            global_seq_counter,
            session_runs: crate::daemon::run_loop::RunRegistry::new(),
            session_seq_counters: Arc::new(std::sync::RwLock::new(HashMap::new())),
            session_buffers: Arc::new(std::sync::RwLock::new(HashMap::new())),
            session_update_lock: Arc::new(tokio::sync::Mutex::new(())),
            api_token: std::sync::RwLock::new(String::new()),
            bind: std::sync::RwLock::new(None),
            active_clients: Arc::new(ActiveClientTracker::new()),
            shutdown_notify: Arc::new(tokio::sync::Notify::new()),
            http_client,
            http_client_stream,
        }
    }

    /// Publish one global event. No subscribers is normal; ignore the error.
    pub fn broadcast_global(
        &self,
        kind: crate::daemon::global_events::GlobalEventKind,
        data: serde_json::Value,
    ) {
        let event = crate::daemon::global_events::GlobalEvent {
            seq: self
                .global_seq_counter
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            kind,
            data,
        };
        let _ = self.global_event_hub.send(event);
    }

    /// Retain-then-broadcast: the result MUST be queryable before any client
    /// sees the event, so offline-then-online clients can still fetch it.
    pub async fn record_background_result(&self, result: BackgroundResult) -> bool {
        let session_id = result.session_id.clone();
        let retained = retain_and_broadcast_background_result(
            &self.background_result_inbox,
            &self.global_event_hub,
            &self.global_seq_counter,
            result,
        )
        .await;
        if retained {
            if let Some(session_id) = session_id {
                let _ = self.background_scheduler_tx.send(
                    crate::daemon::run_loop::BackgroundSchedulerEvent::ResultReady { session_id },
                );
            }
        }
        retained
    }

    /// Snapshot of all unacknowledged owned results (oldest first), including
    /// both pending and claimed entries. Reading never changes lifecycle.
    pub async fn background_results_snapshot(&self) -> Vec<BackgroundResult> {
        self.background_result_inbox.read().await.snapshot_all()
    }

    /// Snapshot unacknowledged results for exactly one session (oldest first).
    pub async fn background_results_snapshot_for_session(
        &self,
        session_id: &str,
    ) -> Vec<BackgroundResult> {
        self.background_result_inbox
            .read()
            .await
            .snapshot_for_session(session_id)
    }

    pub(crate) async fn claim_background_results_for_session(
        &self,
        session_id: &str,
        run_id: &str,
    ) -> Vec<BackgroundResult> {
        self.background_result_inbox
            .write()
            .await
            .claim_for_session(session_id, run_id)
    }

    pub(crate) async fn ack_background_result_claim(&self, session_id: &str, run_id: &str) -> bool {
        self.background_result_inbox
            .write()
            .await
            .ack_claim(session_id, run_id)
    }

    pub(crate) async fn requeue_background_result_claim(
        &self,
        session_id: &str,
        run_id: &str,
    ) -> bool {
        self.background_result_inbox
            .write()
            .await
            .requeue_claim(session_id, run_id)
    }

    pub(crate) async fn has_pending_background_results(&self, session_id: &str) -> bool {
        self.background_result_inbox
            .read()
            .await
            .has_pending_for_session(session_id)
    }

    // ── Queued user messages (run-slot busy → FIFO, drained by scheduler) ──

    pub(crate) async fn enqueue_message(
        &self,
        session_id: &str,
        message: String,
        plan_mode: bool,
    ) -> Result<QueuedMessage, MessageQueueFull> {
        self.message_inbox
            .write()
            .await
            .enqueue(session_id, message, plan_mode)
    }

    pub async fn queued_messages_snapshot(&self, session_id: &str) -> Vec<QueuedMessage> {
        self.message_inbox
            .read()
            .await
            .snapshot_for_session(session_id)
    }

    pub(crate) async fn message_queue_depth(&self, session_id: &str) -> usize {
        self.message_inbox.read().await.depth(session_id)
    }

    pub(crate) async fn has_queued_messages(&self, session_id: &str) -> bool {
        self.message_inbox.read().await.has_pending(session_id)
    }

    pub(crate) async fn claim_queued_message(
        &self,
        session_id: &str,
        run_id: &str,
    ) -> Option<QueuedMessage> {
        self.message_inbox
            .write()
            .await
            .claim_head(session_id, run_id)
    }

    pub(crate) async fn ack_message_claim(&self, session_id: &str, run_id: &str) -> bool {
        self.message_inbox
            .write()
            .await
            .ack_claim(session_id, run_id)
    }

    pub(crate) async fn requeue_message_claim(&self, session_id: &str, run_id: &str) -> bool {
        self.message_inbox
            .write()
            .await
            .requeue_claim(session_id, run_id)
    }

    pub(crate) async fn remove_queued_message(&self, session_id: &str, message_id: &str) -> bool {
        self.message_inbox
            .write()
            .await
            .remove(session_id, message_id)
    }

    pub(crate) async fn clear_queued_messages(&self, session_id: &str) -> usize {
        self.message_inbox.write().await.clear(session_id)
    }

    pub(crate) async fn background_continuation_is_blocked(&self, session_id: &str) -> bool {
        self.background_result_inbox
            .read()
            .await
            .continuation_is_blocked(session_id)
    }

    pub(crate) async fn block_background_continuation(&self, session_id: &str) {
        self.background_result_inbox
            .write()
            .await
            .block_continuation(session_id);
    }

    pub(crate) async fn allow_background_continuation(&self, session_id: &str) {
        self.background_result_inbox
            .write()
            .await
            .allow_continuation(session_id);
    }

    /// Start the daemon-owned continuation scheduler once `DaemonState` is in
    /// an `Arc`. Router construction is the common production/test lifecycle
    /// seam where that ownership is available.
    pub fn start_background_continuation_scheduler(self: &Arc<Self>) {
        let receiver = self
            .background_scheduler_rx
            .lock()
            .expect("background scheduler receiver lock poisoned")
            .take();
        let Some(receiver) = receiver else {
            return;
        };
        self.background_scheduler_started
            .store(true, std::sync::atomic::Ordering::Release);
        crate::daemon::run_loop::spawn_background_continuation_scheduler(
            Arc::downgrade(self),
            receiver,
        );

        // Forward coordinator task-group readiness pings into the scheduler so
        // completed subagent groups are delivered to the root agent
        // server-side, without relying on a polling client.
        let mut ready_rx = self.coordinator.subscribe_ready_groups();
        let tx = self.background_scheduler_tx.clone();
        tokio::spawn(async move {
            loop {
                match ready_rx.recv().await {
                    Ok(session_id) => {
                        let _ = tx.send(
                            crate::daemon::run_loop::BackgroundSchedulerEvent::TaskGroupReady {
                                session_id: session_id.as_str().to_string(),
                            },
                        );
                    }
                    // Pings are idempotent hints; a lagged receiver just
                    // re-checks readiness on the next one.
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    pub(crate) fn background_scheduler_sender(
        &self,
    ) -> Option<tokio::sync::mpsc::UnboundedSender<crate::daemon::run_loop::BackgroundSchedulerEvent>>
    {
        self.background_scheduler_started
            .load(std::sync::atomic::Ordering::Acquire)
            .then(|| self.background_scheduler_tx.clone())
    }

    /// Single write-path for the shared todo list: update state, then
    /// broadcast a full-snapshot TodosChanged (snapshots are small; YAGNI:
    /// no incremental diff). Routes to the session's project todo state so
    /// multi-project sessions never cross-contaminate. `project_path` is
    /// included in the broadcast so clients can filter.
    pub async fn apply_todos_update(&self, session_id: &str, items: Vec<crate::tasks::TodoItem>) {
        let root = self.effective_session_root(session_id).await;
        let todo_state = self.todo_router.for_project(&root).await;
        {
            let mut todos = todo_state.write().await;
            todos.items = items;
        }
        let snapshot = {
            let todos = todo_state.read().await;
            serde_json::json!({
                "project": root,
                "items": todos.items,
                "has_open_items": todos.has_open_items(),
            })
        };
        self.broadcast_global(
            crate::daemon::global_events::GlobalEventKind::TodosChanged,
            snapshot,
        );
    }

    /// Task manager for the session's project (get-or-create via router).
    /// HTTP handlers and the run loop use this so tasks are isolated per
    /// project, mirroring `session_manager_for_project` / `checkpoints_for_project`.
    pub async fn task_manager_for_session(
        &self,
        session_id: &str,
    ) -> Arc<crate::tasks::TaskManagementTool> {
        let root = self.effective_session_root(session_id).await;
        self.task_router.for_project(&root).await
    }

    /// Todo state for the session's project (get-or-create via router).
    pub async fn todo_state_for_session(
        &self,
        session_id: &str,
    ) -> Arc<RwLock<crate::tasks::TodoState>> {
        let root = self.effective_session_root(session_id).await;
        self.todo_router.for_project(&root).await
    }

    /// Returns the trusted root execution context for `session_id`, creating
    /// it via `ensure_root` on first use. Never accepts agent ID, parent ID, or
    /// depth from request JSON.
    pub async fn root_context(
        &self,
        session_id: &str,
    ) -> anyhow::Result<crate::agent::AgentExecutionContext> {
        let context = {
            let roots = self.root_contexts.read().await;
            if let Some(ctx) = roots.get(session_id) {
                ctx.clone()
            } else {
                drop(roots);
                let ctx = self
                    .coordinator
                    .ensure_root(crate::agent::SessionId::new(session_id))
                    .await
                    .map_err(|e| anyhow::anyhow!("ensure_root failed: {e}"))?;
                let mut roots = self.root_contexts.write().await;
                roots.insert(session_id.to_string(), ctx.clone());
                ctx
            }
        };
        let tool_context = crate::agent::ToolContext {
            agent: &context,
            invocation_id: crate::agent::ToolInvocationId::new(uuid::Uuid::new_v4().to_string()),
            origin_turn_id: None,
            workdir: Some(&self.app_state.settings.storage.working_dir),
            effective_mode: *self.effective_mode.read().expect("effective mode lock"),
            checkpoint: Some(self.checkpoint_store.as_ref()),
        };
        if let Some(child_id) = self
            .tool_registry
            .dispatch_recovered_root_cause(&tool_context)
            .await
            .map_err(|error| anyhow::anyhow!("recover root-cause specialist: {}", error.message))?
        {
            tracing::info!(
                session_id,
                child_id,
                "redispatched recovered root-cause specialist"
            );
        }
        Ok(context)
    }

    /// Creates a trusted UI viewer: generates a 256-bit bearer token, stores
    /// only its HMAC digest mapped to a fresh ViewerId, and returns the raw
    /// token once.
    pub async fn create_viewer(&self) -> String {
        use rand::RngCore;
        let mut bytes = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        let token = hex_string(&bytes);
        let digest = self.digest_viewer_token(&token);
        let viewer_id = crate::agent::capability::ViewerId::new(token.clone());
        let mut tokens = self.viewer_tokens.write().await;
        tokens.insert(digest, viewer_id);
        token
    }

    /// Resolves a viewer bearer token to its ViewerId. Returns None for
    /// missing/unknown tokens; callers surface one stable unauthorized
    /// response regardless of cause.
    pub async fn resolve_viewer(&self, token: &str) -> Option<crate::agent::capability::ViewerId> {
        let digest = self.digest_viewer_token(token);
        let tokens = self.viewer_tokens.read().await;
        tokens.get(&digest).cloned()
    }

    /// Computes the HMAC-SHA256 digest of a viewer token under the daemon
    /// viewer secret. Used as the lookup key; the raw token is never stored.
    fn digest_viewer_token(&self, token: &str) -> String {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        type HmacSha256 = Hmac<Sha256>;
        let mut mac = HmacSha256::new_from_slice(&self.daemon_viewer_secret)
            .expect("HMAC accepts any key size");
        mac.update(token.as_bytes());
        let bytes = mac.finalize().into_bytes();
        hex_string(bytes.as_slice())
    }

    /// Check if a session rule is approved for a given session.
    pub async fn is_rule_approved(&self, session_id: &str, rule: &str) -> bool {
        let sessions = self.sessions.read().await;
        sessions
            .get(session_id)
            .map(|s| s.approved.contains(rule))
            .unwrap_or(false)
    }

    /// Approve a session rule.
    pub async fn approve_rule(&self, session_id: &str, rule: String) {
        let mut sessions = self.sessions.write().await;
        sessions
            .entry(session_id.to_string())
            .or_insert_with(SessionRules::new)
            .approved
            .insert(rule);
    }

    /// Remove a previously approved session rule.
    pub async fn unapprove_rule(&self, session_id: &str, rule: &str) {
        let mut sessions = self.sessions.write().await;
        if let Some(s) = sessions.get_mut(session_id) {
            s.approved.remove(rule);
        }
    }

    /// Record a poll time for the given session.
    pub async fn touch_subagent_session(&self, session_id: &str) {
        let mut poll_times = self.subagent_poll_times.write().await;
        poll_times.insert(session_id.to_string(), Instant::now());
    }

    /// Remove subagent progress entries for sessions that haven't been polled
    /// within `ttl` duration.
    pub async fn cleanup_stale_subagent_sessions(&self, ttl: std::time::Duration) {
        let now = Instant::now();
        let mut poll_times = self.subagent_poll_times.write().await;
        let mut progress = self.subagent_progress.write().await;

        // Collect stale session IDs
        let stale: Vec<String> = poll_times
            .iter()
            .filter(|(_, last)| now.duration_since(**last) > ttl)
            .map(|(sid, _)| sid.clone())
            .collect();

        for sid in &stale {
            poll_times.remove(sid);
            progress.remove(sid);
        }

        if !stale.is_empty() {
            tracing::debug!(
                "Cleaned up {} stale subagent session(s) (TTL={:?})",
                stale.len(),
                ttl
            );
        }
    }
}

/// Enqueue-then-broadcast core shared by [`DaemonState::record_background_result`]
/// and the tool-layer manager hook installed in [`DaemonState::new`] (which
/// runs before the `DaemonState` itself exists). Enqueue MUST complete before
/// broadcast; unowned and duplicate results produce no notification.
async fn retain_and_broadcast_background_result(
    inbox: &RwLock<BackgroundResultInbox>,
    hub: &crate::daemon::global_events::GlobalEventHub,
    seq_counter: &AtomicU64,
    result: BackgroundResult,
) -> bool {
    if !inbox.write().await.enqueue(result.clone()) {
        return false;
    }
    let event = crate::daemon::global_events::GlobalEvent {
        seq: seq_counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        kind: crate::daemon::global_events::GlobalEventKind::BackgroundResult,
        data: serde_json::json!({ "result": result }),
    };
    // No subscribers is normal; ignore the error (same as broadcast_global).
    let _ = hub.send(event);
    true
}

/// Encodes bytes as a lowercase hex string.
fn hex_string(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

#[cfg(test)]
mod background_inbox_lifecycle_tests {
    use super::*;

    fn result_for_session(task_id: &str, session_id: &str) -> BackgroundResult {
        BackgroundResult {
            task_id: task_id.to_string(),
            session_id: Some(session_id.to_string()),
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

    fn result(task_id: &str) -> BackgroundResult {
        result_for_session(task_id, "session-a")
    }

    #[test]
    fn another_sessions_traffic_cannot_evict_consumed_tombstones() {
        let mut inbox = BackgroundResultInbox::default();
        assert!(inbox.enqueue(result("anchor")));
        assert_eq!(inbox.claim_for_session("session-a", "run-a").len(), 1);
        assert!(inbox.ack_claim("session-a", "run-a"));

        for index in 0..=BACKGROUND_RESULT_TOMBSTONE_CAPACITY {
            let task_id = format!("foreign_{index}");
            assert!(inbox.enqueue(result_for_session(&task_id, "session-b")));
            assert_eq!(
                inbox
                    .claim_for_session("session-b", &format!("run-b-{index}"))
                    .len(),
                1
            );
            assert!(inbox.ack_claim("session-b", &format!("run-b-{index}")));
        }

        assert!(
            !inbox.enqueue(result("anchor")),
            "another session must not evict session-a's exact-once memory"
        );
    }

    #[test]
    fn enqueue_after_requeue_preserves_the_recovery_batch() {
        let mut inbox = BackgroundResultInbox::default();
        for index in 0..BACKGROUND_RESULTS_CAPACITY {
            assert!(inbox.enqueue(result(&format!("recovery_{index}"))));
        }
        assert_eq!(
            inbox.claim_for_session("session-a", "failed-run").len(),
            BACKGROUND_RESULTS_CAPACITY
        );
        for index in 0..BACKGROUND_RESULTS_CAPACITY {
            assert!(inbox.enqueue(result(&format!("newer_{index}"))));
        }
        assert!(inbox.requeue_claim("session-a", "failed-run"));

        assert!(inbox.enqueue(result("newest")));
        let retained = inbox.snapshot_for_session("session-a");
        assert!(
            retained.iter().any(|result| result.task_id == "recovery_0"),
            "new traffic must never evict the oldest requeued recovery result"
        );
        assert!(retained.iter().any(|result| result.task_id == "newest"));
        assert!(retained.len() <= BACKGROUND_RESULTS_MAX_LIVE_PER_SESSION);
    }

    #[test]
    fn consumed_tombstones_evict_oldest_at_the_per_session_capacity() {
        let mut inbox = BackgroundResultInbox::default();
        for index in 0..=BACKGROUND_RESULT_TOMBSTONE_CAPACITY {
            let task_id = format!("bg_{index}");
            assert!(inbox.enqueue(result(&task_id)));
            let claimed = inbox.claim_for_session("session-a", &format!("run_{index}"));
            assert_eq!(claimed.len(), 1);
            assert_eq!(claimed[0].task_id, task_id);
            assert!(inbox.ack_claim("session-a", &format!("run_{index}")));
        }

        assert!(
            inbox.enqueue(result("bg_0")),
            "oldest tombstone is evicted at the bounded replay window"
        );
        assert!(
            !inbox.enqueue(result(&format!(
                "bg_{}",
                BACKGROUND_RESULT_TOMBSTONE_CAPACITY
            ))),
            "newest consumed task remains deduplicated"
        );
    }

    #[test]
    fn repeated_requeue_keeps_live_session_results_bounded() {
        let mut inbox = BackgroundResultInbox::default();
        for index in 0..BACKGROUND_RESULTS_CAPACITY {
            assert!(inbox.enqueue(result(&format!("initial_{index}"))));
        }
        assert_eq!(
            inbox.claim_for_session("session-a", "run_0").len(),
            BACKGROUND_RESULTS_CAPACITY
        );
        for index in 0..BACKGROUND_RESULTS_CAPACITY {
            assert!(inbox.enqueue(result(&format!("during_0_{index}"))));
        }
        assert!(inbox.requeue_claim("session-a", "run_0"));

        for cycle in 1..=3 {
            assert_eq!(
                inbox
                    .claim_for_session("session-a", &format!("run_{cycle}"))
                    .len(),
                BACKGROUND_RESULTS_CAPACITY,
                "one claim never exceeds the configured batch capacity"
            );
            for index in 0..BACKGROUND_RESULTS_CAPACITY {
                assert!(inbox.enqueue(result(&format!("during_{cycle}_{index}"))));
            }
            assert!(inbox.requeue_claim("session-a", &format!("run_{cycle}")));
            assert!(
                inbox.snapshot_for_session("session-a").len()
                    <= BACKGROUND_RESULTS_MAX_LIVE_PER_SESSION,
                "repeated startup crashes keep live memory bounded"
            );
        }
    }

    #[test]
    fn consumed_tombstone_is_scoped_by_session_and_task_id() {
        let mut inbox = BackgroundResultInbox::default();
        assert!(inbox.enqueue(result("shared")));
        assert_eq!(inbox.claim_for_session("session-a", "run-a").len(), 1);
        assert!(inbox.ack_claim("session-a", "run-a"));

        let mut foreign = result("shared");
        foreign.session_id = Some("session-b".to_string());
        assert!(inbox.enqueue(foreign));
        assert!(!inbox.enqueue(result("shared")));
    }
}

// ── Session → worktree binding (project v1) ──────────────────────────────────

/// Shared map type for session → bound worktree path. std RwLock: critical
/// sections are a single HashMap op, never held across .await.
pub type SessionWorkdirs = Arc<std::sync::RwLock<HashMap<String, PathBuf>>>;

/// Bind `session_id` to a worktree path (multiple sessions may share one path).
pub(crate) fn bind_in(map: &SessionWorkdirs, session_id: &str, path: PathBuf) {
    map.write()
        .expect("session_workdirs lock poisoned")
        .insert(session_id.to_string(), path);
}

/// Remove a session's binding (the on-disk worktree is untouched).
pub(crate) fn unbind_in(map: &SessionWorkdirs, session_id: &str) {
    map.write()
        .expect("session_workdirs lock poisoned")
        .remove(session_id);
}

/// The session's bound worktree, or None (= main working_dir, current behavior).
pub(crate) fn workdir_of(map: &SessionWorkdirs, session_id: &str) -> Option<PathBuf> {
    map.read()
        .expect("session_workdirs lock poisoned")
        .get(session_id)
        .cloned()
}

/// All sessions bound to `path` (reverse lookup, e.g. before worktree removal).
pub(crate) fn sessions_of(map: &SessionWorkdirs, path: &std::path::Path) -> Vec<String> {
    map.read()
        .expect("session_workdirs lock poisoned")
        .iter()
        .filter(|(_, p)| p.as_path() == path)
        .map(|(sid, _)| sid.clone())
        .collect()
}

impl DaemonState {
    /// Bind `session_id` to a worktree path.
    pub fn bind_session_worktree(&self, session_id: &str, path: PathBuf) {
        bind_in(&self.session_workdirs, session_id, path);
    }

    /// Remove a session's binding (session falls back to the main working_dir).
    pub fn unbind_session_worktree(&self, session_id: &str) {
        unbind_in(&self.session_workdirs, session_id);
    }

    /// The session's bound worktree, or None (= main working_dir).
    pub fn session_workdir(&self, session_id: &str) -> Option<PathBuf> {
        workdir_of(&self.session_workdirs, session_id)
    }

    /// All sessions bound to `path` (used before removing a worktree).
    pub fn worktree_sessions(&self, path: &std::path::Path) -> Vec<String> {
        sessions_of(&self.session_workdirs, path)
    }

    /// The session's event sequence counter (get-or-create, starts at 1).
    ///
    /// Shared by every `DaemonEventSink` the session spawns, so `SessionEvent`
    /// `.seq` keeps increasing across runs of one session and clients can
    /// dedup/order by seq alone.
    pub fn session_seq_counter(&self, session_id: &str) -> Arc<AtomicU64> {
        {
            let counters = self
                .session_seq_counters
                .read()
                .expect("session_seq_counters lock poisoned");
            if let Some(counter) = counters.get(session_id) {
                return Arc::clone(counter);
            }
        }
        Arc::clone(
            self.session_seq_counters
                .write()
                .expect("session_seq_counters lock poisoned")
                .entry(session_id.to_string())
                .or_insert_with(|| Arc::new(AtomicU64::new(1))),
        )
    }

    /// Lazily-created per-session replay buffer, mirroring `session_seq_counter`.
    /// Publish call sites (`DaemonEventSink`, `RootToolPort`) dual-write into
    /// it. `pub` so integration tests can build a sink over the real buffer
    /// and exercise `after=` replay end-to-end.
    pub fn session_buffer(
        &self,
        session_id: &str,
    ) -> Arc<std::sync::RwLock<crate::daemon::run_loop::SessionEventBuffer>> {
        let capacity = self.event_buffer_capacity();
        let mut map = self
            .session_buffers
            .write()
            .expect("session_buffers lock poisoned");
        map.entry(session_id.to_string())
            .or_insert_with(|| {
                Arc::new(std::sync::RwLock::new(
                    crate::daemon::run_loop::SessionEventBuffer::new(capacity),
                ))
            })
            .clone()
    }

    pub fn event_buffer_capacity(&self) -> usize {
        self.app_state.settings.daemon.event_buffer_capacity
    }

    // ── Multi-project routing ────────────────────────────────────────────────

    /// Session manager for a project root (get-or-create; runs `load_index`
    /// on first creation so historical sessions are visible). The main
    /// project returns the primary `session_manager`.
    pub async fn session_manager_for_project(&self, root: &Path) -> MemorySessionManager {
        if *root == self.projects.main_root() {
            return self.session_manager.clone();
        }
        {
            let map = self.project_session_managers.read().await;
            if let Some(mgr) = map.get(root) {
                return mgr.clone();
            }
        }
        let mgr = MemorySessionManager::with_project_root(root.to_path_buf());
        if let Err(e) = mgr.load_index().await {
            tracing::warn!(error = %e, root = %root.display(), "project session load_index failed");
        }
        let mut map = self.project_session_managers.write().await;
        // Double-check: a concurrent creator may have won the race; its
        // instance is equally valid and already warmed.
        map.entry(root.to_path_buf()).or_insert(mgr).clone()
    }

    /// The project root a session belongs to: its `project_path`, or the main
    /// project when unset (legacy sessions).
    pub fn session_project_root(
        &self,
        session: &crate::context::memory_session::Session,
    ) -> PathBuf {
        session
            .project_path
            .clone()
            .unwrap_or_else(|| self.projects.main_root())
    }

    /// Find a session across all project managers. Returns the owning manager
    /// together with the session; callers capture the manager so subsequent
    /// saves keep landing in the same store even if the project is
    /// unregistered mid-run.
    pub async fn resolve_session(
        &self,
        session_id: &str,
    ) -> Option<(
        MemorySessionManager,
        crate::context::memory_session::Session,
    )> {
        if let Ok(Some(s)) = self.session_manager.load(session_id).await {
            return Some((self.session_manager.clone(), s));
        }
        for root in self.projects.registered_roots() {
            let mgr = self.session_manager_for_project(&root).await;
            if let Ok(Some(s)) = mgr.load(session_id).await {
                return Some((mgr, s));
            }
        }
        None
    }

    /// The session's effective working root: bound worktree > the session's
    /// project root > the main working_dir. This is the single source of
    /// truth for tool path resolution AND permission-policy rooting — the two
    /// must never diverge (a relative path is validated against the policy
    /// root and executed against the workdir).
    pub async fn effective_session_root(&self, session_id: &str) -> PathBuf {
        if let Some(wd) = self.session_workdir(session_id) {
            return wd;
        }
        if let Some((_mgr, session)) = self.resolve_session(session_id).await {
            if let Some(p) = session.project_path {
                return p;
            }
        }
        self.projects.main_root()
    }

    /// Checkpoint handles for a project root (get-or-create). The main
    /// project returns the tool registry's handles so existing snapshots and
    /// the `checkpoint`/`undo` tools keep working unchanged.
    pub fn checkpoints_for_project(&self, root: &Path) -> ProjectCheckpointHandles {
        if *root == self.projects.main_root() {
            return (
                Arc::clone(&self.checkpoint_manager),
                Arc::clone(&self.checkpoint_store),
            );
        }
        if let Some(pair) = self
            .project_checkpoints
            .read()
            .expect("project_checkpoints lock poisoned")
            .get(root)
        {
            return pair.clone();
        }
        let keep_n = self.app_state.settings.agent.checkpoint.keep_n;
        let store = Arc::new(CheckpointStore::with_keep_n(root.to_path_buf(), keep_n));
        let manager = Arc::new(CheckpointManager::new(store.clone()));
        self.project_checkpoints
            .write()
            .expect("project_checkpoints lock poisoned")
            .entry(root.to_path_buf())
            .or_insert((manager, store))
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_workdirs_bind_query_unbind() {
        let map: SessionWorkdirs = Arc::new(std::sync::RwLock::new(HashMap::new()));
        bind_in(&map, "s1", PathBuf::from("/repo/.worktrees/a"));
        bind_in(&map, "s2", PathBuf::from("/repo/.worktrees/a"));
        bind_in(&map, "s3", PathBuf::from("/repo/.worktrees/b"));

        assert_eq!(
            workdir_of(&map, "s1").unwrap(),
            PathBuf::from("/repo/.worktrees/a")
        );
        assert!(workdir_of(&map, "nobody").is_none());

        let mut sessions = sessions_of(&map, std::path::Path::new("/repo/.worktrees/a"));
        sessions.sort();
        assert_eq!(sessions, vec!["s1".to_string(), "s2".to_string()]);

        unbind_in(&map, "s1");
        assert!(workdir_of(&map, "s1").is_none());
        assert_eq!(
            sessions_of(&map, std::path::Path::new("/repo/.worktrees/a")),
            vec!["s2".to_string()]
        );
    }
}
