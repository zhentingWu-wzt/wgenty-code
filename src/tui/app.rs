//! Application main loop — event handling, layout, and daemon lifecycle.

use crate::state::AppState;
use crate::tui::agent::AgentLoop;
use crate::tui::client::DaemonClient;
use crate::tui::components;
use crate::tui::client::SessionInfo;
use crate::tui::client::TodoItem;
use crate::tui::components::permission::PermissionState;
use crate::tui::components::question::QuestionState;
use crate::tui::components::session::SessionState;
use crate::tui::components::task_panel::TaskPanelState;
use crate::tui::theme;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::{Frame, Terminal};
use std::io;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::Mutex as TokioMutex;

/// Events that drive the UI loop.
#[derive(Debug)]
pub enum AppEvent {
    /// User pressed a key
    Key(KeyCode),
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
    PermissionRequired { reason: String, rule: String },
    /// ask_user_question was invoked
    QuestionAsked {
        question: String,
        options: Vec<String>,
        multi_select: bool,
    },
    /// A stream error occurred
    StreamError(String),
    /// Tick for periodic refresh
    Tick,
    /// Toggle session popup
    ToggleSessions,
    /// Toggle task panel
    ToggleTaskPanel,
    /// Sessions loaded from daemon
    SessionListLoaded(Vec<SessionInfo>),
    /// Todo items updated from daemon
    TodosUpdated(Vec<TodoItem>),
}

/// UI state for a single message in the chat view.
#[derive(Debug, Clone)]
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
}

/// Application state for the TUI.
pub struct App {
    pub daemon_client: DaemonClient,
    pub input: String,
    pub committed_messages: Vec<UIMessage>,
    pub streaming_content: String,
    pub streaming_active: bool,
    pub status: String,
    pub session_id: String,
    pub session_name: String,
    pub scroll_offset: u16,
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
}

impl App {
    pub fn new(daemon_client: DaemonClient, session_id: String) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let agent = AgentLoop::new(
            daemon_client.clone(),
            event_tx.clone(),
            session_id.clone(),
        );
        Self {
            daemon_client,
            input: String::new(),
            committed_messages: Vec::new(),
            streaming_content: String::new(),
            streaming_active: false,
            status: "idle".to_string(),
            session_id,
            session_name: "New Session".to_string(),
            scroll_offset: 0,
            agent: Arc::new(TokioMutex::new(agent)),
            event_tx,
            event_rx,
            should_quit: false,
            permission_state: PermissionState::new(),
            question_state: QuestionState::new(),
            session_state: SessionState::new(),
            task_panel: TaskPanelState::new(),
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
                    match key.code {
                        KeyCode::Char('c')
                            if key.modifiers.contains(KeyModifiers::CONTROL) =>
                        {
                            let _ = tx.send(AppEvent::Key(KeyCode::Esc));
                            return Ok(());
                        }
                        KeyCode::Char('s')
                            if key.modifiers.contains(KeyModifiers::CONTROL) =>
                        {
                            let _ = tx.send(AppEvent::ToggleSessions);
                        }
                        KeyCode::Char('t')
                            if key.modifiers.contains(KeyModifiers::CONTROL) =>
                        {
                            let _ = tx.send(AppEvent::ToggleTaskPanel);
                        }
                        KeyCode::Enter => {
                            let _ = tx.send(AppEvent::Key(KeyCode::Enter));
                        }
                        _ => {
                            let _ = tx.send(AppEvent::Key(key.code));
                        }
                    }
                }
            }
        }
    }

    async fn handle_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::Key(key) => self.handle_key(key),
            AppEvent::Submit(text) => {
                self.committed_messages.push(UIMessage {
                    role: MessageRole::User,
                    content: text.clone(),
                    tool_name: None,
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
            }
            AppEvent::StreamDone { .. } => {
                if !self.streaming_content.is_empty() {
                    self.committed_messages.push(UIMessage {
                        role: MessageRole::Assistant,
                        content: std::mem::take(&mut self.streaming_content),
                        tool_name: None,
                    });
                }
                self.streaming_active = false;
                self.status = "idle".to_string();
            }
            AppEvent::ToolStart { name } => {
                self.status = format!("executing {}", name);
            }
            AppEvent::ToolResult { name, content } => {
                self.committed_messages.push(UIMessage {
                    role: MessageRole::Tool,
                    content,
                    tool_name: Some(name),
                });
                self.status = "thinking".to_string();
            }
            AppEvent::StreamError(msg) => {
                self.committed_messages.push(UIMessage {
                    role: MessageRole::System,
                    content: format!("Error: {}", msg),
                    tool_name: None,
                });
                self.streaming_active = false;
                self.status = "idle".to_string();
            }
            AppEvent::Tick => { /* periodic refresh */ }
            AppEvent::PermissionRequired { reason, rule } => {
                self.permission_state.show(reason, rule);
            }
            AppEvent::QuestionAsked {
                question,
                options,
                multi_select,
            } => {
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

    fn handle_key(&mut self, key: KeyCode) {
        // If permission popup is visible, handle its keys
        if self.permission_state.visible {
            match key {
                KeyCode::Char('y') => {
                    let (_reason, rule) = self.permission_state.dismiss();
                    let _ = self.event_tx.send(AppEvent::Submit(format!("__permission_allow:{}", rule)));
                }
                KeyCode::Char('a') => {
                    let (_reason, rule) = self.permission_state.dismiss();
                    let _ = self.event_tx.send(AppEvent::Submit(format!("__permission_always:{}", rule)));
                }
                KeyCode::Char('n') => {
                    let (_reason, _rule) = self.permission_state.dismiss();
                }
                _ => {}
            }
            return;
        }

        // If question popup is visible, handle its keys
        if self.question_state.visible {
            match key {
                KeyCode::Up | KeyCode::Char('k') => {
                    self.question_state.move_up();
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.question_state.move_down();
                }
                KeyCode::Enter => {
                    let answers = self.question_state.dismiss();
                    let _ = self.event_tx.send(AppEvent::Submit(format!("__question_answer:{:?}", answers)));
                }
                KeyCode::Esc => {
                    self.question_state.dismiss();
                }
                _ => {}
            }
            return;
        }

        // If session popup is visible, handle its keys
        if self.session_state.visible {
            match key {
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
                        // Load session in background
                        let client = self.daemon_client.clone();
                        let agent = self.agent.clone();
                        tokio::spawn(async move {
                            if let Ok(resp) = client.load_session(&id).await {
                                agent.lock().await.load_history(resp.messages);
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

        match key {
            KeyCode::Esc => {
                self.should_quit = true;
            }
            KeyCode::Enter => {
                let text = std::mem::take(&mut self.input);
                if !text.trim().is_empty() {
                    let _ = self.event_tx.send(AppEvent::Submit(text));
                }
            }
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Char(c) => {
                self.input.push(c);
            }
            _ => {}
        }
    }

    fn render(&self, f: &mut Frame) {
        let area = f.area();

        // Split vertically: header, chat, status, input
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),  // header
                Constraint::Min(3),      // chat
                Constraint::Length(1),  // status
                Constraint::Length(1),  // input
            ])
            .split(area);

        self.render_header(f, layout[0]);
        let main_area = if self.task_panel.visible {
            let split = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Percentage(70),
                    Constraint::Percentage(30),
                ])
                .split(layout[1]);
            components::task_panel::render(f, split[1], &self.task_panel);
            split[0]
        } else {
            layout[1]
        };

        if self.committed_messages.is_empty() && !self.streaming_active {
            components::welcome::render(f, main_area);
        } else {
            self.render_chat(f, main_area);
        }
        self.render_status(f, layout[2]);
        self.render_input(f, layout[3]);

        // Render popups on top (at the end so they overlay everything)
        components::permission::render(f, &self.permission_state, centered_rect);
        components::question::render(f, &self.question_state, centered_rect);
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
        let prompt = Span::styled("> ", Style::default().fg(theme::ROLE_USER));
        let input_text = &self.input;
        // Show cursor when input is empty
        let display = if input_text.is_empty() {
            Line::from(vec![
                prompt,
                Span::styled(" ", Style::default().bg(Color::White).fg(Color::Black)),
            ])
        } else {
            Line::from(vec![prompt, Span::raw(input_text)])
        };

        f.render_widget(Paragraph::new(display), area);
    }
}

/// Start the daemon in a background tokio task and wait for it to be ready.
/// Returns the base URL (including port) and a shutdown sender.
#[cfg(feature = "daemon")]
pub async fn start_daemon(
    app_state: AppState,
) -> anyhow::Result<(String, tokio::sync::oneshot::Sender<()>)> {
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

    tokio::spawn(async move {
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
            return Ok((base_url, shutdown_tx));
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    anyhow::bail!("daemon did not become ready within 5 seconds");
}

/// Helper: create a centered rectangle of the given percentage size within `area`.
/// Used by popup components (permission, question, session).
pub fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_width = area.width * percent_x / 100;
    let popup_height = area.height * percent_y / 100;
    let x = (area.width - popup_width) / 2;
    let y = (area.height - popup_height) / 2;
    Rect::new(x, y, popup_width, popup_height)
}
