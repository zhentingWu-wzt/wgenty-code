//! Application main loop — event handling, layout, and daemon lifecycle.

mod event;
mod input;
mod render;
mod turn;
pub mod types;

pub use types::*;

use crate::api::ChatMessage;
use crate::prompts::{self, PromptContext};
use crate::state::agent_phase::{AgentPhase, TurnAbortReason, TurnId};
use crate::tui::client::DaemonClient;
use crate::tui::components::input::InputBox;
use crate::tui::components::permission::PermissionState;
use crate::tui::components::plan_panel::PlanPanelState;
use crate::tui::components::question::QuestionState;
use crate::tui::components::session::SessionState;
use crate::tui::components::subagent_panel_state::SubagentPanelState;
use crate::tui::components::subagent_tree::SubagentTree;
use crate::tui::components::task_panel::TaskPanelState;
use crossterm::event::EnableBracketedPaste;
use ratatui::Terminal;
use std::collections::{HashMap, VecDeque};
use std::io;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::Mutex as TokioMutex;

/// Application state for the TUI.
pub struct App {
    pub daemon_client: DaemonClient,
    pub input_box: InputBox,
    pub committed_messages: Vec<UIMessage>,
    pub streaming_content: String,
    pub streaming_active: bool,
    pub token_counter: crate::api::token_counter::TokenCounter,
    pub phase: AgentPhase,
    pub session_id: String,
    pub session_name: String,
    pub last_tool_name: Option<String>,
    pub last_abort_reason: Option<TurnAbortReason>,
    pub scroll_offset: u16,
    pub user_scrolled: bool,
    /// Shared conversation history — all Turns in this session read/write
    /// through this Arc, so each Turn inherits the accumulated context.
    pub conversation_history: Arc<TokioMutex<Vec<ChatMessage>>>,
    /// Pending user inputs queued while a Turn is running.
    pub pending_inputs: VecDeque<String>,
    /// Handle for the currently executing Turn (None when idle).
    pub current_turn_handle: Option<tokio::task::JoinHandle<()>>,
    /// ID of the currently executing turn (for lifecycle tracking).
    pub current_turn_id: Option<TurnId>,
    /// Number of completed turns (for UI / debugging).
    pub turn_count: usize,
    pub mode: AgentMode,
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
    pub task_panel: TaskPanelState,
    /// Structured plan panel state (Codex-style update_plan tool)
    pub plan_panel_state: PlanPanelState,
    /// Subagent execution tree for the current turn.
    subagent_tree: SubagentTree,
    /// Subagent tree snapshots for completed turns, keyed by turn_id string.
    subagent_history: HashMap<String, SubagentTree>,
    /// Whether the subagent monitor panel is visible.
    subagent_panel_visible: bool,
    /// Interactive state for the subagent monitor panel.
    pub subagent_panel_state: SubagentPanelState,
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
    /// Cancellation flag for blocking input reader task
    shutdown_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,

    /// Completion engine for @ and / auto-completion.
    pub completion_engine: Option<crate::tui::completion::CompletionEngine>,
    /// Current completion state (None = no active completion).
    pub completion_state: Option<CompletionState>,
    /// Shared, optional transcript store for browsing completed subagent records.
    pub transcript_store: Option<std::sync::Arc<crate::transcript::SubagentTranscriptStore>>,
}

impl App {
    pub fn new(
        daemon_client: DaemonClient,
        session_id: String,
        settings_lock: crate::config::watcher::SettingsHandle,
    ) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        // Build layered instructions from settings + context
        let prompt_ctx = PromptContext::new()
            .with_cwd(
                std::env::current_dir()
                    .unwrap_or_else(|_| std::path::PathBuf::from("."))
                    .display()
                    .to_string(),
            )
            .with_shell(std::env::var("SHELL").unwrap_or_else(|_| "sh".to_string()))
            .with_sandbox("workspace-write")
            .with_approval("never");
        let settings = {
            let guard = settings_lock.read().unwrap();
            guard.clone()
        };
        let prompt_ctx =
            prompt_ctx.with_collaboration(settings.prompt.collaboration_mode.clone().unwrap_or_default());

        // Load skills inventory for system prompt injection
        let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
        let skills_dirs = vec![home.join(".wgenty-code").join("skills")];
        let skill_loader = crate::knowledge::loader::SkillLoader::load_from_dirs(&skills_dirs);
        let mut skill_inventory: Vec<prompts::SkillEntry> = Vec::new();
        for name in skill_loader.skill_names() {
            if let Some(skill) = skill_loader.load_skill(&name) {
                let desc = skill.description.clone();
                skill_inventory.push(prompts::SkillEntry {
                    name,
                    description: desc,
                });
            }
        }
        let prompt_ctx = prompt_ctx.with_skills(skill_inventory);

        // Load WGENTY.md and AGENTS.md sections from project root
        let project_root =
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let wgenty_sections = crate::utils::project::read_wgenty_md_sections(&project_root);
        let agents_sections = crate::utils::project::read_agents_md_sections(&project_root);

        // Warn if WGENTY.md + AGENTS.md exceed token budget (fires once per session)
        let wgenty_tokens: usize = wgenty_sections
            .iter()
            .map(|s| crate::utils::estimate_tokens(s))
            .sum();
        let agents_tokens: usize = agents_sections
            .iter()
            .map(|s| crate::utils::estimate_tokens(s))
            .sum();
        let total_md_tokens = wgenty_tokens + agents_tokens;
        if total_md_tokens > 2000 {
            tracing::warn!(
                wgenty_tokens,
                agents_tokens,
                total = total_md_tokens,
                "WGENTY.md + AGENTS.md sections estimate ~{} tokens ({} + {}). \
                 Consider trimming to keep session startup lean.",
                total_md_tokens,
                wgenty_tokens,
                agents_tokens,
            );
        }

        let prompt_ctx = prompt_ctx
            .with_wgenty_md(wgenty_sections)
            .with_agents_md(agents_sections);

        let assembled = prompts::assemble_instructions(&settings, &prompt_ctx);
        let system_messages = assembled.system_messages;
        let conversation_history = Arc::new(TokioMutex::new(system_messages.clone()));
        Self {
            daemon_client,
            input_box: InputBox::new(),
            committed_messages: Vec::new(),
            streaming_content: String::new(),
            streaming_active: false,
            token_counter: {
                let s = settings_lock.read().unwrap();
                crate::api::token_counter::TokenCounter::new(s.agent.token_budget.main_k)
            },
            phase: AgentPhase::Idle,
            session_id,
            session_name: "New Session".to_string(),
            last_tool_name: None,
            last_abort_reason: None,
            scroll_offset: 0,
            user_scrolled: false,
            conversation_history,
            assembled_system_messages: system_messages,
            pending_inputs: VecDeque::new(),
            current_turn_handle: None,
            current_turn_id: None,
            turn_count: 0,
            mode: if settings.agent.plan_mode {
                AgentMode::PlanMode
            } else {
                AgentMode::Normal
            },
            event_tx,
            event_rx,
            should_quit: false,
            permission_state: PermissionState::new(),
            question_state: QuestionState::new(),
            session_state: SessionState::new(),
            task_panel: TaskPanelState::new(),
            plan_panel_state: PlanPanelState::new(),
            subagent_tree: SubagentTree::default(),
            subagent_history: HashMap::new(),
            subagent_panel_visible: false,
            subagent_panel_state: SubagentPanelState::default(),

            last_ctrl_c: None,
            has_running_tool: false,
            spinner_frame: 0,
            turn_started_at: None,
            shutdown_flag: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),

            settings_lock,

            completion_engine: {
                let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
                let skills_dir = home.join(".claude").join("skills");
                let builtin_commands = vec![
                    crate::tui::completion::CommandEntry {
                        name: "code-review".to_string(),
                        description: "Review code changes".to_string(),
                        args_hint: None,
                    },
                    crate::tui::completion::CommandEntry {
                        name: "clear".to_string(),
                        description: "Clear screen".to_string(),
                        args_hint: None,
                    },
                    crate::tui::completion::CommandEntry {
                        name: "help".to_string(),
                        description: "Show help".to_string(),
                        args_hint: None,
                    },
                ];
                Some(crate::tui::completion::CompletionEngine::load(
                    &skills_dir,
                    &builtin_commands,
                ))
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
        }
    }

    pub fn event_sender(&self) -> mpsc::UnboundedSender<AppEvent> {
        self.event_tx.clone()
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
        // Main loop
        while !self.should_quit {
            // Process pending events
            while let Ok(event) = self.event_rx.try_recv() {
                self.handle_event(event).await;
                if self.should_quit {
                    break;
                }
            }
            terminal.draw(|f| self.render(f))?;
            // Block until next event (prevents busy-waiting)
            if let Some(event) = self.event_rx.recv().await {
                self.handle_event(event).await;
            }
        }
        Ok(())
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
