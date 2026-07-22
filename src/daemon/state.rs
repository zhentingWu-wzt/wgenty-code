//! DaemonState -- shared state for the HTTP API server.

use crate::config::agent::RootPermissionMode;
use crate::context::memory_session::SessionManager as MemorySessionManager;
use crate::knowledge::loader::SkillLoader;
use crate::permissions::ToolPermissionPolicy;
use crate::runtime::hooks::HookManager;
use crate::state::AppState;
use crate::tasks::{TaskManagementTool, TodoState};
use crate::teams::mailbox::TeamManager;
use crate::teams::permission_bridge::PermissionBridge;
use crate::tools::execution::background::{BackgroundManager, BackgroundTool};
use crate::tools::meta::team_message::TeamMessageTool;
use crate::tools::{CheckpointManager, CheckpointStore, ToolExecutor, ToolRegistry};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

/// Per-session permission rules.
struct SessionRules {
    approved: HashSet<String>,
}

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
    /// Shared pooled HTTP clients for LLM API calls. Built once at startup
    /// from `app_state.settings` so the reqwest keep-alive pool + TLS session
    /// cache are reused across every per-request `ApiClient` - avoids a fresh
    /// TCP + TLS handshake on each chat turn. `app_state.settings` is a static
    /// snapshot for the daemon lifetime (no runtime reload), so the
    /// connect/read timeouts baked in here match what per-request
    /// `ApiClient::new` would have produced anyway.
    pub http_client: Arc<reqwest::Client>,
    pub http_client_stream: Arc<reqwest::Client>,
    pub tool_registry: Arc<ToolRegistry>,
    pub tool_executor: ToolExecutor,
    pub checkpoint_manager: Arc<CheckpointManager>,
    pub checkpoint_store: Arc<CheckpointStore>,
    pub task_manager: Arc<TaskManagementTool>,
    pub todo_state: Arc<RwLock<TodoState>>,
    pub skill_loader: Arc<SkillLoader>,
    pub background_manager: Arc<BackgroundManager>,
    pub team_manager: Option<Arc<TeamManager>>,
    pub session_manager: MemorySessionManager,
    /// Shared MemoryManager backing the `memory_add` tool and AutoDream (D1).
    pub memory_manager: Arc<crate::context::MemoryManager>,
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
    /// Secret used to digest viewer bearer tokens.
    daemon_viewer_secret: [u8; 32],
    /// Shared subagent policy-Ask bridge (TUI/daemon drains pending approvals).
    pub permission_bridge: Arc<PermissionBridge>,
    /// Shared root agent permission mode (Yolo/AcceptEdits/Normal).
    /// Updated by the TUI at runtime; subagents snapshot at spawn time.
    pub root_mode: Arc<std::sync::RwLock<RootPermissionMode>>,
    /// Sandbox effective mode (includes Plan). Updated with permission mode;
    /// used when building ToolContext for shell tools.
    pub effective_mode: Arc<std::sync::RwLock<crate::sandbox::EffectiveMode>>,
    /// Shared read connection to the global subagent transcript store, used by
    /// the SSE trace endpoint for cold-start replay. `None` when the store
    /// failed to open at startup (SSE then streams live-only). See design D5.
    pub transcript_store: Option<Arc<crate::transcript::SubagentTranscriptStore>>,
}

impl DaemonState {
    pub async fn new(app_state: AppState) -> Self {
        let task_manager = Arc::new(TaskManagementTool::new());
        let todo_state = Arc::new(RwLock::new(TodoState::default()));
        let policy = ToolPermissionPolicy::from_settings(&app_state.settings);

        // Initialize background manager (shares OS sandbox with shell tools)
        let bg_sandbox = Arc::new(crate::sandbox::SandboxManager::new());
        let bg_manager = Arc::new(BackgroundManager::new().with_sandbox(bg_sandbox));

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
        let coordinator = Arc::new(crate::agent::AgentCoordinator::new(
            app_state.settings.agent.subagent.max_concurrent,
            app_state.settings.agent.subagent.max_depth,
        ));
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
        let shared_session_rules = Arc::new(RwLock::new(HashSet::<String>::new()));
        let root_mode = Arc::new(std::sync::RwLock::new(RootPermissionMode::Normal));
        let effective_mode = Arc::new(std::sync::RwLock::new(
            crate::sandbox::EffectiveMode::Normal,
        ));
        // ── Shared MemoryManager (D1): backs memory_add tool + AutoDream ──
        let memory_manager = Arc::new(crate::context::MemoryManager::with_settings(
            &app_state.settings,
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

            // D1: memory_add tool (backed by shared MemoryManager)
            registry.register(Box::new(crate::tools::meta::MemoryAddTool::new(
                memory_manager.clone(),
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
            .with_effective_mode(effective_mode.clone());
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

        // ── D1: AutoDream startup check (fire-and-forget) ────────────────
        // Replaces the old TUI app-side AutoDream spawn (removed in Task 4).
        // daemon is per-session, so this triggers once per REPL start -
        // equivalent to the old behavior but centralized in the daemon.
        {
            let autodream =
                crate::services::AutoDreamService::new(None, Some(memory_manager.clone()));
            tokio::spawn(async move {
                match autodream.check_and_run().await {
                    Ok(true) => tracing::info!("AutoDream: consolidation triggered"),
                    Ok(false) => tracing::debug!("AutoDream: gate not met, skipped"),
                    Err(e) => tracing::warn!(error = %e, "AutoDream check_and_run failed"),
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

        Self {
            app_state,
            tool_executor,
            tool_registry,
            checkpoint_manager,
            checkpoint_store,
            task_manager,
            todo_state,
            skill_loader,
            background_manager: bg_manager,
            team_manager,
            session_manager,
            memory_manager,
            mcp_manager,
            sessions: Arc::new(RwLock::new(std::collections::HashMap::new())),
            subagent_progress: progress_store,
            subagent_poll_times: Arc::new(RwLock::new(HashMap::new())),
            coordinator,
            capability_service,
            viewer_tokens: Arc::new(RwLock::new(HashMap::new())),
            root_contexts: Arc::new(RwLock::new(HashMap::new())),
            daemon_viewer_secret,
            permission_bridge,
            root_mode,
            effective_mode,
            transcript_store: sse_transcript_store,
            http_client,
            http_client_stream,
        }
    }

    /// Returns the trusted root execution context for `session_id`, creating
    /// it via `ensure_root` on first use. Never accepts agent ID, parent ID, or
    /// depth from request JSON.
    pub async fn root_context(
        &self,
        session_id: &str,
    ) -> anyhow::Result<crate::agent::AgentExecutionContext> {
        {
            let roots = self.root_contexts.read().await;
            if let Some(ctx) = roots.get(session_id) {
                return Ok(ctx.clone());
            }
        }
        let ctx = self
            .coordinator
            .ensure_root(crate::agent::SessionId::new(session_id))
            .await
            .map_err(|e| anyhow::anyhow!("ensure_root failed: {}", e))?;
        let mut roots = self.root_contexts.write().await;
        roots.insert(session_id.to_string(), ctx.clone());
        Ok(ctx)
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

/// Encodes bytes as a lowercase hex string.
fn hex_string(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}
