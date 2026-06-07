//! Application main loop — event handling, layout, and daemon lifecycle.
use crate::api::ChatMessage;
use crate::state::AppState;

/// Agent operating mode, cycled via Shift+Tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentMode {
    Normal,
    PlanMode,
    AcceptEdits,
    Yolo,
}

impl AgentMode {
    pub fn label(&self) -> &str {
        match self {
            AgentMode::Normal => "NORMAL",
            AgentMode::PlanMode => "PLAN",
            AgentMode::AcceptEdits => "ACCEPT EDIT",
            AgentMode::Yolo => "YOLO",
        }
    }

    pub fn color(&self) -> Color {
        match self {
            AgentMode::Normal => Color::Rgb(147, 112, 219),
            AgentMode::PlanMode => Color::Rgb(255, 200, 80),
            AgentMode::AcceptEdits => Color::Rgb(80, 220, 120),
            AgentMode::Yolo => Color::Rgb(255, 90, 90),
        }
    }

    pub fn next(&self) -> Self {
        match self {
            AgentMode::Normal => AgentMode::PlanMode,
            AgentMode::PlanMode => AgentMode::AcceptEdits,
            AgentMode::AcceptEdits => AgentMode::Yolo,
            AgentMode::Yolo => AgentMode::Normal,
        }
    }
}
use crate::prompts::{self, PromptContext};
use crate::tui::agent::AgentLoop;
use crate::tui::client::DaemonClient;
use crate::tui::client::SessionInfo;
use crate::tui::client::TodoItem;
use crate::tui::components;
use crate::tui::components::input::InputBox;
use crate::tui::components::permission::PermissionState;
use crate::tui::components::question::QuestionState;

use crate::tui::components::session::SessionState;
use crate::tui::components::plan_panel::PlanPanelState;
use crate::tui::components::task_panel::TaskPanelState;
use crate::state::agent_phase::{AgentPhase, TurnId, TurnAbortReason};


use crate::tui::theme;
use crossterm::event::{self, Event, EnableBracketedPaste, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use ratatui::{Frame, Terminal};
use std::collections::VecDeque;
use std::io;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::Mutex as TokioMutex;
/// Wraps a oneshot sender for returning question answers.
/// Manual Debug impl because `oneshot::Sender` doesn't implement Debug.
pub struct QuestionResponder(pub Option<tokio::sync::oneshot::Sender<Vec<String>>>);
impl std::fmt::Debug for QuestionResponder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("QuestionResponder").finish()
    }
}
#[derive(Debug)]
pub enum PermissionResponse {
    AllowOnce,
    AlwaysAllow,
    Deny,
}
/// Wraps a oneshot sender for returning permission decisions.
pub struct PermissionResponder(pub Option<tokio::sync::oneshot::Sender<PermissionResponse>>);
impl std::fmt::Debug for PermissionResponder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("PermissionResponder").finish()
    }
}
/// Events that drive the UI loop.
#[derive(Debug)]
pub enum AppEvent {
    /// Full key event for tui-textarea processing (CJK/IME support)
    KeyEvent(Box<KeyEvent>),
    /// User submitted input text
    Submit(String),
    /// An SSE content delta arrived
    ContentDelta(String),
    /// An SSE reasoning delta arrived
    ReasoningDelta(String),
    /// Streaming completed
    StreamDone { finish_reason: String },
    /// LLM started generating tool calls (bridge between text and execution)
    PreparingTools,
    /// A tool call started
    ToolStart { name: String, args: serde_json::Value },
    /// A tool result arrived
    ToolResult { name: String, args: serde_json::Value, content: String },
    /// Permission is needed
    PermissionRequired {
        reason: String,
        rule: String,
        responder: PermissionResponder,
    },
    /// ask_user_question was invoked
    QuestionAsked {
        question: String,
        options: Vec<String>,
        multi_select: bool,
        responder: QuestionResponder,
    },
    /// A stream error occurred
    StreamError(String),
    /// A turn (user-input → final response) completed; start next queued input if any
    TurnComplete,
    /// A turn began processing
    TurnStarted { turn_id: TurnId },
    /// A turn was aborted before normal completion
    TurnAborted { reason: TurnAbortReason },
    /// Tick for periodic refresh
    Tick,
    /// Toggle session popup
    ToggleSessions,
    /// Toggle task panel
    ToggleTaskPanel,
    /// Pasted text from bracketed paste
    Paste(String),
    /// Mouse scroll (positive = up, negative = down)
    MouseScrolled(i16),
    /// Ctrl+C pressed (double-press to quit)
    CtrlCPressed,
    /// Structured plan updated via update_plan tool
    PlanUpdate(serde_json::Value),
    /// Sessions loaded from daemon
    SessionListLoaded(Vec<SessionInfo>),
    HistoryLoaded(Vec<crate::api::ChatMessage>),
    SaveSession,
    /// Delete a session by id
    DeleteSession(String),
    /// Toggle collapse all paragraphs
    ToggleCollapseAll,
    /// Toggle collapse latest message paragraphs
    ToggleCollapseLatest,
    /// Undo checkpoint result with diff
    UndoResult(String),
    /// Todo items updated from daemon
    TodosUpdated(Vec<TodoItem>),
    /// Settings were hot-reloaded from disk
    ConfigChanged(crate::config::Settings),
}
/// UI state for a single message in the chat view.
#[derive(Debug, Clone, PartialEq)]
pub enum MessageRole {
    User,
    Assistant,
    Tool,
    System,
}
#[derive(Debug, Clone)]
pub struct UIMessage {
    pub role: MessageRole,
    pub content: String,
    pub tool_name: Option<String>,
    pub tool_args: Option<serde_json::Value>,
    pub content_collapsed: bool,
    pub tool_collapsed: bool,
    pub diff_data: Option<DiffData>,
    pub tool_metadata: Option<serde_json::Value>,
}

/// Structured diff data for syntax-highlighted diff rendering in the TUI.
#[derive(Debug, Clone)]
pub struct DiffData {
    pub file_path: String,
    pub old_content: String,
    pub new_content: String,
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
    /// Shared settings handle — updated by the config watcher on file change.
    pub settings_lock: crate::config::watcher::SettingsHandle,


    /// Timestamp of last Ctrl+C press for double-press detection
    last_ctrl_c: Option<std::time::Instant>,
    /// True while a tool is executing (for spinner animation)
    pub has_running_tool: bool,
    /// Spinner animation frame (0-9), advanced on Tick when has_running_tool
    pub spinner_frame: u8,
    /// Cancellation flag for blocking input reader task
    shutdown_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Pending oneshot sender for question response
    pub question_responder: Option<QuestionResponder>,
    /// Pending oneshot sender for permission response
    pub permission_responder: Option<PermissionResponder>,
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
            .with_shell(
                std::env::var("SHELL").unwrap_or_else(|_| "sh".to_string()),
            )
            .with_sandbox("workspace-write")
            .with_approval("never");
        let settings = {
            let guard = settings_lock.read().unwrap();
            guard.clone()
        };
        let prompt_ctx = prompt_ctx
            .with_collaboration(settings.collaboration_mode.clone().unwrap_or_default());

        // Load skills inventory for system prompt injection
        let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
        let skills_dirs = vec![
            home.join(".wgenty-code").join("skills"),
        ];
        let skill_loader = crate::knowledge::loader::SkillLoader::load_from_dirs(&skills_dirs);
        let mut skill_inventory: Vec<prompts::SkillEntry> = Vec::new();
        for name in skill_loader.skill_names() {
            if let Some(skill) = skill_loader.load_skill(&name) {
                let desc = skill.description.clone();
                skill_inventory.push(prompts::SkillEntry { name, description: desc });
            }
        }
        let prompt_ctx = prompt_ctx.with_skills(skill_inventory);

        // Load WGENTY.md and AGENTS.md sections from project root
        let project_root = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let wgenty_sections = crate::utils::project::read_wgenty_md_sections(&project_root);
        let agents_sections = crate::utils::project::read_agents_md_sections(&project_root);

        // Warn if WGENTY.md + AGENTS.md exceed token budget (fires once per session)
        let wgenty_tokens: usize = wgenty_sections.iter().map(|s| crate::utils::estimate_tokens(s)).sum();
        let agents_tokens: usize = agents_sections.iter().map(|s| crate::utils::estimate_tokens(s)).sum();
        let total_md_tokens = wgenty_tokens + agents_tokens;
        if total_md_tokens > 2000 {
            tracing::warn!(
                wgenty_tokens,
                agents_tokens,
                total = total_md_tokens,
                "WGENTY.md + AGENTS.md sections estimate ~{} tokens ({} + {}). \
                 Consider trimming to keep session startup lean.",
                total_md_tokens, wgenty_tokens, agents_tokens,
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
                crate::api::token_counter::TokenCounter::new(s.token_budget_k)
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
            mode: if settings.plan_mode { AgentMode::PlanMode } else { AgentMode::Normal },
            event_tx,
            event_rx,
            should_quit: false,
            permission_state: PermissionState::new(),
            question_state: QuestionState::new(),
            session_state: SessionState::new(),
            task_panel: TaskPanelState::new(),
            plan_panel_state: PlanPanelState::new(),


            last_ctrl_c: None,
            has_running_tool: false,
            spinner_frame: 0,
            shutdown_flag: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            question_responder: None,
            permission_responder: None,
            settings_lock,
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
            let _ = Self::read_input(tx, shutdown);
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
    fn read_input(
        tx: mpsc::UnboundedSender<AppEvent>,
        shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> io::Result<()> {
        use std::sync::atomic::Ordering;
        while !shutdown.load(Ordering::SeqCst) {
            // Poll with 100ms timeout so we can check shutdown flag frequently
            if event::poll(std::time::Duration::from_millis(100))? {
                let ev = event::read()?;
                if let Event::Mouse(mouse) = &ev {
                    use crossterm::event::MouseEventKind;
                    match mouse.kind {
                        MouseEventKind::ScrollUp => {
                            let _ = tx.send(AppEvent::MouseScrolled(5));
                        }
                        MouseEventKind::ScrollDown => {
                            let _ = tx.send(AppEvent::MouseScrolled(-5));
                        }
                        _ => {}
                    }
                    continue;
                }
                if let Event::Paste(text) = &ev {
                    let _ = tx.send(AppEvent::Paste(text.clone()));
                    continue;
                }
                if let Event::Key(key) = ev {
                    if key.kind == KeyEventKind::Press || key.kind == KeyEventKind::Repeat {
                        if key.code == KeyCode::Char('c')
                            && key.modifiers.contains(KeyModifiers::CONTROL)
                        {
                            let _ = tx.send(AppEvent::CtrlCPressed);
                            continue;
                        }
                        if key.code == KeyCode::Char('s')
                            && key.modifiers.contains(KeyModifiers::CONTROL)
                        {
                            let _ = tx.send(AppEvent::ToggleSessions);
                            continue;
                        }
                        if key.code == KeyCode::Char('t')
                            && key.modifiers.contains(KeyModifiers::CONTROL)
                        {
                            let _ = tx.send(AppEvent::ToggleTaskPanel);
                            continue;
                        }
                        if key.code == KeyCode::Char('e')
                            && key.modifiers.contains(KeyModifiers::CONTROL)
                        {
                            let _ = tx.send(AppEvent::ToggleCollapseAll);
                            continue;
                        }
                        if key.code == KeyCode::Char('o')
                            && key.modifiers.contains(KeyModifiers::CONTROL)
                        {
                            let _ = tx.send(AppEvent::ToggleCollapseLatest);
                            continue;
                        }
                        let _ = tx.send(AppEvent::KeyEvent(Box::new(key)));
                    }
                }
            }
        }
        Ok(())
    }
    async fn handle_event(&mut self, event: AppEvent) {
        // Derive phase from event (pure function); fall back to current
        if let Some(next_phase) = agent_phase_from_event(&event) {
            self.phase = next_phase;
        }
        match event {
            AppEvent::KeyEvent(key) => {
                // Permission panel handling (inline, not popup)
                // Shift+Tab: cycle agent mode
                if key.code == KeyCode::BackTab {
                    self.mode = self.mode.next();
                    return;
                }
                // Ctrl+P: toggle plan mode
                if key.code == KeyCode::Char('p') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    let is_plan = self.mode == AgentMode::PlanMode;
                    self.mode = if is_plan { AgentMode::Normal } else { AgentMode::PlanMode };
                    let msg = if !is_plan { "Plan mode enabled" } else { "Plan mode disabled" };
                    self.committed_messages.push(UIMessage {
                        role: MessageRole::System,
                        content: msg.to_string(),
                        tool_name: None,
                        content_collapsed: false,
                        tool_collapsed: true,
                        tool_args: None,
                        diff_data: None,
                        tool_metadata: None,
                    });
                    return;
                }
                // Permission panel key handling
                if self.permission_state.visible {
                    match key.code {
                        KeyCode::Char('y') => {
                            let (reason, _rule) = self.permission_state.dismiss();
                            self.push_permission_result(&reason, "Allowed once");
                            if let Some(responder) = self.permission_responder.take() {
                                let _ = responder.0.unwrap().send(PermissionResponse::AllowOnce);
                            }
                        }
                        KeyCode::Char('a') => {
                            let (reason, _rule) = self.permission_state.dismiss();
                            self.push_permission_result(&reason, "Always allow");
                            if let Some(responder) = self.permission_responder.take() {
                                let _ = responder.0.unwrap().send(PermissionResponse::AlwaysAllow);
                            }
                        }
                        KeyCode::Char('n') | KeyCode::Esc => {
                            let (reason, _rule) = self.permission_state.dismiss();
                            self.push_permission_result(&reason, "Denied");
                            if let Some(responder) = self.permission_responder.take() {
                                let _ = responder.0.unwrap().send(PermissionResponse::Deny);
                            }
                        }
                        _ => {}
                    }
                    return;
                }
                // Question panel handling (inline, not popup)
                if self.question_state.visible {
                    // Text input mode: cursor is on "Other" option
                    if self.question_state.cursor_on_other() {
                        match key.code {
                            KeyCode::Char(c) => {
                                self.question_state.other_value.push(c);
                            }
                            KeyCode::Backspace => {
                                self.question_state.other_value.pop();
                            }
                            KeyCode::Enter => {
                                let answers = self.question_state.dismiss();
                                self.push_question_answer(&answers);
                                if let Some(responder) = self.question_responder.take() {
                                    let _ = responder.0.unwrap().send(answers);
                                }
                            }
                            KeyCode::Up => {
                                self.question_state.move_up();
                            }
                            KeyCode::Down => {
                                self.question_state.move_down();
                            }
                            KeyCode::Esc => {
                                self.question_state.dismiss();
                                self.question_responder = None;
                            }
                            _ => {}
                        }
                        return;
                    }
                    match key.code {
                        KeyCode::Up | KeyCode::Char('k') => {
                            self.question_state.move_up();
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            self.question_state.move_down();
                        }
                        KeyCode::Enter => {
                            let can_submit = !self.question_state.multi_select
                                || !self.question_state.selected.is_empty();
                            if can_submit {
                                let answers = self.question_state.dismiss();
                                self.push_question_answer(&answers);
                                if let Some(responder) = self.question_responder.take() {
                                    let _ = responder.0.unwrap().send(answers);
                                }
                            }
                        }
                        KeyCode::Char(' ') => {
                            self.question_state.toggle_selection();
                        }
                        KeyCode::Esc => {
                            self.question_state.dismiss();
                            self.question_responder = None;
                        }
                        KeyCode::Char(c) if c.is_ascii_digit() => {
                            let n = c.to_digit(10).unwrap() as usize;
                            if self.question_state.select_number(n) {
                                let answers = self.question_state.dismiss();
                                self.push_question_answer(&answers);
                                if let Some(responder) = self.question_responder.take() {
                                    let _ = responder.0.unwrap().send(answers);
                                }
                            }
                        }
                        _ => {}
                    }
                    return;
                }
                // Session popup handling
                if self.session_state.visible {
                    match key.code {
                        KeyCode::Up | KeyCode::Char('k') => {
                            self.session_state.move_up();
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            self.session_state.move_down();
                        }
                        KeyCode::Enter => {
                            if let Some(session) = self.session_state.selected_session() {
                                let id = session.id.clone();
                                self.session_name = session.name.clone();
                                self.session_state.dismiss();
                                let client = self.daemon_client.clone();
                                let history = self.conversation_history.clone();
                                let tx = self.event_tx.clone();
                                tokio::spawn(async move {
                                    if let Ok(resp) = client.load_session(&id).await {
                                        let messages = resp.messages;
                                        {
                                            let mut h = history.lock().await;
                                            *h = messages.clone();
                                        }
                                        let _ = tx.send(AppEvent::HistoryLoaded(messages));
                                    }
                                });
                            }
                        }
                        KeyCode::Char('d') => {
                            if let Some(id) = self.session_state.delete_selected() {
                                let _ = self.event_tx.send(AppEvent::DeleteSession(id));
                            }
                        }
                        KeyCode::Delete | KeyCode::Backspace => {
                            if let Some(id) = self.session_state.delete_selected() {
                                let _ = self.event_tx.send(AppEvent::DeleteSession(id));
                            }
                        }
                        KeyCode::Esc => {
                            self.session_state.dismiss();
                        }
                        _ => {}
                    }
                    return;
                }
                // Scroll handling (when no popup is active)
                // scroll_offset = ratatui-native: lines skipped from top (0 = oldest, max = newest)
                match key.code {
                    KeyCode::Up => {
                        // Scroll UP → see OLDER content → fewer lines skipped from top
                        self.scroll_offset = self.scroll_offset.saturating_sub(1);
                        self.user_scrolled = true;
                        return;
                    }
                    KeyCode::Down => {
                        // Scroll DOWN → see NEWER content → more lines skipped from top
                        self.scroll_offset = self.scroll_offset.saturating_add(1);
                        return;
                    }
                    KeyCode::PageUp => {
                        self.scroll_offset = self.scroll_offset.saturating_sub(10);
                        self.user_scrolled = true;
                        return;
                    }
                    KeyCode::PageDown => {
                        self.scroll_offset = self.scroll_offset.saturating_add(10);
                        return;
                    }
                    _ => {}
                }
                // Ctrl+L: clear screen
                if key.code == KeyCode::Char('l') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    self.committed_messages.clear();
                    self.streaming_content.clear();
                    self.scroll_offset = 0;
                    self.user_scrolled = false;
                    return;
                }
                // Handle Enter/Shift+Enter BEFORE tui-textarea consumes them.
                // tui-textarea's default binding inserts newline on Enter.
                if key.code == KeyCode::Enter {
                    if key.modifiers.contains(KeyModifiers::SHIFT) {
                        // Shift+Enter → newline
                        self.input_box.textarea.insert_char('\n');
                    } else if !self.input_box.is_empty() {
                        let text = self.input_box.take_text();
                        let _ = self.event_tx.send(AppEvent::Submit(text));
                    }
                    return;
                }
                // Feed to tui-textarea for CJK/IME input.
                // Returns true if tui-textarea consumed the key.
                let handled = self.input_box.textarea.input(*key);
                if !handled {
                    match key.code {
                        KeyCode::Esc => {
                            self.should_quit = true;
                        }
                        _ => {}
                    }
                }
            }
            AppEvent::Paste(text) => {
                for c in text.chars() {
                    self.input_box.textarea.insert_char(c);
                }
            }
            AppEvent::MouseScrolled(delta) => {
                if delta > 0 {
                    self.scroll_offset = self.scroll_offset.saturating_sub(delta as u16);
                } else {
                    self.scroll_offset = self.scroll_offset.saturating_add((-delta) as u16);
                }
                self.user_scrolled = true;
            }
            AppEvent::ToggleCollapseAll => {
                let any_expanded = self.committed_messages.iter().any(|m| {
                    !m.content_collapsed || !m.tool_collapsed
                });
                let new_state = any_expanded;
                for m in &mut self.committed_messages {
                    m.content_collapsed = new_state;
                    m.tool_collapsed = new_state;
                }
            }
            AppEvent::ToggleCollapseLatest => {
                if let Some(last) = self.committed_messages.last_mut() {
                    let any_expanded = !last.content_collapsed || !last.tool_collapsed;
                    let new_state = any_expanded;
                    last.content_collapsed = new_state;
                    last.tool_collapsed = new_state;
                }
            }
            AppEvent::Submit(text) => {
                self.submit_input(text);
            }
            AppEvent::PreparingTools => { /* phase transitions automatically */ }
            AppEvent::ContentDelta(text) => {
                self.streaming_content.push_str(&text);
                self.streaming_active = true;
                // Auto-scroll: keep at bottom when streaming and user hasn't manually scrolled
                if !self.user_scrolled {
                    self.scroll_offset = 0;
                }
            }
            AppEvent::StreamDone { .. } => {
                if !self.streaming_content.is_empty() {
                    let content = std::mem::take(&mut self.streaming_content);
                    // Last response always expanded (never auto-collapse the latest reply)
                    self.committed_messages.push(UIMessage {
                        role: MessageRole::Assistant,
                        content,
                        tool_name: None,
                        tool_args: None,
                        content_collapsed: false,
                        tool_collapsed: true,
                        diff_data: None,
                        tool_metadata: None,
                    });
                }
                self.streaming_active = false;
                self.scroll_offset = 0;
                self.user_scrolled = false;
            }
            AppEvent::ToolStart { name, args } => {
                self.has_running_tool = true;
                self.last_tool_name = Some(tool_label(&name, &args));
                self.committed_messages.push(UIMessage {
                    role: MessageRole::Tool,
                    content: String::new(),
                    tool_name: Some(name.clone()),
                            tool_args: Some(args.clone()),
                    content_collapsed: false,
                    tool_collapsed: true,
                    diff_data: None,
                    tool_metadata: None,
                });
            }
            AppEvent::ToolResult { name, args, content } => {
                let diff_data = extract_diff_data(&name, &args, &content);
                let tool_metadata = extract_tool_metadata(&content);
                // Tool completed, clear running state for spinner
                self.has_running_tool = false;
                // Replace the placeholder ToolStart message with the result
                if let Some(last) = self.committed_messages.last_mut() {
                    if last.role == MessageRole::Tool
                        && last.content.is_empty()
                        && last.tool_name.as_deref() == Some(&name)
                    {
                        last.content = format_tool_result(&name, &args, &content);
                        last.tool_collapsed = true;
                        last.diff_data = diff_data;
                        last.tool_metadata = tool_metadata;
                    } else {
                        let formatted = format_tool_result(&name, &args, &content);
                        self.committed_messages.push(UIMessage {
                            role: MessageRole::Tool,
                            content: formatted,
                            tool_name: Some(name),
                            tool_args: Some(args),
                            content_collapsed: false,
                            tool_collapsed: true,
                            diff_data,
                            tool_metadata,
                        });
                    }
                }
            }
            AppEvent::StreamError(msg) => {
                self.committed_messages.push(UIMessage {
                    role: MessageRole::System,
                    content: format!("⚠ {}", msg),
                    tool_name: None,
                    content_collapsed: false,
                    tool_collapsed: true,
                    tool_args: None,
                    diff_data: None,
                    tool_metadata: None,
                });
                self.streaming_active = false;
            }
            AppEvent::Tick => {
                if self.has_running_tool {
                    self.spinner_frame = self.spinner_frame.wrapping_add(1);
                }
            }
            AppEvent::ConfigChanged(new_settings) => {
                // Rebuild system messages from new settings
                let prompt_ctx = PromptContext::new()
                    .with_cwd(
                        std::env::current_dir()
                            .unwrap_or_else(|_| std::path::PathBuf::from("."))
                            .display()
                            .to_string(),
                    )
                    .with_shell(
                        std::env::var("SHELL").unwrap_or_else(|_| "sh".to_string()),
                    )
                    .with_sandbox("workspace-write")
                    .with_approval("never")
                    .with_collaboration(new_settings.collaboration_mode.clone().unwrap_or_default());
                let project_root = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
                let wgenty_sections = crate::utils::project::read_wgenty_md_sections(&project_root);
                let agents_sections = crate::utils::project::read_agents_md_sections(&project_root);
                let prompt_ctx = prompt_ctx
                    .with_wgenty_md(wgenty_sections)
                    .with_agents_md(agents_sections);
                let assembled = prompts::assemble_instructions(&new_settings, &prompt_ctx);
                self.assembled_system_messages = assembled.system_messages;
                self.committed_messages.push(UIMessage {
                    role: MessageRole::System,
                    content: "Settings reloaded".to_string(),
                    tool_name: None,
                    content_collapsed: false,
                    tool_collapsed: true,
                    tool_args: None,
                    diff_data: None,
                    tool_metadata: None,
                });
            }
            AppEvent::TurnComplete => {
                self.turn_count += 1;
                self.current_turn_handle = None;
                self.last_abort_reason = None; // normal completion clears
                if !self.pending_inputs.is_empty() {
                    self.start_next_turn();
                }
            }
            AppEvent::TurnAborted { ref reason } => {
                self.last_abort_reason = Some(reason.clone());
            }
            AppEvent::PermissionRequired {
                reason,
                rule,
                responder,
            } => {
                tracing::info!("🔐 App: showing permission panel for '{}'", rule);
                // Yolo mode: auto-approve all permissions
                if self.mode == AgentMode::Yolo {
                    let _ = responder.0.unwrap().send(PermissionResponse::AllowOnce);
                    return;
                }
                // AcceptEdits mode: auto-approve file-edit permissions
                if self.mode == AgentMode::AcceptEdits
                    && (rule == "apply_patch" || rule == "file_edit" || rule == "file_write")
                {
                    let _ = responder.0.unwrap().send(PermissionResponse::AllowOnce);
                    return;
                }
                self.permission_responder = Some(responder);
                self.permission_state.show(reason, rule);
            }
            AppEvent::QuestionAsked {
                question,
                options,
                multi_select,
                responder,
            } => {
                self.question_responder = Some(responder);
                self.question_state.show(question, options, multi_select);
            }
            AppEvent::ToggleSessions => {
                if self.session_state.visible {
                    self.session_state.dismiss();
                } else {
                    let client = self.daemon_client.clone();
                    let tx = self.event_tx.clone();
                    tokio::spawn(async move {
                        if let Ok(sessions) = client.list_sessions().await {
                            let _ = tx.send(AppEvent::SessionListLoaded(sessions));
                        }
                    });
                }
            }
            AppEvent::SessionListLoaded(sessions) => {
                self.session_state.show(sessions);
            }
            AppEvent::DeleteSession(id) => {
                let client = self.daemon_client.clone();
                let tx = self.event_tx.clone();
                tokio::spawn(async move {
                    let _ = client.delete_session(&id).await;
                    // Refresh session list after deletion
                    if let Ok(sessions) = client.list_sessions().await {
                        let _ = tx.send(AppEvent::SessionListLoaded(sessions));
                    }
                });
            }
            AppEvent::HistoryLoaded(messages) => {
                // Convert ChatMessage to UIMessage for display
                self.committed_messages.clear();
                for msg in &messages {
                    let role = match msg.role.as_str() {
                        "user" => MessageRole::User,
                        "assistant" => MessageRole::Assistant,
                        "tool" => MessageRole::Tool,
                        _ => MessageRole::System,
                    };
                    let content = msg.content.clone().unwrap_or_default();
                    let (content_collapsed, tool_collapsed) = compute_collapse_state(&role, &content);
                    self.committed_messages.push(UIMessage {
                        role,
                        content,
                        tool_name: msg.tool_call_id.clone(),
                        tool_args: None,
                        content_collapsed,
                        tool_collapsed,
                        diff_data: None,
                        tool_metadata: None,
                    });
                }
                self.scroll_offset = 0;
                self.user_scrolled = false;
            }
            AppEvent::UndoResult(output) => {
                self.committed_messages.push(UIMessage {
                    role: MessageRole::Tool,
                    content: output.clone(),
                    tool_name: Some("undo".to_string()),
                    tool_args: Some(serde_json::json!({})),
                    tool_metadata: None,
                    content_collapsed: false,
                    tool_collapsed: false,
                    diff_data: extract_diff_data("undo", &serde_json::json!({}), &output),
                });
            }
            AppEvent::SaveSession => {
                let id = self.session_id.clone();
                let name = self.session_name.clone();
                let client = self.daemon_client.clone();
                let history = self.conversation_history.clone();
                tokio::spawn(async move {
                    let h = history.lock().await.clone();
                    let _ = client.save_session(&id, &name, &h).await;
                });
            }
            AppEvent::CtrlCPressed => {
                let now = std::time::Instant::now();
                if let Some(last) = self.last_ctrl_c {
                    if last.elapsed().as_millis() < 500 {
                        self.shutdown_flag.store(true, std::sync::atomic::Ordering::SeqCst);
                        self.should_quit = true;
                        return;
                    }
                }
                self.last_ctrl_c = Some(now);
            }
            AppEvent::PlanUpdate(value) => {
                use crate::tui::components::plan_panel::PlanItem;
                use crate::tui::components::plan_panel::PlanStatus;
                if let Some(plan_array) = value.get("plan").and_then(|p| p.as_array()) {
                    let items: Vec<PlanItem> = plan_array.iter().filter_map(|v| {
                        let step = v.get("step")?.as_str()?.to_string();
                        let status_str = v.get("status")?.as_str().unwrap_or("pending");
                        let status = PlanStatus::from_str(status_str);
                        Some(PlanItem { step, status })
                    }).collect();
                    self.plan_panel_state.update(items);
                }
            }
            AppEvent::ToggleTaskPanel => {
                self.task_panel.toggle();
                // Fetch todos from daemon if opening
                if self.task_panel.visible {
                    let client = self.daemon_client.clone();
                    let tx = self.event_tx.clone();
                    tokio::spawn(async move {
                        if let Ok(todos) = client.get_todos().await {
                            let _ = tx.send(AppEvent::TodosUpdated(todos.items));
                        }
                    });
                }
            }
            AppEvent::TodosUpdated(items) => {
                self.task_panel.update(items);
            }
            _ => {}
        }
    }
    /// Record the user's question answer as a chat message.
    fn push_question_answer(&mut self, answers: &[String]) {
        let q = &self.question_state.question;
        let a = answers.join(", ");
        self.committed_messages.push(UIMessage {
            role: MessageRole::System,
            content: format!("Q: {}\nA: {}", q, a),
            tool_name: Some("ask".to_string()),
                content_collapsed: false,
                tool_collapsed: false,
                tool_args: None,
                diff_data: None,
                tool_metadata: None,
        });
    }
    fn push_permission_result(&mut self, reason: &str, decision: &str) {
        self.committed_messages.push(UIMessage {
            role: MessageRole::System,
            content: format!("🔐 {} — {}", reason, decision),
            tool_name: Some("permission".to_string()),
                content_collapsed: false,
                tool_collapsed: false,
                tool_args: None,
                diff_data: None,
                tool_metadata: None,
        });
    }
    fn render(&self, f: &mut Frame) {
        let area = f.area();
        // Layout changes when question or permission is active:
        //   normal:    header | chat | status | input
        //   question:  header | chat | question-panel | status | input(hidden)
        //   permission: header | chat | permission-panel | status | input(hidden)
        let has_question = self.question_state.visible;
        let has_permission = self.permission_state.visible;
        let show_panel = has_question || has_permission;
        let panel_height = if has_question {
            self.question_state.height_needed()
        } else if has_permission {
            self.permission_state.height_needed()
        } else {
            0
        };
        let pending_height = self.pending_count().min(5) as u16;
        let has_pending = pending_height > 0;
        let constraints: Vec<Constraint> = if show_panel {
            vec![
                Constraint::Length(1),
                Constraint::Min(3),
                Constraint::Length(panel_height),
                Constraint::Length(1),
                Constraint::Length(if has_pending { pending_height } else { 0 }),
                Constraint::Length(0),
            ]
        } else {
            vec![
                Constraint::Length(1),
                Constraint::Min(3),
                Constraint::Length(1),
                Constraint::Length(if has_pending { pending_height } else { 0 }),
                Constraint::Length((self.input_box.textarea.lines().len() + 3).clamp(6, 16) as u16),
            ]
        };
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(area);
        let chat_idx = 1;
        let panel_idx = if show_panel { 2 } else { 0 };
        let status_idx = if show_panel { 3 } else { 2 };
        let pending_idx = if show_panel { 4 } else { 3 };
        let input_idx = if show_panel { 5 } else { 4 };
        self.render_header(f, layout[chat_idx - 1]);
        let main_area = if self.task_panel.visible {
            let split = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
                .split(layout[chat_idx]);
            components::task_panel::render(f, split[1], &self.task_panel);
            split[0]
        } else {
            layout[chat_idx]
        };
        if self.committed_messages.is_empty() && !self.streaming_active {
            components::welcome::render(f, main_area);
        } else {
            self.render_chat(f, main_area);
        }
        // Inline question / permission panel
        if self.question_state.visible {
            components::question::render(f, layout[panel_idx], &self.question_state);
        } else if self.permission_state.visible {
            components::permission::render(f, layout[panel_idx], &self.permission_state);
        } else if self.plan_panel_state.visible {
            components::plan_panel::render(f, &self.plan_panel_state, layout[panel_idx]);
        }
        self.render_status(f, layout[status_idx]);
        if has_pending {
            self.render_pending_inputs(f, layout[pending_idx]);
        }
        self.render_input(f, layout[input_idx]);
        // Session is still a popup overlay
        components::session::render(f, &self.session_state, centered_rect);
    }
    fn render_header(&self, f: &mut Frame, area: Rect) {
        components::status::render(f, area, &self.phase, &self.session_name);
    }
    fn render_chat(&self, f: &mut Frame, area: Rect) {
        components::chat::render(
            f,
            area,
            &self.committed_messages,
            &self.streaming_content,
            self.streaming_active,
            self.scroll_offset,
            self.user_scrolled,
            self.spinner_frame,
        );
    }
    fn render_status(&self, f: &mut Frame, area: Rect) {
        components::status::render(f, area, &self.phase, &self.session_name);
    }
    fn render_input(&self, f: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(area);
        self.render_mode_label(f, chunks[1]);
        self.input_box.render(f, chunks[0]);
    }

    /// Render the agent mode label at the top-left of the input area.
    fn render_mode_label(&self, f: &mut Frame, area: Rect) {
        let color = self.mode.color();
        let label = format!(" {} ", self.mode.label());
        let paragraph = Paragraph::new(label)
            .style(Style::default().fg(color))
            .alignment(ratatui::layout::Alignment::Left);
        f.render_widget(paragraph, area);
    }
    /// Display queued user inputs waiting to be processed.
    fn render_pending_inputs(&self, f: &mut Frame, area: Rect) {
        let pending_count = self.pending_inputs.len();
        if pending_count == 0 {
            return;
        }
        let max_show = (area.height as usize).min(pending_count);
        if max_show == 0 {
            return;
        }
        let mut lines: Vec<String> = Vec::new();
        for (i, input) in self.pending_inputs.iter().enumerate().take(max_show) {
            let first_line = input.lines().next().unwrap_or("");
            let trunc = if first_line.len() > 60 {
                format!("{}...", &first_line[..57])
            } else {
                first_line.to_string()
            };
            lines.push(format!("  {}. {}", i + 1, trunc));
        }
        let more = if pending_count > max_show {
            format!(" ... and {} more", pending_count - max_show)
        } else {
            String::new()
        };
        let text = format!("⏳ Queued ({}){}:\n{}", pending_count, more, lines.join("\n"));
        f.render_widget(
            Paragraph::new(Span::styled(text, Style::default().fg(theme::DIM))),
            area,
        );
    }
    /// Submit user input, automatically queueing if a Turn is already running.
    fn submit_input(&mut self, text: String) {
        // Slash commands
        if text.trim() == "/clear" {
            self.committed_messages.clear();
            self.streaming_content.clear();
            self.streaming_active = false;
            self.scroll_offset = 0;
            self.user_scrolled = false;
            self.cancel_current_turn();
            let history = self.conversation_history.clone();
            let sys_msgs = self.assembled_system_messages.clone();
            tokio::spawn(async move {
                let mut h = history.lock().await;
                *h = sys_msgs;
            });
            return;
        }
        if text.trim() == "/plan" {
            let is_plan = self.mode == AgentMode::PlanMode;
            self.mode = if is_plan { AgentMode::Normal } else { AgentMode::PlanMode };
            let msg = if !is_plan { "Plan mode enabled" } else { "Plan mode disabled" };
            self.committed_messages.push(UIMessage {
                role: MessageRole::System,
                content: msg.to_string(),
                tool_name: None,
                content_collapsed: false,
                tool_collapsed: false,
                tool_args: None,
                diff_data: None,
                tool_metadata: None,
            });
            return;
        }
        if text.trim() == "/continue" {
            if let Some(ref reason) = self.last_abort_reason {
                let label = match reason {
                    TurnAbortReason::MaxRoundsExceeded => "max rounds limit",
                    TurnAbortReason::TimedOut => "timeout",
                    _ => "recoverable error",
                };
                self.committed_messages.push(UIMessage {
                    role: MessageRole::System,
                    content: format!("\u{267B}\u{FE0F} Continuing after {}...", label),
                    tool_name: None,
                    content_collapsed: false,
                    tool_collapsed: false,
                    tool_args: None,
                    diff_data: None,
                    tool_metadata: None,
                });
                // Inject system message into conversation history
                let history = self.conversation_history.clone();
                let label_clone = label.to_string();
                tokio::spawn(async move {
                    let mut h = history.lock().await;
                    h.push(ChatMessage::system(&format!(
                        "[User pressed /continue after {}. Continue working on the previous task from where you left off.]",
                        label_clone
                    )));
                });
                self.last_abort_reason = None;
                self.pending_inputs.push_back("Continue the current task from where you left off.".to_string());
                if self.current_turn_handle.is_none() {
                    self.start_next_turn();
                }
            } else {
                self.committed_messages.push(UIMessage {
                    role: MessageRole::System,
                    content: "No interrupted turn to continue. The last turn completed normally.".to_string(),
                    tool_name: None,
                    content_collapsed: false,
                    tool_collapsed: false,
                    tool_args: None,
                    diff_data: None,
                    tool_metadata: None,
                });
            }
            return;
        }
        if text.trim() == "/undo" {
            self.committed_messages.push(UIMessage {
                role: MessageRole::System,
                content: "Undo requested".to_string(),
                tool_name: None,
                content_collapsed: false,
                tool_collapsed: false,
                tool_args: None,
                diff_data: None,
                tool_metadata: None,
            });
            self.pending_inputs.push_back("undo the most recent operation".to_string());
            if self.current_turn_handle.is_none() {
                self.start_next_turn();
            }
            return;
        }
        if text.trim() == "/init" {
            self.committed_messages.push(UIMessage {
                role: MessageRole::System,
                content: "🔄 Running /init — 正在分析代码库以生成 WGENTY.md 和 AGENTS.md...".to_string(),
                tool_name: None,
                content_collapsed: false,
                tool_collapsed: false,
                tool_args: None,
                diff_data: None,
                tool_metadata: None,
            });
            if self.current_turn_handle.is_none() {
                let init_prompt = crate::prompts::get_init_prompt().to_string();
                self.spawn_agent_turn(init_prompt, true);
            }
            return;
        }
        if self.mode == AgentMode::PlanMode {
            self.phase = AgentPhase::Thinking;
            self.pending_inputs.push_back(text);
            self.start_next_turn();
            self.mode = AgentMode::Normal;
            return;
        }
        self.pending_inputs.push_back(text);
        if self.current_turn_handle.is_none() {
            self.start_next_turn();
        }
    }
    /// Start the next pending turn (if any).
    fn start_next_turn(&mut self) {
        if let Some(text) = self.pending_inputs.pop_front() {
            // Push user message to UI immediately
            self.committed_messages.push(UIMessage {
                role: MessageRole::User,
                content: text.clone(),
                tool_name: None,
                content_collapsed: false,
                tool_collapsed: false,
                tool_args: None,
                diff_data: None,
                tool_metadata: None,
            });
            // Auto-name the session from the first user message
            if self.session_name == "New Session" {
                let name = truncate_session_name(&text);
                self.session_name = name;
            }
            self.spawn_agent_turn(text, false);
        }
    }

    /// Spawn an agent turn with `input_text` as the initial user message.
    /// When `hide_input` is true, the input is not displayed as a user message
    /// in the chat (used for internal prompts like /init).
    fn spawn_agent_turn(&mut self, input_text: String, hide_input: bool) {
        if hide_input {
            // Auto-name session from a short label instead of the full prompt
            if self.session_name == "New Session" {
                self.session_name = "Init Project".to_string();
            }
        } else if self.session_name == "New Session" {
            let name = truncate_session_name(&input_text);
            self.session_name = name;
        }
        self.phase = AgentPhase::Thinking;
        let turn_id = TurnId::new();
        self.current_turn_id = Some(turn_id.clone());
        let _ = self.event_tx.send(AppEvent::TurnStarted { turn_id: turn_id.clone() });
        let history = self.conversation_history.clone();
        let client = self.daemon_client.clone();
        let event_tx = self.event_tx.clone();
        let session_id = self.session_id.clone();
        let sys_msgs = self.assembled_system_messages.clone();
        let plan_mode = self.mode == AgentMode::PlanMode;
        // Read agent config from settings
        let (planner_client, max_rounds) = {
            let s = self.settings_lock.read().unwrap();
            let planner = if let Some(ref pm) = s.planner_model {
                let mut planner_settings = s.clone();
                planner_settings.model = pm.clone();
                if let Some(ref url) = s.planner_model_base_url {
                    planner_settings.api.base_url = url.clone();
                }
                if let Some(ref key) = s.planner_model_api_key {
                    planner_settings.api.api_key = Some(key.clone());
                }
                Some(crate::api::ApiClient::new(planner_settings))
            } else {
                None
            };
            (planner, s.max_rounds.unwrap_or(100))
        };
        let token_counter = self.token_counter.clone();
        self.current_turn_handle = Some(tokio::spawn(async move {
            let mut agent = AgentLoop::new(client, event_tx.clone(), session_id, history, sys_msgs, plan_mode, planner_client, max_rounds, token_counter);
            let result = agent.process_input(input_text).await;
            if let Err(ref e) = result {
                let reason = if e.contains("timed out") {
                    TurnAbortReason::TimedOut
                } else if e.contains("max rounds") || e.contains("LLM rounds") || e.contains("Token budget exhausted") {
                    TurnAbortReason::MaxRoundsExceeded
                } else {
                    TurnAbortReason::StreamError
                };
                let _ = event_tx.send(AppEvent::TurnAborted { reason });
            }
            let _ = event_tx.send(AppEvent::TurnComplete);
        }));
    }
    /// Cancel the current turn and flush all queued input.
    fn cancel_current_turn(&mut self) {
        self.pending_inputs.clear();
        if let Some(handle) = self.current_turn_handle.take() {
            handle.abort();
            let _ = self.event_tx.send(AppEvent::TurnAborted {
                reason: TurnAbortReason::Interrupted,
            });
        }
        self.current_turn_id = None;
    }
    /// Number of inputs waiting in the queue (excluding the running one).
    fn pending_count(&self) -> usize {
        self.pending_inputs.len()
    }
}

/// Truncate a user message to a short session name (max ~50 chars, no newlines).
fn truncate_session_name(text: &str) -> String {
    let first_line = text.lines().next().unwrap_or("");
    let trimmed = first_line.trim();
    if trimmed.len() <= 50 {
        trimmed.to_string()
    } else {
        let end = trimmed.char_indices().take(50).last().map(|(i, _)| i).unwrap_or(0);
        format!("{}...", &trimmed[..end])
    }
}

/// Start the daemon in a background tokio task and wait for it to be ready.
/// Returns the base URL (including port) and a shutdown sender.
#[cfg(feature = "daemon")]
pub async fn start_daemon(
    app_state: AppState,
) -> anyhow::Result<(String, tokio::sync::oneshot::Sender<()>, tokio::task::JoinHandle<()>)> {
    // Bind to a random available port
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let base_url = format!("http://127.0.0.1:{}", port);
    use crate::daemon::routes;
    use crate::daemon::state::DaemonState;
    use std::sync::Arc;
    use tower_http::cors::{Any, CorsLayer};
    let daemon_state = Arc::new(DaemonState::new(app_state));
    let app = routes::create_router(daemon_state).layer(
        CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any),
    );
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
            .ok();
    });
    // Wait for daemon to be ready (poll health endpoint)
    let client = DaemonClient::new(base_url.clone());
    for _attempt in 0..50 {
        if client.health().await.is_ok() {
            tracing::info!("daemon ready on port {}", port);
            return Ok((base_url, shutdown_tx, handle));
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }
    anyhow::bail!("daemon did not become ready within 5 seconds");
}
/// Compute initial collapse state based on line-count thresholds.
/// Returns (content_collapsed, tool_collapsed) tuple.
fn compute_collapse_state(role: &MessageRole, content: &str) -> (bool, bool) {
    let line_count = content.lines().count();
    match role {
        MessageRole::Assistant => {
            (line_count > 50, false)
        }
        MessageRole::Tool => {
            (false, true)
        }
        _ => (false, false),
    }
}
/// Extract DiffData from tool result. Tries metadata first, then auto-detects
/// unified diff content (lines with @@ / +++ / --- markers).
fn extract_diff_data(
    _name: &str,
    args: &serde_json::Value,
    raw_json: &str,
) -> Option<DiffData> {
    let file_path = args.get("path").and_then(|v| v.as_str()).unwrap_or("").to_string();
    // Try structured metadata first
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(raw_json) {
        if let Some(metadata) = parsed.get("metadata") {
            if let (Some(old), Some(new)) = (
                metadata.get("old_content").and_then(|v| v.as_str()),
                metadata.get("new_content").and_then(|v| v.as_str()),
            ) {
                return Some(DiffData {
                    file_path,
                    old_content: old.to_string(),
                    new_content: new.to_string(),
                });
            }
        }
    }
    // Auto-detect unified diff in content
    let content = raw_json.trim();
    let has_diff_markers = content.contains("@@") && content.contains("+++") && content.contains("---");
    if has_diff_markers {
        let (old, new) = split_unified_diff(content);
        if !old.is_empty() || !new.is_empty() {
            return Some(DiffData {
                file_path,
                old_content: old,
                new_content: new,
            });
        }
    }
    None
}

/// Split a unified diff string into old and new content for diff rendering.
fn split_unified_diff(content: &str) -> (String, String) {
    let mut old = String::new();
    let mut new = String::new();
    for line in content.lines() {
        if line.starts_with("@@") { continue; }
        if line.starts_with("---") { 
            old.push_str(line.trim_start_matches("--- "));
            old.push('\n');
            continue;
        }
        if line.starts_with("+++") {
            new.push_str(line.trim_start_matches("+++ "));
            new.push('\n');
            continue;
        }
        if line.starts_with('-') && !line.starts_with("---") {
            old.push_str(&line[1..]);
            old.push('\n');
        } else if line.starts_with('+') && !line.starts_with("+++") {
            new.push_str(&line[1..]);
            new.push('\n');
        } else {
            old.push_str(line);
            old.push('\n');
            new.push_str(line);
            new.push('\n');
        }
    }
    (old, new)
}

/// Extract execution metadata from a raw tool result JSON.
/// Returns the "metadata" sub-object if present, None otherwise.
fn extract_tool_metadata(raw_json: &str) -> Option<serde_json::Value> {
    let parsed: serde_json::Value = serde_json::from_str(raw_json).ok()?;
    parsed.get("metadata").cloned()
}

/// Parse the JSON wrapper from execute_tool_with_permission and extract the
/// meaningful content for display. Strips metadata noise like success/output_type.
/// Format a tool result for codex-style tree display. The header bullet is
/// rendered by chat.rs; this produces the content body with action verb,
/// key parameter, and indented output.
fn format_tool_result(_name: &str, _args: &serde_json::Value, raw_json: &str) -> String {
    let parsed: serde_json::Value = match serde_json::from_str(raw_json) {
        Ok(v) => v,
        Err(_) => return raw_json.trim_end().to_string(),
    };
    let error = parsed["error"].as_str().unwrap_or("");
    if !error.is_empty() {
        return error.to_string();
    }
    parsed["content"].as_str().unwrap_or("").to_string()
}

fn tool_label(name: &str, args: &serde_json::Value) -> String {
    match name {
        "exec_command" | "execute_command" => {
            args.get("command").and_then(|v| v.as_str()).unwrap_or("").to_string()
        }
        "file_read" | "read_file" => {
            args.get("path").and_then(|v| v.as_str()).unwrap_or("").to_string()
        }
        "file_write" | "file_edit" | "apply_patch" => {
            args.get("path").and_then(|v| v.as_str()).unwrap_or("").to_string()
        }
        "grep" | "search" => {
            args.get("pattern").and_then(|v| v.as_str()).unwrap_or("").to_string()
        }
        "glob_search" | "glob" | "list_files" => {
            args.get("path").and_then(|v| v.as_str()).unwrap_or("").to_string()
        }
        "web_search" => {
            args.get("query").and_then(|v| v.as_str()).unwrap_or("").to_string()
        }
        "web_fetch" => {
            args.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string()
        }
        _ => String::new(),
    }
}
/// Pure function: derive the next AgentPhase from a single AppEvent.
fn agent_phase_from_event(event: &AppEvent) -> Option<AgentPhase> {
    match event {
        AppEvent::Submit(_) => Some(AgentPhase::Thinking),
        AppEvent::PreparingTools => Some(AgentPhase::PreparingTools),
        AppEvent::ContentDelta(_) | AppEvent::ReasoningDelta(_) => {
            Some(AgentPhase::StreamingResponse)
        }
        AppEvent::StreamDone { .. } => Some(AgentPhase::Thinking),
        AppEvent::ToolStart { name, args: _ } => Some(AgentPhase::ExecutingTool {
            name: name.clone(),
        }),
        AppEvent::ToolResult { .. } => Some(AgentPhase::Thinking),
        AppEvent::PermissionRequired { reason, rule, .. } => {
            Some(AgentPhase::AwaitingPermission {
                tool: rule.clone(),
                rule: reason.clone(),
            })
        }
        AppEvent::QuestionAsked { question, .. } => {
            Some(AgentPhase::AwaitingUserInput {
                question: question.clone(),
            })
        }
        AppEvent::StreamError(_) => Some(AgentPhase::Errored(
            "Stream error".to_string(),
        )),
        AppEvent::TurnComplete => Some(AgentPhase::Idle),
        AppEvent::TurnAborted { reason } => match reason {
            TurnAbortReason::TimedOut => {
                Some(AgentPhase::Errored("Agent loop timed out".to_string()))
            }
            _ => Some(AgentPhase::Idle),
        },
        // Events that don't change phase
        AppEvent::MouseScrolled(_)
        | AppEvent::Paste(_)
        | AppEvent::KeyEvent(_)
        | AppEvent::Tick
        | AppEvent::ToggleSessions
        | AppEvent::ToggleTaskPanel
        | AppEvent::CtrlCPressed
        | AppEvent::SessionListLoaded(_)
        | AppEvent::HistoryLoaded(_)
        | AppEvent::PlanUpdate(_)
        | AppEvent::UndoResult(_)
        | AppEvent::SaveSession
        | AppEvent::DeleteSession(_)
        | AppEvent::ToggleCollapseAll
        | AppEvent::ToggleCollapseLatest
        | AppEvent::TodosUpdated(_)
        | AppEvent::TurnStarted { .. }
        | AppEvent::ConfigChanged(_) => None,
    }
}
/// Helper: create a centered rectangle of the given percentage size within `area`.
/// Used by popup components (session).
pub fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_width = area.width * percent_x / 100;
    let popup_height = area.height * percent_y / 100;
    let x = (area.width - popup_width) / 2;
    let y = (area.height - popup_height) / 2;
    Rect::new(x, y, popup_width, popup_height)
}
#[cfg(test)]
mod phase_tests {
    use super::*;
    #[test]
    fn test_phase_transitions() {
        assert_eq!(
            agent_phase_from_event(&AppEvent::Submit("hello".into())),
            Some(AgentPhase::Thinking)
        );
        assert_eq!(
            agent_phase_from_event(&AppEvent::ContentDelta("text".into())),
            Some(AgentPhase::StreamingResponse)
        );
        assert_eq!(
            agent_phase_from_event(&AppEvent::StreamDone { finish_reason: "stop".into() }),
            Some(AgentPhase::Thinking)
        );
        assert_eq!(
            agent_phase_from_event(&AppEvent::ToolStart { name: "file_read".into(), args: serde_json::json!({}) }),
            Some(AgentPhase::ExecutingTool { name: "file_read".into() })
        );
        assert_eq!(
            agent_phase_from_event(&AppEvent::ToolResult { name: "x".into(), args: serde_json::json!({}), content: "y".into() }),
            Some(AgentPhase::Thinking)
        );
        assert_eq!(
            agent_phase_from_event(&AppEvent::StreamError("fail".into())),
            Some(AgentPhase::Errored("Stream error".into()))
        );
        assert_eq!(
            agent_phase_from_event(&AppEvent::TurnComplete),
            Some(AgentPhase::Idle)
        );
        assert_eq!(
            agent_phase_from_event(&AppEvent::TurnAborted { reason: TurnAbortReason::TimedOut }),
            Some(AgentPhase::Errored("Agent loop timed out".into()))
        );
        assert_eq!(
            agent_phase_from_event(&AppEvent::TurnAborted { reason: TurnAbortReason::Interrupted }),
            Some(AgentPhase::Idle)
        );
    }
    #[test]
    fn test_non_phase_events_return_none() {
        assert_eq!(agent_phase_from_event(&AppEvent::Tick), None);
        assert_eq!(agent_phase_from_event(&AppEvent::MouseScrolled(3)), None);
        assert_eq!(agent_phase_from_event(&AppEvent::Paste("test".into())), None);
        assert_eq!(agent_phase_from_event(&AppEvent::SaveSession), None);
        assert_eq!(
            agent_phase_from_event(&AppEvent::TurnStarted { turn_id: TurnId::new() }),
            None
        );
    }
    #[test]
    fn test_phase_is_busy() {
        assert!(!AgentPhase::Idle.is_busy());
        assert!(!AgentPhase::Completed.is_busy());
        assert!(AgentPhase::Thinking.is_busy());
        assert!(AgentPhase::StreamingResponse.is_busy());
        assert!(AgentPhase::ExecutingTool { name: "x".into() }.is_busy());
        assert!(!AgentPhase::Errored("e".into()).is_busy());
    }
}