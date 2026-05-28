//! Application main loop — event handling, layout, and daemon lifecycle.

use crate::state::AppState;
use crate::tui::client::DaemonClient;
use crate::tui::theme;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;
use ratatui::{Frame, Terminal};
use std::io;
use tokio::sync::mpsc;

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
    /// A stream error occurred
    StreamError(String),
    /// Tick for periodic refresh
    Tick,
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
    /// Channel sender for agent/input events
    event_tx: mpsc::UnboundedSender<AppEvent>,
    /// Channel receiver
    event_rx: mpsc::UnboundedReceiver<AppEvent>,
    should_quit: bool,
}

impl App {
    pub fn new(daemon_client: DaemonClient, session_id: String) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
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
            event_tx,
            event_rx,
            should_quit: false,
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
                self.input.clear();
                self.status = "thinking".to_string();
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
            _ => {}
        }
    }

    fn handle_key(&mut self, key: KeyCode) {
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
        self.render_chat(f, layout[1]);
        self.render_status(f, layout[2]);
        self.render_input(f, layout[3]);
    }

    fn render_header(&self, f: &mut Frame, area: Rect) {
        let text = Span::styled(
            format!(
                " {} | {} | Ctrl+C to quit",
                self.session_name, self.status
            ),
            Style::default().fg(theme::DIM),
        );
        f.render_widget(Paragraph::new(text), area);
    }

    fn render_chat(&self, f: &mut Frame, area: Rect) {
        let mut lines: Vec<Line> = Vec::new();

        for msg in &self.committed_messages {
            match msg.role {
                MessageRole::User => {
                    lines.push(Line::from(vec![
                        Span::styled("> ", Style::default().fg(theme::ROLE_USER)),
                        Span::raw(&msg.content),
                    ]));
                }
                MessageRole::Assistant => {
                    lines.push(Line::from(vec![
                        Span::styled("* ", Style::default().fg(theme::ROLE_ASSISTANT)),
                        Span::raw(&msg.content),
                    ]));
                }
                MessageRole::Tool => {
                    let label = msg.tool_name.as_deref().unwrap_or("tool");
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!("[{}] ", label),
                            Style::default().fg(theme::ROLE_TOOL),
                        ),
                        Span::styled(&msg.content, Style::default().fg(theme::DIM)),
                    ]));
                }
                MessageRole::System => {
                    lines.push(Line::from(vec![Span::styled(
                        &msg.content,
                        Style::default().fg(theme::DIM),
                    )]));
                }
            }
        }

        // Streaming content as transient line
        if self.streaming_active && !self.streaming_content.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("* ", Style::default().fg(theme::ROLE_ASSISTANT)),
                Span::raw(&self.streaming_content),
            ]));
        }

        let para = Paragraph::new(Text::from(lines)).scroll((self.scroll_offset, 0));
        f.render_widget(para, area);
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
