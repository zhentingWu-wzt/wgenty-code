//! Application main loop — event handling, layout, and daemon lifecycle.

mod continuation;
mod event;
mod event_key;
mod input;
mod render;
mod turn;
pub mod types;

pub use types::*;

use crate::api::ChatMessage;
use crate::prompts::{self, PromptContext};
use crate::runtime::command::CommandRouter;
use crate::runtime::context::ContextAssembler;
use crate::runtime::hooks::HookManager;
use crate::runtime::interaction::InteractionService;
use crate::runtime::interaction_tui::TuiInteractionService;
use crate::state::agent_phase::{AgentPhase, TurnAbortReason, TurnId};
use crate::tui::client::DaemonClient;
use crate::tui::components::input::InputBox;
use crate::tui::components::permission::PermissionState;
use crate::tui::components::plan_panel::PlanPanelState;
use crate::tui::components::question::QuestionState;
use crate::tui::components::session::SessionState;
use crate::tui::components::subagent_focus_view::FocusViewState;
use crate::tui::components::subagent_tree::SubagentTree;
use crate::tui::components::task_panel::TaskPanelState;
use crossterm::event::EnableBracketedPaste;
use ratatui::Terminal;
use std::collections::{HashMap, VecDeque};
use std::io;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::Mutex as TokioMutex;
use tokio::sync::RwLock;

/// A queued turn with separate user-facing text and agent-facing input.
///
/// `continuation` carries a claimed task-group delivery for a synthetic
/// continuation turn. When set, the turn injects the delivered child results
/// as a system message (no visible user row) instead of a user prompt.
#[derive(Debug, Clone)]
pub struct PendingInput {
    pub display_text: String,
    pub agent_input: String,
    pub continuation: Option<crate::tui::client::TaskGroupDeliveryResponse>,
}

impl PendingInput {
    pub fn new(text: String) -> Self {
        Self {
            display_text: text.clone(),
            agent_input: text,
            continuation: None,
        }
    }

    pub fn internal(display_text: String, agent_input: String) -> Self {
        Self {
            display_text,
            agent_input,
            continuation: None,
        }
    }

    /// A synthetic continuation turn that consumes a claimed task-group
    /// delivery. No visible user row is added; the delivery is injected as a
    /// structured system message.
    pub fn continuation(delivery: crate::tui::client::TaskGroupDeliveryResponse) -> Self {
        Self {
            display_text: String::new(),
            agent_input: String::new(),
            continuation: Some(delivery),
        }
    }

    pub fn is_continuation(&self) -> bool {
        self.continuation.is_some()
    }
}

/// Application state for the TUI.
pub struct App {
    pub daemon_client: DaemonClient,
    pub input_box: InputBox,
    pub committed_messages: Vec<UIMessage>,
    pub streaming_content: String,
    pub streaming_active: bool,
    pub token_counter: crate::api::token_counter::TokenCounter,
    pub phase: AgentPhase,
    /// When true, phase-changing events from a just-cancelled turn are ignored
    /// to prevent stale events (e.g. StreamDone, ToolResult) from overriding
    /// the Idle phase back to Thinking. Set by `/clear` and
    /// `cancel_current_turn`; cleared when a new turn starts.
    suppress_phase_updates: bool,
    pub session_id: String,
    pub session_name: String,
    pub last_tool_name: Option<String>,
    pub last_abort_reason: Option<TurnAbortReason>,
    pub scroll_offset: u16,
    pub user_scrolled: bool,
    /// Shared conversation history — all Turns in this session read/write
    /// through this Arc, so each Turn inherits the accumulated context.
    pub conversation_history: Arc<TokioMutex<Vec<ChatMessage>>>,
    /// Serializes session persistence so Turn-complete saves and exit flush
    /// cannot race (last-writer-wins with a stale snapshot).
    session_save_lock: Arc<TokioMutex<()>>,
    /// Set after a successful exit flush. In-flight `spawn_save_session` tasks
    /// observe this under the save lock and skip so they cannot overwrite the
    /// final snapshot with a UI clone taken earlier in the session.
    session_exit_saved: Arc<std::sync::atomic::AtomicBool>,
    /// Pending user inputs queued while a Turn is running.
    pub pending_inputs: VecDeque<PendingInput>,
    /// Handle for the currently executing Turn (None when idle).
    pub current_turn_handle: Option<tokio::task::JoinHandle<()>>,
    /// ID of the currently executing turn (for lifecycle tracking).
    pub current_turn_id: Option<TurnId>,
    /// Coordinator-owned task generation for this session. Incremented on
    /// `/clear`/reset; ready root-direct task groups are claimed at this
    /// generation. Stale-generation deliveries are rejected by the daemon.
    pub agent_generation: u64,
    /// Instant of the last task-group claim poll. Throttles the 100ms Tick
    /// path to a 500ms claim interval so idle polling does not generate
    /// excessive HTTP traffic.
    last_claim_attempt: Option<std::time::Instant>,
    /// Number of completed turns (for UI / debugging).
    pub turn_count: usize,
    pub mode: AgentMode,
    /// Previous mode before entering PlanMode via toggle (Ctrl+P or /plan).
    /// Used to restore the correct mode when toggling back.
    pub previous_mode: Option<AgentMode>,
    /// Pre-assembled system messages (layered instructions from PromptAssembler).
    /// Cloned into each new AgentLoop so every Turn inherits the same base instructions.
    pub assembled_system_messages: Vec<ChatMessage>,
    /// Channel sender for agent/input events
    event_tx: mpsc::UnboundedSender<AppEvent>,
    /// Channel receiver
    event_rx: mpsc::UnboundedReceiver<AppEvent>,
    should_quit: bool,
    pub permission_state: PermissionState,
    pub question_state: QuestionState,
    pub session_state: SessionState,
    pub memory_state: crate::tui::components::memory::MemoryState,
    pub task_panel: TaskPanelState,
    /// Structured plan panel state (Codex-style update_plan tool)
    pub plan_panel_state: PlanPanelState,
    /// Subagent execution tree for the current turn.
    subagent_tree: SubagentTree,
    /// Subagent tree snapshots for completed turns, keyed by turn_id string.
    subagent_history: HashMap<String, SubagentTree>,
    /// Full-screen subagent focus view state (None = not active).
    pub subagent_focus: Option<FocusViewState>,
    /// Selected index in the subagent status bar.
    pub subagent_status_bar_selected: usize,
    /// Scoped agent navigation state: current view frame + back stack.
    pub agent_navigation: crate::tui::app::types::AgentNavigationState,
    /// Whether the status bar has keyboard focus (Tab toggles).
    pub subagent_status_bar_focused: bool,
    pub mouse_capture_enabled: bool,
    pub mouse_capture_toggle: Option<bool>,
    /// Shared settings handle — updated by the config watcher on file change.
    pub settings_lock: crate::config::watcher::SettingsHandle,

    /// Timestamp of last Ctrl+C press for double-press detection
    last_ctrl_c: Option<std::time::Instant>,
    /// True while a tool is executing (for spinner animation)
    pub has_running_tool: bool,
    /// Spinner animation frame (0-9), advanced on Tick when has_running_tool
    pub spinner_frame: u8,
    /// When the current turn started (for elapsed-time display).
    turn_started_at: Option<std::time::Instant>,
    /// Completion timestamps for subagent nodes — used by the focus view
    /// selector to dim completed subagents and remove them after a delay.
    /// Cleared on a new turn (Submit).
    pub completed_at: HashMap<String, std::time::Instant>,
    /// Cancellation flag for blocking input reader task
    shutdown_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,

    /// Completion engine for @ and / auto-completion.
    pub completion_engine: Option<crate::tui::completion::CompletionEngine>,
    /// Current completion state (None = no active completion).
    pub completion_state: Option<CompletionState>,
    /// Shared, optional transcript store for browsing completed subagent records.
    pub transcript_store: Option<std::sync::Arc<crate::transcript::SubagentTranscriptStore>>,
    /// External skill registry for resolving slash commands.
    pub external_skill_registry: Option<std::sync::Arc<crate::knowledge::ExternalSkillRegistry>>,
    /// Hook manager for lifecycle event hooks (SessionStart, Stop, etc.).
    pub hook_manager: std::sync::Arc<HookManager>,
    /// Prompt context shared with each AgentLoop for per-turn reminder construction.
    pub prompt_context: std::sync::Arc<PromptContext>,
    /// Memory manager for cross-session memory (extraction, storage, recall, consolidation).
    pub memory_manager: std::sync::Arc<crate::context::MemoryManager>,
    /// Memories recalled at session startup; injected into compact turns' PromptContext.
    pub(crate) startup_memories: Vec<String>,
    /// CodeGraph availability (sync probe result), refreshed from settings.
    pub codegraph_status: crate::mcp::codegraph::CodegraphInstallState,
    /// Sticky session flag: shell ran outside OS sandbox (degrade / disabled).
    /// Cleared on `/clear` / new session reset paths that rebuild App state.
    pub sandbox_bypassed_session: bool,
    /// Command router for slash command dispatch (replaces Comet-specific routing).
    pub command_router: Option<CommandRouter>,
    /// Interaction service for runtime user interaction (ask, confirm).
    pub interaction_service: Option<Arc<dyn InteractionService>>,
    /// Shared workflow state handle (e.g. Comet phase: "open", "design", "build").
    pub workflow_state: Option<Arc<RwLock<String>>>,
}

impl App {
    pub fn new(
        daemon_client: DaemonClient,
        session_id: String,
        settings_lock: crate::config::watcher::SettingsHandle,
    ) -> Self {
        // One-time legacy session migration (idempotent via marker file).
        crate::context::migration::migrate_legacy_sessions();

        let (event_tx, event_rx) = mpsc::unbounded_channel();
        // Build layered instructions from settings + context
        let settings = {
            let guard = settings_lock.read().expect("lock poisoned: settings");
            guard.clone()
        };
        // Initial mode mirrors settings.agent.plan_mode (same as Self fields below).
        let initial_mode = if settings.agent.plan_mode {
            AgentMode::PlanMode
        } else {
            AgentMode::Normal
        };
        let prompt_ctx = PromptContext::new()
            .with_cwd(
                std::env::current_dir()
                    .unwrap_or_else(|_| std::path::PathBuf::from("."))
                    .display()
                    .to_string(),
            )
            .with_shell(std::env::var("SHELL").unwrap_or_else(|_| "sh".to_string()))
            .with_sandbox(initial_mode.prompt_sandbox_mode())
            .with_approval(initial_mode.prompt_approval_policy());
        let prompt_ctx = prompt_ctx.with_collaboration(
            settings
                .prompt
                .collaboration_mode
                .clone()
                .unwrap_or_default(),
        );
        let prompt_ctx =
            prompt_ctx.with_codegraph_state(crate::mcp::codegraph::probe_install_state(&settings));

        // Cache user global instructions & rules for Layers 7/8 in system prompt
        let prompt_ctx = prompt_ctx
            .with_user_global_instructions(crate::utils::project::read_user_global_instructions());
        let prompt_ctx =
            prompt_ctx.with_user_global_rules(crate::utils::project::read_user_global_rules());
        // Skill discovery (SkillLoader + ExternalSkillRegistry) involves
        // synchronous disk I/O that can take 50-200ms on systems with many
        // skills. It is spawned in a background blocking task and delivered
        // via AppEvent::SkillsReady so it never delays the first rendered
        // frame. The App starts with an empty inventory; the event handler
        // re-assembles the system prompt when results arrive.
        let prompt_ctx = prompt_ctx.with_skills(Vec::new());

        // ── Generic Agent Runtime: CommandRouter (workflow.yaml deferred) ─
        let builtin_commands = crate::tui::completion::CompletionEngine::default_builtin_commands();
        let builtin_command_names: Vec<String> =
            builtin_commands.iter().map(|c| c.name.clone()).collect();
        let command_router = CommandRouter::new(builtin_command_names);

        let context_assembler: Option<Arc<ContextAssembler>> = None;
        let workflow_state: Option<Arc<RwLock<String>>> = None;
        let external_skill_registry: Option<crate::knowledge::ExternalSkillRegistry> = None;

        // Spawn background skill discovery + workflow.yaml parsing
        {
            let tx = event_tx.clone();
            tokio::task::spawn_blocking(move || {
                let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
                let skills_dirs = vec![home.join(".wgenty-code").join("skills")];
                let skill_loader =
                    crate::knowledge::loader::SkillLoader::load_from_dirs(&skills_dirs);
                let mut skill_inventory: Vec<prompts::SkillEntry> = Vec::new();
                for name in skill_loader.skill_names() {
                    if !crate::knowledge::should_expose_skill_by_default(&name) {
                        continue;
                    }
                    if let Some(skill) = skill_loader.load_skill(&name) {
                        skill_inventory.push(prompts::SkillEntry {
                            name,
                            description: skill.description.clone(),
                        });
                    }
                }

                let project_root =
                    std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
                let external_registry_roots =
                    crate::knowledge::SkillRootResolver::roots_with(&home, &project_root);
                let root_count = external_registry_roots.len();
                let external_registry =
                    crate::knowledge::ExternalSkillRegistry::discover(external_registry_roots).ok();
                if let Some(ref reg) = external_registry {
                    for skill_def in reg.list() {
                        if !crate::knowledge::should_expose_skill_by_default(
                            &skill_def.canonical_name,
                        ) {
                            continue;
                        }
                        if !skill_inventory
                            .iter()
                            .any(|s| s.name == skill_def.canonical_name)
                        {
                            skill_inventory.push(prompts::SkillEntry {
                                name: skill_def.canonical_name.clone(),
                                description: skill_def.description.clone(),
                            });
                        }
                    }
                    tracing::info!(
                        total_skills = skill_inventory.len(),
                        root_count,
                        "Skill registry initialized with external skills"
                    );
                } else {
                    tracing::info!(
                        total_skills = skill_inventory.len(),
                        root_count,
                        "Skill registry initialized (no external skills discovered)"
                    );
                }

                // Parse comet entry commands from workflow.yaml, falling back
                // to the external registry if workflow.yaml is absent.
                let mut comet_entry_commands: Vec<String> = Vec::new();
                let workflow_yaml_path =
                    std::path::PathBuf::from(".wgenty-code/skills/comet/workflow.yaml");
                if workflow_yaml_path.exists() {
                    if let Ok(content) = std::fs::read_to_string(&workflow_yaml_path) {
                        let entry_cmds = parse_yaml_list(&content, "entry_commands:");
                        if !entry_cmds.is_empty() {
                            comet_entry_commands = entry_cmds;
                        }
                    }
                }
                if comet_entry_commands.is_empty() {
                    if let Some(ref reg) = external_registry {
                        comet_entry_commands = reg
                            .list()
                            .iter()
                            .filter(|s| s.canonical_name.starts_with("comet"))
                            .map(|s| s.canonical_name.clone())
                            .collect();
                    }
                }

                let _ = tx.send(AppEvent::SkillsReady(Box::new(SkillsReadyData {
                    skill_inventory,
                    external_skill_registry: external_registry.map(std::sync::Arc::new),
                    comet_entry_commands,
                })));
            });
        }

        // Load WGENTY.md and AGENTS.md sections from project root
        let project_root =
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let wgenty_sections = crate::utils::project::read_wgenty_md_sections(&project_root);
        let agents_sections = crate::utils::project::read_agents_md_sections(&project_root);
        crate::utils::startup_timing::mark("app new: wgenty/agents sections read");

        // Static project/user instruction files live in the system cascade
        // (Layers 7–10). Per-turn <system-reminder> is hook-only and starts empty.
        let mut prompt_ctx = prompt_ctx
            .with_wgenty_md(wgenty_sections)
            .with_agents_md(agents_sections)
            .with_project_root(project_root.clone());

        // Inject context assembler from workflow config (Generic Agent Runtime)
        if let Some(assembler) = context_assembler.clone() {
            prompt_ctx.context_assembler = Some(assembler);
        }

        let assembled = prompts::assemble_instructions(&settings, &prompt_ctx);
        crate::utils::startup_timing::mark("app new: prompt assembled");
        let system_messages = assembled.system_messages;
        // Dialogue-only history: system layers are prepended each API round and
        // must not be seeded into (or duplicated by) conversation_history.
        let conversation_history = Arc::new(TokioMutex::new(Vec::new()));
        // Share the prompt context with each AgentLoop so per-turn reminders
        // can inject hook fragments. Project/user instruction files are cached
        // at construction and delivered via the system cascade.
        let prompt_context = Arc::new(prompt_ctx);

        // Initialize hook manager from settings
        let hook_manager = {
            let hooks_config = settings
                .integrations
                .hooks
                .as_ref()
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            std::sync::Arc::new(HookManager::from_settings(&hooks_config))
        };

        // Set up TUI interaction service
        let (interaction_tx, _interaction_rx) = mpsc::unbounded_channel::<String>();
        let (_answer_tx, answer_rx) =
            mpsc::unbounded_channel::<crate::runtime::interaction::UserAnswer>();
        let interaction_service: Option<Arc<dyn InteractionService>> = Some(Arc::new(
            TuiInteractionService::new(interaction_tx, answer_rx),
        ));

        // Fire SessionStart hook asynchronously (non-blocking)
        {
            let hm = hook_manager.clone();
            let sid = session_id.clone();
            tokio::spawn(async move {
                let ctx = HookManager::session_start_context(&sid);
                hm.fire(
                    &crate::runtime::hooks::HookEvent::SessionStart,
                    &ctx,
                    None,
                    None,
                )
                .await;
            });
        }

        // ── Memory manager (for cross-session recall + injection) ────────
        // Configured from settings so consolidation thresholds are tunable
        // via `storage.memory` in settings.json.
        let mm = Arc::new(crate::context::MemoryManager::with_settings(
            &settings,
            crate::utils::current_project_root(),
        ));

        // ── Detect CodeGraph MCP status from settings ─────────────────────
        let codegraph_status = detect_codegraph_status(&settings);

        let app = Self {
            daemon_client,
            input_box: InputBox::new(),
            committed_messages: Vec::new(),
            streaming_content: String::new(),
            streaming_active: false,
            token_counter: crate::api::token_counter::TokenCounter::new(),
            phase: AgentPhase::Idle,
            suppress_phase_updates: false,
            session_id,
            session_name: "New Session".to_string(),
            last_tool_name: None,
            last_abort_reason: None,
            scroll_offset: 0,
            user_scrolled: false,
            conversation_history,
            session_save_lock: Arc::new(TokioMutex::new(())),
            session_exit_saved: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            assembled_system_messages: system_messages,
            pending_inputs: VecDeque::new(),
            current_turn_handle: None,
            current_turn_id: None,
            agent_generation: 0,
            last_claim_attempt: None,
            turn_count: 0,
            mode: if settings.agent.plan_mode {
                AgentMode::PlanMode
            } else {
                AgentMode::Normal
            },
            previous_mode: None,
            event_tx,
            event_rx,
            should_quit: false,
            permission_state: PermissionState::new(),
            question_state: QuestionState::new(),
            session_state: SessionState::new(),
            memory_state: crate::tui::components::memory::MemoryState::new(),
            task_panel: TaskPanelState::new(),
            plan_panel_state: PlanPanelState::new(),
            subagent_tree: SubagentTree::default(),
            subagent_history: HashMap::new(),
            subagent_focus: None,
            subagent_status_bar_selected: 0,
            agent_navigation: crate::tui::app::types::AgentNavigationState::default(),
            subagent_status_bar_focused: false,
            mouse_capture_enabled: true,
            mouse_capture_toggle: None,

            last_ctrl_c: None,
            has_running_tool: false,
            spinner_frame: 0,
            turn_started_at: None,
            completed_at: HashMap::new(),
            shutdown_flag: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),

            settings_lock,

            completion_engine: {
                let builtin_commands =
                    crate::tui::completion::CompletionEngine::default_builtin_commands();
                let mut engine = crate::tui::completion::CompletionEngine::load(&builtin_commands);
                // Merge workflow entry commands from CommandRouter into completion
                let entry_cmds = command_router.entry_commands();
                engine.merge_from_entry_commands(&entry_cmds);
                Some(engine)
            },
            completion_state: None,
            transcript_store: {
                let db_path_str = &settings.storage.transcript.db_path;
                let db_path = std::path::PathBuf::from(db_path_str);
                match crate::transcript::SubagentTranscriptStore::open(&db_path) {
                    Ok(store) => Some(std::sync::Arc::new(store)),
                    Err(e) => {
                        tracing::warn!("Failed to open transcript store at {}: {}. Running without persistence.", db_path.display(), e);
                        None
                    }
                }
            },
            external_skill_registry: external_skill_registry.map(std::sync::Arc::new),
            hook_manager,
            prompt_context,
            memory_manager: mm,
            startup_memories: Vec::new(),
            codegraph_status,
            sandbox_bypassed_session: false,
            command_router: Some(command_router),
            interaction_service,
            workflow_state,
        };
        app
    }

    pub fn event_sender(&self) -> mpsc::UnboundedSender<AppEvent> {
        self.event_tx.clone()
    }

    /// Max time to wait for the final session flush on exit.
    /// Local daemon + JSON write is typically tens of ms; 3s covers a
    /// momentarily busy daemon without making Ctrl+C feel stuck.
    const EXIT_SAVE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

    /// Snapshot current history + UI track and persist under `session_save_lock`.
    ///
    /// Exit path only. After a successful write, sets `session_exit_saved` so
    /// any earlier fire-and-forget save still waiting on the lock drops its
    /// stale UI clone instead of overwriting the final snapshot.
    pub(super) async fn save_session_snapshot(&self) {
        let id = self.session_id.clone();
        let name = self.session_name.clone();
        let client = self.daemon_client.clone();
        let history = self.conversation_history.clone();
        let ui_messages: Vec<_> = self
            .committed_messages
            .iter()
            .map(UIMessage::to_session_ui_message)
            .collect();
        let lock = self.session_save_lock.clone();
        let exit_saved = self.session_exit_saved.clone();

        let _guard = lock.lock().await;
        // Sanitize under the save lock so interrupt/exit never persist unpaired
        // tool_calls (idempotent when history is already well-formed).
        let h = {
            let mut hist = history.lock().await;
            crate::api::types::sanitize_tool_call_pairing(&mut hist);
            hist.clone()
        };
        match client.save_session(&id, &name, &h, &ui_messages).await {
            Ok(()) => {
                exit_saved.store(true, std::sync::atomic::Ordering::Release);
            }
            Err(e) => {
                tracing::error!(
                    session_id = %id,
                    session_name = %name,
                    error = %e,
                    "Failed to save session to daemon"
                );
            }
        }
    }

    /// Non-blocking save for TurnComplete / interrupt paths.
    ///
    /// Clones the UI track at schedule time; history is re-read + sanitized
    /// under the lock. Skips the write if exit flush already persisted the
    /// final snapshot — preventing last-writer-wins with a stale UI clone.
    ///
    /// If exit flush *timed out* without setting the flag, an in-flight spawn
    /// still writes (best-effort) rather than dropping the only remaining save.
    pub(super) fn spawn_save_session(&self) {
        let id = self.session_id.clone();
        let name = self.session_name.clone();
        let client = self.daemon_client.clone();
        let history = self.conversation_history.clone();
        let ui_messages: Vec<_> = self
            .committed_messages
            .iter()
            .map(UIMessage::to_session_ui_message)
            .collect();
        let lock = self.session_save_lock.clone();
        let exit_saved = self.session_exit_saved.clone();
        tokio::spawn(async move {
            let _guard = lock.lock().await;
            if exit_saved.load(std::sync::atomic::Ordering::Acquire) {
                tracing::debug!(
                    session_id = %id,
                    "skipping spawned session save; exit flush already persisted"
                );
                return;
            }
            let h = {
                let mut hist = history.lock().await;
                crate::api::types::sanitize_tool_call_pairing(&mut hist);
                hist.clone()
            };
            if let Err(e) = client.save_session(&id, &name, &h, &ui_messages).await {
                tracing::error!(
                    session_id = %id,
                    session_name = %name,
                    error = %e,
                    "Failed to save session to daemon"
                );
            }
        });
    }

    /// Best-effort final persist before process teardown.
    ///
    /// Waits up to [`Self::EXIT_SAVE_TIMEOUT`]. If a TurnComplete save is still
    /// holding the lock, we queue behind it then write the latest snapshot so
    /// the on-disk file matches the UI the user just left.
    async fn flush_session_on_exit(&self) {
        match tokio::time::timeout(Self::EXIT_SAVE_TIMEOUT, self.save_session_snapshot()).await {
            Ok(()) => {
                tracing::debug!(session_id = %self.session_id, "exit session flush completed");
            }
            Err(_) => {
                tracing::warn!(
                    session_id = %self.session_id,
                    timeout_secs = Self::EXIT_SAVE_TIMEOUT.as_secs(),
                    "exit session flush timed out; continuing shutdown"
                );
            }
        }
    }

    /// Run the main event loop.
    pub async fn run<B: ratatui::backend::Backend + std::marker::Unpin>(
        &mut self,
        terminal: &mut Terminal<B>,
    ) -> anyhow::Result<()> {
        crossterm::execute!(io::stdout(), EnableBracketedPaste).ok();
        // Spawn input reader task (blocking crossterm event::read)
        let tx = self.event_tx.clone();
        let shutdown = self.shutdown_flag.clone();
        tokio::task::spawn_blocking(move || {
            let _ = super::input_reader::read_input(tx, shutdown);
        });
        // Spawn ticker for periodic refresh
        let tx = self.event_tx.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(100));
            loop {
                interval.tick().await;
                if tx.send(AppEvent::Tick).is_err() {
                    break;
                }
            }
        });

        // Render the first frame IMMEDIATELY so the user sees the UI before any
        // startup background work runs. Cross-session memory recall is spawned
        // below and delivers its results via events - it never blocks the
        // first paint. (AutoDream consolidation is handled by the daemon, D1.)
        terminal.draw(|f| self.render(f))?;
        crate::utils::startup_timing::mark("first frame rendered (UI ready)");

        // ── Background startup: recall cross-session memories ─────────────
        // Loads + searches memories off the render path; formatted results are
        // delivered via `MemoriesReady` so they populate `startup_memories`
        // without blocking the first frame (arriving one tick later).
        {
            let mm = self.memory_manager.clone();
            let tx = self.event_tx.clone();
            tokio::spawn(async move {
                if let Err(e) = mm.load().await {
                    tracing::warn!(
                        error = %e,
                        "failed to load memories at session startup; recall skipped"
                    );
                    return;
                }
                let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
                let project_name = cwd
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "unknown".to_string());
                let matched = mm.search_memories(&project_name).await;
                let lines = format_startup_memories(&matched);
                if !lines.is_empty() {
                    tracing::info!(
                        count = lines.len(),
                        "recalled cross-session memories at startup"
                    );
                    let _ = tx.send(AppEvent::MemoriesReady(lines));
                }

                // Format global memories for the system prompt <global-memory>
                // block. Unlike project memories, these are injected every
                // turn without relevance filtering (soft cap 50).
                let global_lines =
                    crate::context::inject::MemoryContextInjector::format_global(&mm).await;
                if !global_lines.is_empty() {
                    tracing::info!(
                        count = global_lines.len(),
                        "loaded global memories at startup"
                    );
                    let _ = tx.send(AppEvent::GlobalMemoriesReady(global_lines));
                }
            });
        }

        // Main loop
        while !self.should_quit {
            // Process pending events
            while let Ok(event) = self.event_rx.try_recv() {
                self.handle_event(event).await;
                if self.should_quit {
                    break;
                }
            }
            if let Some(enable) = self.mouse_capture_toggle.take() {
                use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
                use crossterm::execute;
                let mut stdout = std::io::stdout();
                let _ = if enable {
                    execute!(stdout, EnableMouseCapture)
                } else {
                    execute!(stdout, DisableMouseCapture)
                };
            }

            terminal.draw(|f| self.render(f))?;
            // Block until next event (prevents busy-waiting)
            if let Some(event) = self.event_rx.recv().await {
                self.handle_event(event).await;
            }
        }

        // Persist the latest transcript before tearing down the daemon session.
        // Bounded wait — see EXIT_SAVE_TIMEOUT — so a hung daemon cannot block
        // Ctrl+C indefinitely.
        self.flush_session_on_exit().await;

        // Cancel the agent session through the coordinator so no subagent
        // outlives the TUI: live root-direct subtrees are cancelled bottom-up
        // and their permits released. Best-effort: shutdown proceeds even if
        // the daemon is unreachable.
        {
            let client = self.daemon_client.clone();
            let sid = self.session_id.clone();
            let handle = tokio::spawn(async move {
                let _ = client.cancel_agent_session(&sid).await;
            });
            let _ = tokio::time::timeout(std::time::Duration::from_secs(5), handle).await;
        }

        // Fire SessionEnd hook before exit; wait up to 5s for it to complete
        {
            let hm = self.hook_manager.clone();
            let sid = self.session_id.clone();
            let handle = tokio::spawn(async move {
                let ctx = HookManager::session_end_context(&sid);
                hm.fire(
                    &crate::runtime::hooks::HookEvent::SessionEnd,
                    &ctx,
                    None,
                    None,
                )
                .await;
            });
            let _ = tokio::time::timeout(std::time::Duration::from_secs(5), handle).await;
        }

        Ok(())
    }
}

/// Parse a simple YAML list from text.
/// Looks for lines like "key:" followed by "  - value" entries.
fn parse_yaml_list(text: &str, key: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut in_section = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed == key {
            in_section = true;
            continue;
        }
        if in_section {
            if let Some(value) = trimmed.strip_prefix("- ") {
                result.push(value.trim().to_string());
            } else if !trimmed.starts_with('-') && !trimmed.is_empty() {
                // No longer in the list section
                break;
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_memory_type_returns_human_readable_labels() {
        use crate::context::MemoryType;
        assert_eq!(format_memory_type(&MemoryType::Decision), "decision");
        assert_eq!(format_memory_type(&MemoryType::Error), "error");
        assert_eq!(format_memory_type(&MemoryType::Preference), "preference");
        assert_eq!(format_memory_type(&MemoryType::Insight), "insight");
        assert_eq!(format_memory_type(&MemoryType::Knowledge), "knowledge");
        assert_eq!(format_memory_type(&MemoryType::Task), "task");
        assert_eq!(format_memory_type(&MemoryType::Session), "session");
        assert_eq!(
            format_memory_type(&MemoryType::Conversation),
            "conversation"
        );
    }

    #[test]
    fn test_parse_yaml_list_basic() {
        let yaml = "entry_commands:\n  - comet\n  - comet-open\n  - comet-build\n";
        let result = parse_yaml_list(yaml, "entry_commands:");
        assert_eq!(result, vec!["comet", "comet-open", "comet-build"]);
    }

    #[test]
    fn test_parse_yaml_list_empty_section() {
        let yaml = "entry_commands:\nname: test\n  - comet\n";
        let result = parse_yaml_list(yaml, "entry_commands:");
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_yaml_list_missing_key() {
        let yaml = "name: comet\nhooks:\n  - some_hook\n";
        let result = parse_yaml_list(yaml, "entry_commands:");
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_yaml_list_values_trimmed() {
        let yaml = "entry_commands:\n  -   comet  \n  -  comet-open \n";
        let result = parse_yaml_list(yaml, "entry_commands:");
        assert_eq!(result, vec!["comet", "comet-open"]);
    }

    #[test]
    fn format_startup_memories_filters_below_threshold() {
        use crate::context::{MemoryEntry, MemoryType};
        let mems = vec![
            MemoryEntry::new(MemoryType::Decision, "keep me").with_importance(0.8),
            MemoryEntry::new(MemoryType::Error, "drop me").with_importance(0.3),
        ];
        let lines = format_startup_memories(&mems);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("keep me"));
    }

    #[test]
    fn format_startup_memories_sorts_by_importance_desc() {
        use crate::context::{MemoryEntry, MemoryType};
        let mems = vec![
            MemoryEntry::new(MemoryType::Knowledge, "low").with_importance(0.5),
            MemoryEntry::new(MemoryType::Decision, "high").with_importance(0.9),
            MemoryEntry::new(MemoryType::Insight, "mid").with_importance(0.7),
        ];
        let lines = format_startup_memories(&mems);
        assert_eq!(lines.len(), 3);
        assert!(lines[0].contains("high"));
        assert!(lines[1].contains("mid"));
        assert!(lines[2].contains("low"));
    }

    #[test]
    fn format_startup_memories_limits_to_top_n() {
        use crate::context::{MemoryEntry, MemoryType};
        let mems: Vec<_> = (0..10)
            .map(|i| {
                MemoryEntry::new(MemoryType::Knowledge, &format!("m{}", i))
                    .with_importance(0.5 + i as f32 * 0.01)
            })
            .collect();
        let lines = format_startup_memories(&mems);
        assert_eq!(lines.len(), STARTUP_MEMORY_TOP_N);
    }

    #[test]
    fn format_startup_memories_empty_input() {
        let lines: Vec<String> = format_startup_memories(&[]);
        assert!(lines.is_empty());
    }
}

#[cfg(test)]
mod token_budget_tests {
    use std::path::Path;

    /// Synthesize a long string that pushes the reminder past 2000-token threshold.
    fn long_section(target_chars: usize) -> String {
        "lorem ipsum dolor sit amet ".repeat(target_chars / 27 + 1)
    }

    /// Scope a fake `$HOME` so reminder readers don't pick up the developer's
    /// real `~/.wgenty-code/` files (which would skew the estimate and could
    /// cause flaky failures in the under-threshold test).
    fn with_fake_home<F: FnOnce() -> R, R>(home: &Path, f: F) -> R {
        let prev = std::env::var_os("HOME");
        std::env::set_var("HOME", home);
        let result = f();
        match prev {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        result
    }

    #[test]
    #[serial_test::serial]
    fn reminder_is_hook_only_project_docs_do_not_inflate_user_turn() {
        // Static WGENTY content no longer rides the per-turn reminder channel.
        // A huge project section alone must not produce a user-turn reminder.
        let tmp = tempfile::TempDir::new().unwrap();
        let huge = long_section(12_000);
        let preview_ctx = crate::prompts::PromptContext::new().with_wgenty_md(vec![huge]);

        let reminder = with_fake_home(tmp.path(), || {
            crate::prompts::build_user_turn_reminder(&preview_ctx, &[])
        });

        assert!(
            reminder.is_none(),
            "project docs must not produce a per-turn system-reminder without hooks"
        );
    }

    #[test]
    #[serial_test::serial]
    fn reminder_with_hook_injection_is_present() {
        use crate::runtime::hooks::{InjectedFragment, LayerVisibility};

        let tmp = tempfile::TempDir::new().unwrap();
        let preview_ctx = crate::prompts::PromptContext::new();
        let hooks = vec![InjectedFragment {
            content: "hook body".into(),
            source_label: "test-hook".into(),
            visibility: LayerVisibility::Internal,
            priority: 50,
        }];

        let reminder = with_fake_home(tmp.path(), || {
            crate::prompts::build_user_turn_reminder(&preview_ctx, &hooks)
        })
        .expect("hook injection must produce a reminder");

        assert!(reminder.to_model.contains("<system-reminder>"));
        assert!(reminder.to_model.contains("hook body"));
        assert!(reminder.to_transcript.is_none());
    }

    /// Regression guard for the channel bug introduced in commit 006945f and
    /// fixed in `fix-token-budget-warning-channel`: the over-threshold budget
    /// warning MUST be dev-log-only — `App::new` must NOT construct a
    /// user-visible notice to push into `committed_messages`.
    ///
    /// `App::new` is too heavy to construct in a unit test, so this guard
    /// asserts a source-level invariant: the removed user-visible notice
    /// constructor binding is absent from this module. If a future change
    /// reintroduces a user-visible budget notice, this test fails and forces
    /// a spec conversation — `system-reminder-injection` mandates dev-log-only.
    #[test]
    fn budget_warning_is_dev_log_only_no_user_visible_notice() {
        let src = include_str!("mod.rs");
        // Assemble the forbidden identifier from fragments so this guard's
        // own source text does not self-match.
        let forbidden = format!("{}_{}", "token_budget", "notice");
        assert!(
            !src.contains(&forbidden),
            "user-visible budget notice binding reintroduced — spec \
             system-reminder-injection mandates dev-log-only; do not push a \
             notice into committed_messages"
        );
        // The dev-log path must still exist.
        assert!(
            src.contains("tracing::warn!"),
            "dev-facing tracing::warn! for budget warning was removed"
        );
    }
}

pub use super::util::agent_phase_from_event;
pub use super::util::centered_rect;
pub use super::util::compute_collapse_state;
pub use super::util::extract_diff_data;
pub use super::util::extract_tool_metadata;
pub use super::util::format_tool_result;
pub use super::util::split_unified_diff;
pub use super::util::start_daemon;
pub use super::util::tool_label;
/// Truncate a user message to a short session name (max ~50 chars, no newlines).
pub use super::util::truncate_session_name;

/// Format a MemoryType variant as a short human-readable string.
fn format_memory_type(mt: &crate::context::MemoryType) -> &'static str {
    match mt {
        crate::context::MemoryType::Decision => "decision",
        crate::context::MemoryType::Error => "error",
        crate::context::MemoryType::Preference => "preference",
        crate::context::MemoryType::Insight => "insight",
        crate::context::MemoryType::Knowledge => "knowledge",
        crate::context::MemoryType::Task => "task",
        crate::context::MemoryType::Session => "session",
        crate::context::MemoryType::Conversation => "conversation",
    }
}

/// Minimum importance for a memory to be surfaced at startup recall.
const STARTUP_MEMORY_MIN_IMPORTANCE: f32 = 0.5;
/// Maximum number of memories recalled at startup.
const STARTUP_MEMORY_TOP_N: usize = 5;

/// Filter, rank, and format cross-session memories for startup recall.
///
/// Extracted as a pure function so the selection logic (importance threshold,
/// descending sort, top-N cap, line formatting) is unit-testable independently
/// of the async memory manager. Returns the formatted lines (possibly empty).
fn format_startup_memories(matched: &[crate::context::MemoryEntry]) -> Vec<String> {
    let mut important: Vec<&crate::context::MemoryEntry> = matched
        .iter()
        .filter(|m| m.importance >= STARTUP_MEMORY_MIN_IMPORTANCE)
        .collect();
    // Sort by importance descending (stable on equal importance).
    important.sort_by(|a, b| {
        b.importance
            .partial_cmp(&a.importance)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    important
        .into_iter()
        .take(STARTUP_MEMORY_TOP_N)
        .map(|m| {
            format!(
                "- [{}] {} (importance: {:.1})",
                format_memory_type(&m.memory_type),
                m.content,
                m.importance
            )
        })
        .collect()
}

/// Detect the CodeGraph MCP server status from settings.
pub fn detect_codegraph_status(
    settings: &crate::config::Settings,
) -> crate::mcp::codegraph::CodegraphInstallState {
    crate::mcp::codegraph::probe_install_state(settings)
}
