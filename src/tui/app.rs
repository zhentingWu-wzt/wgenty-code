//! Application main loop — event handling, layout, and daemon lifecycle.

use crate::state::AppState;
use crate::tui::agent::AgentLoop;
use crate::tui::client::DaemonClient;
use crate::tui::client::SessionInfo;
use crate::tui::client::TodoItem;
use crate::tui::components;
use crate::tui::components::input::InputBox;
use crate::tui::components::permission::PermissionState;
use crate::tui::components::question::QuestionState;
use crate::tui::components::session::SessionState;
use crate::tui::components::task_panel::TaskPanelState;
use crate::tui::theme;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use ratatui::{Frame, Terminal};
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
    KeyEvent(KeyEvent),
    /// User submitted input text
    Submit(String),
    /// An SSE content delta arrived
    ContentDelta(String),
    /// An SSE reasoning delta arrived
    ReasoningDelta(String),
    /// Streaming completed
    StreamDone { finish_reason: String },
    /// A tool call started
    ToolStart { name: String },
    /// A tool result arrived
    ToolResult { name: String, content: String },
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
    /// Tick for periodic refresh
    Tick,
    /// Toggle session popup
    ToggleSessions,
    /// Toggle task panel
    ToggleTaskPanel,
    /// Ctrl+C pressed (double-press to quit)
    CtrlCPressed,
    /// Sessions loaded from daemon
    SessionListLoaded(Vec<SessionInfo>),
    HistoryLoaded(Vec<crate::api::ChatMessage>),
    SaveSession,
    /// Toggle collapse all paragraphs
    ToggleCollapseAll,
    /// Toggle collapse latest message paragraphs
    ToggleCollapseLatest,
    /// Todo items updated from daemon
    TodosUpdated(Vec<TodoItem>),
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
    pub content_collapsed: bool,
    pub tool_collapsed: bool,
}

/// Application state for the TUI.
pub struct App {
    pub daemon_client: DaemonClient,
    pub input_box: InputBox,
    pub committed_messages: Vec<UIMessage>,
    pub streaming_content: String,
    pub streaming_active: bool,
    pub status: String,
    pub session_id: String,
    pub session_name: String,
    pub scroll_offset: u16,
    pub user_scrolled: bool,
    pub agent: Arc<TokioMutex<AgentLoop>>,
    /// Channel sender for agent/input events
    event_tx: mpsc::UnboundedSender<AppEvent>,
    /// Channel receiver
    event_rx: mpsc::UnboundedReceiver<AppEvent>,
    should_quit: bool,
    pub permission_state: PermissionState,
    pub question_state: QuestionState,
    pub session_state: SessionState,
    pub task_panel: TaskPanelState,
    /// Timestamp of last Ctrl+C press for double-press detection
    last_ctrl_c: Option<std::time::Instant>,
    /// Pending oneshot sender for question response
    pub question_responder: Option<QuestionResponder>,
    /// Pending oneshot sender for permission response
    pub permission_responder: Option<PermissionResponder>,
}

impl App {
    pub fn new(daemon_client: DaemonClient, session_id: String) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let agent = AgentLoop::new(daemon_client.clone(), event_tx.clone(), session_id.clone());
        Self {
            daemon_client,
            input_box: InputBox::new(),
            committed_messages: Vec::new(),
            streaming_content: String::new(),
            streaming_active: false,
            status: "idle".to_string(),
            session_id,
            session_name: "New Session".to_string(),
            scroll_offset: 0,
            user_scrolled: false,
            agent: Arc::new(TokioMutex::new(agent)),
            event_tx,
            event_rx,
            should_quit: false,
            permission_state: PermissionState::new(),
            question_state: QuestionState::new(),
            session_state: SessionState::new(),
            task_panel: TaskPanelState::new(),
            last_ctrl_c: None,
            question_responder: None,
            permission_responder: None,
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
        // Spawn input reader task (blocking crossterm event::read)
        let tx = self.event_tx.clone();
        tokio::task::spawn_blocking(move || {
            let _ = Self::read_input(tx);
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

    fn read_input(tx: mpsc::UnboundedSender<AppEvent>) -> io::Result<()> {
        loop {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press || key.kind == KeyEventKind::Repeat {
                    // Ctrl+C -> require double-press within 500ms to quit
                    if key.code == KeyCode::Char('c')
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        let _ = tx.send(AppEvent::CtrlCPressed);
                        continue;
                    }
                    // Ctrl+S -> sessions
                    if key.code == KeyCode::Char('s')
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        let _ = tx.send(AppEvent::ToggleSessions);
                        continue;
                    }
                    // Ctrl+T -> task panel
                    if key.code == KeyCode::Char('t')
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        let _ = tx.send(AppEvent::ToggleTaskPanel);
                        continue;
                    }
                    // Ctrl+E -> toggle collapse all
                    if key.code == KeyCode::Char('e')
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        let _ = tx.send(AppEvent::ToggleCollapseAll);
                        continue;
                    }
                    // Ctrl+O -> toggle collapse latest message
                    if key.code == KeyCode::Char('o')
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        let _ = tx.send(AppEvent::ToggleCollapseLatest);
                        continue;
                    }
                    // Send the full key event to be processed by tui-textarea + key handler
                    let _ = tx.send(AppEvent::KeyEvent(key));
                }
            }
        }
    }

    async fn handle_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::KeyEvent(key) => {
                // Permission panel handling (inline, not popup)
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
                                self.session_state.dismiss();
                                let client = self.daemon_client.clone();
                                let agent = self.agent.clone();
                                let tx = self.event_tx.clone();
                                tokio::spawn(async move {
                                    if let Ok(resp) = client.load_session(&id).await {
                                        let messages = resp.messages;
                                        agent.lock().await.load_history(messages.clone());
                                        let _ = tx.send(AppEvent::HistoryLoaded(messages));
                                    }
                                });
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
                let handled = self.input_box.textarea.input(key);

                if !handled {
                    match key.code {
                        KeyCode::Esc => {
                            self.should_quit = true;
                        }
                        _ => {}
                    }
                }
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
                // Slash commands
                if text.trim() == "/clear" {
                    self.committed_messages.clear();
                    self.streaming_content.clear();
                    self.streaming_active = false;
                    self.status = "idle".to_string();
                    self.scroll_offset = 0;
                    self.user_scrolled = false;
                    let agent = self.agent.clone();
                    tokio::spawn(async move {
                        agent.lock().await.reset();
                    });
                    return;
                }
                self.committed_messages.push(UIMessage {
                    role: MessageRole::User,
                    content: text.clone(),
                    tool_name: None,
                    content_collapsed: false,
                    tool_collapsed: false,
                });
                self.status = "thinking".to_string();
                let agent = self.agent.clone();
                tokio::spawn(async move {
                    agent.lock().await.process_input(text).await;
                });
            }
            AppEvent::ContentDelta(text) => {
                self.streaming_content.push_str(&text);
                self.streaming_active = true;
                self.status = "streaming".to_string();
                // Auto-scroll: keep at bottom when streaming and user hasn't manually scrolled
                if !self.user_scrolled {
                    self.scroll_offset = 0;
                }
            }
            AppEvent::StreamDone { .. } => {
                if !self.streaming_content.is_empty() {
                    let content = std::mem::take(&mut self.streaming_content);
                    let (content_collapsed, tool_collapsed) = compute_collapse_state(&MessageRole::Assistant, &content);
                    self.committed_messages.push(UIMessage {
                        role: MessageRole::Assistant,
                        content,
                        tool_name: None,
                        content_collapsed,
                        tool_collapsed,
                    });
                }
                self.streaming_active = false;
                self.status = "idle".to_string();
                self.scroll_offset = 0;
                self.user_scrolled = false;
            }
            AppEvent::ToolStart { name } => {
                self.status = format!("executing {}", name);
                self.committed_messages.push(UIMessage {
                    role: MessageRole::Tool,
                    content: String::new(),
                    tool_name: Some(name),
                    content_collapsed: false,
                    tool_collapsed: false,
                });
            }
            AppEvent::ToolResult { name, content } => {
                // Replace the placeholder ToolStart message with the result
                if let Some(last) = self.committed_messages.last_mut() {
                    if last.role == MessageRole::Tool
                        && last.content.is_empty()
                        && last.tool_name.as_deref() == Some(&name)
                    {
                        last.content = format_tool_result(&name, &content);
                        let (cc, tc) = compute_collapse_state(&MessageRole::Tool, &last.content);
                        last.content_collapsed = cc;
                        last.tool_collapsed = tc;
                    } else {
                        let formatted = format_tool_result(&name, &content);
                        let (content_collapsed, tool_collapsed) = compute_collapse_state(&MessageRole::Tool, &formatted);
                        self.committed_messages.push(UIMessage {
                            role: MessageRole::Tool,
                            content: formatted,
                            tool_name: Some(name),
                            content_collapsed,
                            tool_collapsed,
                        });
                    }
                }
                self.status = "thinking".to_string();
            }
            AppEvent::StreamError(msg) => {
                self.committed_messages.push(UIMessage {
                    role: MessageRole::System,
                    content: format!("⚠ {}", msg),
                    tool_name: None,
                    content_collapsed: false,
                    tool_collapsed: false,
                });
                self.streaming_active = false;
                self.status = "idle".to_string();
            }
            AppEvent::Tick => { /* periodic refresh */ }
            AppEvent::PermissionRequired {
                reason,
                rule,
                responder,
            } => {
                tracing::info!("🔐 App: showing permission panel for '{}'", rule);
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
                        content_collapsed,
                        tool_collapsed,
                    });
                }
                self.scroll_offset = 0;
                self.user_scrolled = false;
            }
            AppEvent::SaveSession => {
                let id = self.session_id.clone();
                let name = self.session_name.clone();
                let client = self.daemon_client.clone();
                let agent = self.agent.clone();
                tokio::spawn(async move {
                    let history = agent.lock().await.get_history().to_vec();
                    let _ = client.save_session(&id, &name, &history).await;
                });
            }
            AppEvent::CtrlCPressed => {
                let now = std::time::Instant::now();
                if let Some(last) = self.last_ctrl_c {
                    if last.elapsed().as_millis() < 500 {
                        self.should_quit = true;
                        return;
                    }
                }
                self.last_ctrl_c = Some(now);
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
        });
    }

    fn push_permission_result(&mut self, reason: &str, decision: &str) {
        self.committed_messages.push(UIMessage {
            role: MessageRole::System,
            content: format!("🔐 {} — {}", reason, decision),
            tool_name: Some("permission".to_string()),
                content_collapsed: false,
                tool_collapsed: false,
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

        let constraints: Vec<Constraint> = if show_panel {
            vec![
                Constraint::Length(1),
                Constraint::Min(3),
                Constraint::Length(panel_height),
                Constraint::Length(1),
                Constraint::Length(0),
            ]
        } else {
            vec![
                Constraint::Length(1),
                Constraint::Min(3),
                Constraint::Length(1),
                Constraint::Length(8),
            ]
        };

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(area);

        let chat_idx = 1;
        let panel_idx = if show_panel { 2 } else { 0 };
        let status_idx = if show_panel { 3 } else { 2 };
        let input_idx = if show_panel { 4 } else { 3 };

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
        }

        self.render_status(f, layout[status_idx]);
        self.render_input(f, layout[input_idx]);

        // Session is still a popup overlay
        components::session::render(f, &self.session_state, centered_rect);
    }

    fn render_header(&self, f: &mut Frame, area: Rect) {
        components::status::render(f, area, &self.status, &self.session_name);
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
        );
    }

    fn render_status(&self, f: &mut Frame, area: Rect) {
        let text = match self.status.as_str() {
            "idle" => Span::styled(" Ready", Style::default().fg(theme::DIM)),
            "thinking" => Span::styled(" Thinking...", Style::default().fg(theme::WARNING)),
            "streaming" => Span::styled(" Streaming...", Style::default().fg(theme::WARNING)),
            s if s.starts_with("executing") => {
                Span::styled(format!(" {}", s), Style::default().fg(theme::ACCENT))
            }
            _ => Span::raw(format!(" {}", self.status)),
        };
        f.render_widget(Paragraph::new(text), area);
    }

    fn render_input(&self, f: &mut Frame, area: Rect) {
        self.input_box.render(f, area);
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
            (false, line_count > 10)
        }
        _ => (false, false),
    }
}

/// Parse the JSON wrapper from execute_tool_with_permission and extract the
/// meaningful content for display. Strips metadata noise like success/output_type.
fn format_tool_result(name: &str, raw_json: &str) -> String {
    let parsed: serde_json::Value = match serde_json::from_str(raw_json) {
        Ok(v) => v,
        Err(_) => {
            return raw_json.to_string();
        }
    };

    let error = parsed["error"].as_str().unwrap_or("");
    if !error.is_empty() {
        return format!("{}:\n{}", name, error);
    }

    let content = parsed["content"].as_str().unwrap_or("");
    if content.is_empty() {
        let success = parsed["success"].as_bool().unwrap_or(false);
        return if success {
            format!("{}: done", name)
        } else {
            format!("{}: failed", name)
        };
    }

    format!("{}:\n{}", name, content)
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
