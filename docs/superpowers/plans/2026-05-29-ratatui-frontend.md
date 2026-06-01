# ratatui 前端实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 用 ratatui 重写 CLI 前端，替代 TypeScript+React 前端，实现单二进制分发（保留 daemon 架构零改动）。

**Architecture:** 单进程双 tokio task — 后台启动 axum daemon server（随机端口），主线程运行 ratatui Terminal + AgentLoop。daemon 代码完全不动，TypeScript 前端完整保留在 `packages/` 中。

**Tech Stack:** ratatui 0.29, tui-textarea 0.7, crossterm 0.28, reqwest (已有), tokio (已有)

**关键参考文件：**
- TS agent loop: `packages/core/src/agent-loop.ts`（需移植到 Rust）
- TS HTTP client: `packages/core/src/client.ts`（API 调用参考）
- 已有 StreamProcessor: `src/agent/core.rs`（可直接复用）
- Daemon API 路由: `src/daemon/routes.rs`（HTTP API 端点定义）
- Daemon 启动: `src/daemon/mod.rs:run()`
- API 类型: `src/api/mod.rs`（ChatMessage, ToolCall, StreamChunk 等）

---

### Task 1: 添加依赖并创建模块骨架

**Files:**
- Modify: `Cargo.toml:30-32`
- Create: `src/tui/mod.rs`
- Create: `src/tui/theme.rs`

- [ ] **Step 1: 替换 Cargo.toml 中的依赖注释为实际依赖**

```toml
# Terminal UI — ratatui-based frontend (replaces TypeScript/React frontend)
crossterm = "0.28"
ratatui = "0.29"
tui-textarea = "0.7"
```

编辑位置：找到 `# Terminal UI — migrated to TypeScript frontend (packages/cli/)` 注释块，替换为上述内容。

- [ ] **Step 2: 验证依赖能正确解析**

```bash
cargo check 2>&1 | head -5
```
Expected: 编译通过（无新模块引用，所以只是依赖下载和检查）。

- [ ] **Step 3: 创建 `src/tui/mod.rs`**

```rust
//! TUI frontend — ratatui-based terminal UI replacing the TypeScript frontend.
//!
//! Architecture:
//!   app.rs       — main event loop + layout
//!   agent.rs     — AgentLoop (SSE streaming + tool execution loop)
//!   client.rs    — HTTP client for the daemon API
//!   theme.rs     — color/styling constants
//!   components/  — ratatui widget components

pub mod agent;
pub mod app;
pub mod client;
pub mod components;
pub mod theme;
```

- [ ] **Step 4: 创建 `src/tui/theme.rs`**

```rust
use ratatui::style::Color;

// Brand colors matching the original Wgenty Code aesthetic
pub const PRIMARY: Color = Color::Magenta;
pub const ACCENT: Color = Color::Rgb(255, 140, 66);
pub const DIM: Color = Color::Rgb(120, 120, 120);
pub const SUCCESS: Color = Color::Rgb(100, 255, 100);
pub const ERROR: Color = Color::Rgb(255, 100, 100);
pub const WARNING: Color = Color::Rgb(255, 200, 100);

// Roles
pub const ROLE_USER: Color = Color::Rgb(100, 200, 255);
pub const ROLE_ASSISTANT: Color = Color::Rgb(200, 180, 255);
pub const ROLE_TOOL: Color = Color::Rgb(160, 160, 160);
pub const ROLE_SYSTEM: Color = Color::Rgb(180, 180, 140);

// Layout
pub const PROMPT_SYMBOL: &str = "▸";
```

- [ ] **Step 5: 在 `src/lib.rs` 中添加 `pub mod tui;`**

在现有 `pub mod state;` 行后添加：

```rust
pub mod tui;
```

- [ ] **Step 6: 验证编译**

```bash
cargo check 2>&1
```
Expected: 编译通过，无错误。

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml src/tui/mod.rs src/tui/theme.rs src/lib.rs
git commit -m "feat: add ratatui deps and create tui module skeleton"
```

---

### Task 2: 创建 HTTP Client（daemon 通信层）

**Files:**
- Create: `src/tui/client.rs`

- [ ] **Step 1: 创建 `src/tui/client.rs`**

```rust
//! HTTP client for communicating with the daemon API.
//! Mirrors the TypeScript ApiClient in packages/core/src/client.ts.

use crate::api::ChatMessage;
use serde::{Deserialize, Serialize};
use std::time::Duration;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);
const TOOL_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone)]
pub struct DaemonClient {
    http: reqwest::Client,
    base_url: String,
}

impl DaemonClient {
    pub fn new(base_url: String) -> Self {
        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .expect("reqwest client build");
        Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Check daemon health. Returns true if daemon is ready.
    pub async fn health(&self) -> anyhow::Result<HealthResponse> {
        let url = format!("{}/api/v1/health", self.base_url);
        let resp = self.http.get(&url).send().await?;
        Ok(resp.json().await?)
    }

    /// Get daemon config.
    pub async fn get_config(&self) -> anyhow::Result<ConfigResponse> {
        let url = format!("{}/api/v1/config", self.base_url);
        let resp = self.http.get(&url).send().await?;
        Ok(resp.json().await?)
    }

    /// POST /api/v1/chat/stream — returns the raw SSE response stream.
    pub async fn chat_stream(
        &self,
        messages: Vec<ChatMessage>,
        max_tokens: Option<usize>,
    ) -> anyhow::Result<reqwest::Response> {
        let url = format!("{}/api/v1/chat/stream", self.base_url);
        let body = ChatStreamRequest {
            messages,
            model: None,
            max_tokens,
        };
        let resp = self
            .http
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("API error ({}): {}", status, text);
        }
        Ok(resp)
    }

    /// POST /api/v1/tools/execute
    pub async fn execute_tool(
        &self,
        tool_name: &str,
        arguments: serde_json::Value,
        session_id: &str,
    ) -> anyhow::Result<ExecuteToolResponse> {
        let url = format!("{}/api/v1/tools/execute", self.base_url);
        let body = ExecuteToolRequest {
            tool_name: tool_name.to_string(),
            arguments,
            session_id: Some(session_id.to_string()),
        };
        let resp = self
            .http
            .post(&url)
            .header("Content-Type", "application/json")
            .timeout(TOOL_TIMEOUT)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            anyhow::bail!("Tool execution failed ({})", resp.status());
        }
        Ok(resp.json().await?)
    }

    /// POST /api/v1/tools/approve
    pub async fn approve_tool(&self, session_rule: &str) -> anyhow::Result<()> {
        let url = format!("{}/api/v1/tools/approve", self.base_url);
        self.http
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({"session_rule": session_rule}))
            .send()
            .await?;
        Ok(())
    }

    /// GET /api/v1/background/results
    pub async fn get_background_results(&self) -> anyhow::Result<Vec<serde_json::Value>> {
        let url = format!("{}/api/v1/background/results", self.base_url);
        let resp = self.http.get(&url).send().await?;
        if !resp.status().is_success() {
            return Ok(Vec::new());
        }
        let data: serde_json::Value = resp.json().await?;
        Ok(data["results"].as_array().cloned().unwrap_or_default())
    }

    /// GET /api/v1/sessions
    pub async fn list_sessions(&self) -> anyhow::Result<Vec<SessionInfo>> {
        let url = format!("{}/api/v1/sessions", self.base_url);
        let resp = self.http.get(&url).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("Failed to list sessions ({})", resp.status());
        }
        Ok(resp.json().await?)
    }

    /// POST /api/v1/sessions
    pub async fn create_session(&self, name: Option<&str>) -> anyhow::Result<SessionResponse> {
        let url = format!("{}/api/v1/sessions", self.base_url);
        let resp = self
            .http
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({"name": name}))
            .send()
            .await?;
        if !resp.status().is_success() {
            anyhow::bail!("Failed to create session ({})", resp.status());
        }
        Ok(resp.json().await?)
    }

    /// GET /api/v1/sessions/:id
    pub async fn load_session(&self, id: &str) -> anyhow::Result<SessionResponse> {
        let url = format!(
            "{}/api/v1/sessions/{}",
            self.base_url,
            urlencoding(id)
        );
        let resp = self.http.get(&url).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("Failed to load session ({})", resp.status());
        }
        Ok(resp.json().await?)
    }

    /// PUT /api/v1/sessions/:id
    pub async fn save_session(
        &self,
        id: &str,
        name: &str,
        messages: &[ChatMessage],
    ) -> anyhow::Result<()> {
        let url = format!(
            "{}/api/v1/sessions/{}",
            self.base_url,
            urlencoding(id)
        );
        let resp = self
            .http
            .put(&url)
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({"name": name, "messages": messages}))
            .send()
            .await?;
        if !resp.status().is_success() {
            anyhow::bail!("Failed to save session ({})", resp.status());
        }
        Ok(())
    }

    /// DELETE /api/v1/sessions/:id
    pub async fn delete_session(&self, id: &str) -> anyhow::Result<()> {
        let url = format!(
            "{}/api/v1/sessions/{}",
            self.base_url,
            urlencoding(id)
        );
        let resp = self.http.delete(&url).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("Failed to delete session ({})", resp.status());
        }
        Ok(())
    }

    /// GET /api/v1/sessions/search?q=...
    pub async fn search_sessions(&self, query: &str) -> anyhow::Result<Vec<SessionInfo>> {
        let url = format!(
            "{}/api/v1/sessions/search?q={}",
            self.base_url,
            urlencoding(query)
        );
        let resp = self.http.get(&url).send().await?;
        if !resp.ok() {
            return Ok(Vec::new());
        }
        Ok(resp.json().await?)
    }

    /// GET /api/v1/todos
    pub async fn get_todos(&self) -> anyhow::Result<TodoResponse> {
        let url = format!("{}/api/v1/todos", self.base_url);
        let resp = self.http.get(&url).send().await?;
        Ok(resp.json().await?)
    }
}

fn urlencoding(s: &str) -> String {
    // Simple percent-encode for path segments
    let mut result = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            }
            _ => {
                result.push_str(&format!("%{:02X}", byte));
            }
        }
    }
    result
}

// ── Request types ─────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct ChatStreamRequest {
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<usize>,
}

#[derive(Debug, Serialize)]
struct ExecuteToolRequest {
    tool_name: String,
    arguments: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
}

// ── Response types ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
}

#[derive(Debug, Deserialize)]
pub struct ConfigResponse {
    pub model: String,
    pub api_base: String,
    pub max_tokens: usize,
    pub timeout: u64,
    pub streaming: bool,
}

#[derive(Debug, Deserialize)]
pub struct ExecuteToolResponse {
    pub success: bool,
    pub output_type: Option<String>,
    pub content: Option<String>,
    pub error: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub permission_required: Option<PermissionRequiredInfo>,
}

#[derive(Debug, Deserialize)]
pub struct PermissionRequiredInfo {
    pub reason: String,
    pub session_rule: String,
}

#[derive(Debug, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub updated_at: String,
    pub message_count: usize,
    pub summary: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SessionResponse {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub updated_at: String,
    pub messages: Vec<ChatMessage>,
}

#[derive(Debug, Deserialize)]
pub struct TodoItem {
    pub content: String,
    pub status: String,
    pub active_form: String,
}

#[derive(Debug, Deserialize)]
pub struct TodoResponse {
    pub items: Vec<TodoItem>,
    pub has_open_items: bool,
    pub display: String,
}
```

- [ ] **Step 2: 验证编译**

```bash
cargo check 2>&1
```
Expected: 编译通过。

- [ ] **Step 3: Commit**

```bash
git add src/tui/client.rs
git commit -m "feat: add DaemonClient for HTTP communication with daemon API"
```

---

### Task 3: 创建 App 骨架（主循环 + 布局 + daemon 后台启动）

**Files:**
- Create: `src/tui/app.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: 创建 `src/tui/app.rs`**

```rust
//! Application main loop — event handling, layout, and daemon lifecycle.

use crate::agent::StreamProcessor;
use crate::api::ChatMessage;
use crate::state::AppState;
use crate::tui::client::DaemonClient;
use crate::tui::theme;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::{Frame, Terminal};
use std::io::{self, stdout};
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
    pub cursor_pos: usize,
    pub committed_messages: Vec<UIMessage>,
    pub streaming_content: String,
    pub streaming_active: bool,
    pub status: String,
    pub session_id: String,
    pub session_name: String,
    pub scroll_offset: u16,
    /// Channel receiver for events
    event_rx: mpsc::UnboundedReceiver<AppEvent>,
    /// Channel sender (clone for passing to agent loop)
    event_tx: mpsc::UnboundedSender<AppEvent>,
    should_quit: bool,
}

impl App {
    pub fn new(daemon_client: DaemonClient, session_id: String) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        Self {
            daemon_client,
            input: String::new(),
            cursor_pos: 0,
            committed_messages: Vec::new(),
            streaming_content: String::new(),
            streaming_active: false,
            status: "idle".to_string(),
            session_id,
            session_name: "New Session".to_string(),
            scroll_offset: 0,
            event_rx,
            event_tx,
            should_quit: false,
        }
    }

    pub fn event_sender(&self) -> mpsc::UnboundedSender<AppEvent> {
        self.event_tx.clone()
    }

    /// Run the main event loop.
    pub async fn run<B: tokio::io::AsyncWrite + std::marker::Unpin>(
        &mut self,
        terminal: &mut Terminal<B>,
    ) -> anyhow::Result<()> {
        // Spawn input reader task
        let tx = self.event_tx.clone();
        tokio::task::spawn_blocking(move || {
            let _ = Self::read_input(tx);
        });

        // Spawn ticker
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
            // Process events
            while let Ok(event) = self.event_rx.try_recv() {
                self.handle_event(event).await;
                if self.should_quit {
                    break;
                }
            }

            terminal.draw(|f| self.render(f))?;

            // Block until next event
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
                        KeyCode::Char('c') if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
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
                // Will be connected to agent loop in Task 6
                self.committed_messages.push(UIMessage {
                    role: MessageRole::User,
                    content: text.clone(),
                    tool_name: None,
                });
                self.input.clear();
                self.cursor_pos = 0;
                self.status = "thinking".to_string();
            }
            AppEvent::ContentDelta(text) => {
                self.streaming_content.push_str(&text);
                self.streaming_active = true;
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
                self.status = format!("error: {}", msg);
                self.streaming_active = false;
            }
            AppEvent::Tick => {
                // Periodic refresh — handled by draw loop
            }
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
                Constraint::Length(3),  // input
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
                        Span::styled("▸ ", Style::default().fg(theme::ROLE_USER)),
                        Span::raw(&msg.content),
                    ]));
                }
                MessageRole::Assistant => {
                    lines.push(Line::from(vec![
                        Span::styled("● ", Style::default().fg(theme::ROLE_ASSISTANT)),
                        Span::raw(&msg.content),
                    ]));
                }
                MessageRole::Tool => {
                    let label = msg
                        .tool_name
                        .as_deref()
                        .unwrap_or("tool");
                    lines.push(Line::from(vec![
                        Span::styled(format!("⚙ {}: ", label), Style::default().fg(theme::ROLE_TOOL)),
                        Span::styled(
                            &msg.content,
                            Style::default().fg(theme::DIM),
                        ),
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

        // Streaming content
        if self.streaming_active && !self.streaming_content.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("● ", Style::default().fg(theme::ROLE_ASSISTANT)),
                Span::raw(&self.streaming_content),
            ]));
        }

        let block = Block::default().borders(Borders::NONE);
        let para = Paragraph::new(Text::from(lines))
            .block(block)
            .scroll((self.scroll_offset, 0));
        f.render_widget(para, area);
    }

    fn render_status(&self, f: &mut Frame, area: Rect) {
        let text = match self.status.as_str() {
            "idle" => Span::styled(" Ready", Style::default().fg(theme::DIM)),
            "thinking" => Span::styled(" Thinking...", Style::default().fg(theme::WARNING)),
            s if s.starts_with("executing") => {
                Span::styled(format!(" {}", s), Style::default().fg(theme::ACCENT))
            }
            s if s.starts_with("error") => {
                Span::styled(format!(" {}", s), Style::default().fg(theme::ERROR))
            }
            _ => Span::raw(&self.status),
        };
        f.render_widget(Paragraph::new(text), area);
    }

    fn render_input(&self, f: &mut Frame, area: Rect) {
        let prompt = Span::styled("▸ ", Style::default().fg(theme::ROLE_USER));
        let input_text = self.input.clone();
        // Show cursor position as a block character
        let display = if input_text.is_empty() {
            Line::from(vec![
                prompt,
                Span::styled(" ", Style::default().bg(Color::White).fg(Color::Black)),
            ])
        } else {
            // Simple display — will be replaced with tui-textarea in Task 18
            Line::from(vec![prompt, Span::raw(&input_text)])
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::DIM));
        f.render_widget(Paragraph::new(display).block(block), area);
    }
}

/// Start the daemon in a background tokio task and wait for it to be ready.
/// Returns the base URL (including port) and a shutdown sender.
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

    // Wait for daemon to be ready
    let client = DaemonClient::new(base_url.clone());
    for _ in 0..50 {
        if client.health().await.is_ok() {
            return Ok((base_url, shutdown_tx));
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    anyhow::bail!("daemon did not become ready within 5 seconds");
}
```

- [ ] **Step 2: 修改 `src/main.rs` — 添加 tui 启动路径**

```rust
//! Wgenty Code Rust - Main Entry Point

use clap::Parser;
use wgenty_code::cli::Cli;
use wgenty_code::config::Settings;
use wgenty_code::state::AppState;
use wgenty_code::utils::logging;
use tracing::error;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    logging::init();

    let cli = Cli::parse();
    let settings = Settings::load()?;
    let state = AppState::new(settings);

    match cli.run_async(state).await {
        Ok(_) => {}
        Err(e) => {
            error!(error = ?e, "application failed");
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }

    Ok(())
}
```

对比原 `src/main.rs`，`run_async` 中的 `repl` 命令需要重定向到 tui 模式。查看 `src/cli/args.rs` 了解当前命令枚举。

- [ ] **Step 3: 检查 cli/args.rs 中的命令分发逻辑**

```bash
head -80 src/cli/args.rs
```
找到 `repl` 命令的处理逻辑，为接入 tui 做准备（不立刻修改，Task 5 再统一对接）。

- [ ] **Step 4: 验证编译**

```bash
cargo check 2>&1
```
Expected: 编译通过。

- [ ] **Step 5: Commit**

```bash
git add src/tui/app.rs src/main.rs
git commit -m "feat: add TUI app skeleton with daemon background startup"
```

---

### Task 4: 创建组件模块骨架 + WelcomeBanner + StatusBar

**Files:**
- Create: `src/tui/components/mod.rs`
- Create: `src/tui/components/welcome.rs`
- Create: `src/tui/components/status.rs`

- [ ] **Step 1: 创建 `src/tui/components/mod.rs`**

```rust
pub mod chat;
pub mod input;
pub mod permission;
pub mod question;
pub mod session;
pub mod status;
pub mod task_panel;
pub mod welcome;
```

- [ ] **Step 2: 创建 `src/tui/components/welcome.rs`**

```rust
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

/// Render the ASCII art welcome banner centered in the given area.
pub fn render(f: &mut Frame, area: Rect) {
    let banner = vec![
        "  ╔══════════════════════════════════╗",
        "  ║      Wgenty Code Rust            ║",
        "  ║      ratatui frontend             ║",
        "  ╚══════════════════════════════════╝",
        "",
        "  Type a message and press Enter to start.",
        "  Ctrl+C to quit.",
    ];

    let lines: Vec<Line> = banner
        .iter()
        .map(|s| {
            Line::from(Span::styled(
                *s,
                Style::default().fg(Color::Rgb(200, 180, 255)),
            ))
        })
        .collect();

    let para = Paragraph::new(Text::from(lines));
    f.render_widget(para, area);
}
```

- [ ] **Step 3: 创建 `src/tui/components/status.rs`**

```rust
use crate::tui::theme;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use ratatui::Frame;

/// Render the status bar at the bottom of the screen.
pub fn render(f: &mut Frame, area: Rect, status: &str, session_name: &str) {
    let text = Span::styled(
        format!(" {} | {}", session_name, status_label(status)),
        Style::default().fg(theme::DIM),
    );
    f.render_widget(Paragraph::new(text), area);
}

fn status_label(status: &str) -> &str {
    match status {
        "idle" => "Ready",
        "thinking" => "Thinking...",
        s if s.starts_with("executing") => s,
        s if s.starts_with("error") => s,
        _ => status,
    }
}
```

- [ ] **Step 4: 更新 `src/tui/app.rs` 的 render 方法使用组件**

将 `render_header` 替换为 `components::status::render(f, area, &self.status, &self.session_name)`.

- [ ] **Step 5: 验证编译**

```bash
cargo check 2>&1
```
Expected: 编译通过。

- [ ] **Step 6: Commit**

```bash
git add src/tui/components/
git commit -m "feat: add welcome banner and status bar components"
```

---

### Task 5: 创建 AgentLoop（TS agent-loop.ts → Rust 移植）

**Files:**
- Create: `src/tui/agent.rs`

这是最核心的任务，对应 TS `agent-loop.ts` 的 766 行逻辑。由于 `src/agent/core.rs` 已有 `StreamProcessor`，直接复用。

- [ ] **Step 1: 创建 `src/tui/agent.rs`**

```rust
//! AgentLoop — the core agent loop: SSE streaming + tool execution + context compaction.
//! Port of TypeScript agent-loop.ts (766 lines) to Rust.

use crate::agent::{StreamEvent, StreamProcessor, StreamResult};
use crate::api::ChatMessage;
use crate::tui::app::AppEvent;
use crate::tui::client::DaemonClient;
use std::path::PathBuf;
use tokio::sync::mpsc;

const MAX_RETRIES: u32 = 2;
const MAX_ESTIMATED_TOKENS: usize = 50_000;
const MAX_ROUNDS: usize = 10;

pub struct AgentLoop {
    client: DaemonClient,
    event_tx: mpsc::UnboundedSender<AppEvent>,
    conversation_history: Vec<ChatMessage>,
    rounds_since_todo: usize,
    compacted_summary: String,
    session_id: String,
}

impl AgentLoop {
    pub fn new(
        client: DaemonClient,
        event_tx: mpsc::UnboundedSender<AppEvent>,
        session_id: String,
    ) -> Self {
        Self {
            client,
            event_tx,
            conversation_history: vec![ChatMessage::system(build_system_prompt())],
            rounds_since_todo: 0,
            compacted_summary: String::new(),
            session_id,
        }
    }

    /// Process a single user input. Handles the full agent loop (SSE + tools).
    pub async fn process_input(&mut self, input: String) {
        self.inject_background_results().await;

        self.conversation_history
            .push(ChatMessage::user(&input));

        for _round in 0..MAX_ROUNDS {
            let messages = self.micro_compact();

            if self.needs_compaction(&messages) {
                self.do_auto_compact().await;
                continue;
            }

            let result = match self.stream_with_retry(&messages).await {
                Ok(r) => r,
                Err(e) => {
                    let _ = self
                        .event_tx
                        .send(AppEvent::StreamError(e.to_string()));
                    return;
                }
            };

            if result.has_tool_calls && !result.tool_calls.is_empty() {
                let assistant_msg =
                    StreamProcessor::build_assistant_message(
                        result.content,
                        result.reasoning_content,
                        result.tool_calls.clone(),
                    );
                self.conversation_history.push(assistant_msg);

                let mut used_todo = false;
                for tc in &result.tool_calls {
                    let args: serde_json::Value =
                        serde_json::from_str(&tc.function.arguments).unwrap_or_default();

                    if tc.function.name == "ask_user_question" {
                        let tool_result = self.handle_ask_user_question(&args).await;
                        self.conversation_history.push(ChatMessage::tool(
                            &tc.id,
                            tool_result,
                        ));
                        continue;
                    }

                    if tc.function.name == "compact" {
                        let _ = self.event_tx.send(AppEvent::ToolStart {
                            name: "compact".to_string(),
                        });
                        self.do_auto_compact().await;
                        let _ = self.event_tx.send(AppEvent::ToolResult {
                            name: "compact".to_string(),
                            content: "Conversation history compressed.".to_string(),
                        });
                        self.conversation_history.push(ChatMessage::tool(
                            &tc.id,
                            r#"{"success":true,"content":"Conversation compressed"}"#,
                        ));
                        continue;
                    }

                    if tc.function.name == "TodoWrite" {
                        used_todo = true;
                    }

                    let _ = self
                        .event_tx
                        .send(AppEvent::ToolStart {
                            name: tc.function.name.clone(),
                        });

                    let exec_result = self
                        .execute_tool_with_permission(&tc.function.name, args.clone())
                        .await;

                    let _ = self.event_tx.send(AppEvent::ToolResult {
                        name: tc.function.name.clone(),
                        content: exec_result.clone(),
                    });

                    self.conversation_history.push(ChatMessage::tool(
                        &tc.id,
                        exec_result,
                    ));
                }

                // s03: nag reminder after 3 rounds without TodoWrite
                self.rounds_since_todo = if used_todo { 0 } else { self.rounds_since_todo + 1 };
                if self.rounds_since_todo >= 3 {
                    if let Some(last) = self.conversation_history.last_mut() {
                        if last.role == "tool" {
                            if let Some(ref mut content) = last.content {
                                content.push_str("\n<reminder>Update your todos with TodoWrite.</reminder>");
                            }
                        }
                    }
                }

                continue; // Continue the tool loop
            }

            // Normal response
            if !result.content.is_empty() {
                let reasoning = if result.reasoning_content.is_empty() {
                    None
                } else {
                    Some(result.reasoning_content)
                };
                self.conversation_history.push(ChatMessage {
                    role: "assistant".to_string(),
                    content: Some(result.content),
                    reasoning_content: reasoning,
                    tool_calls: None,
                    tool_call_id: None,
                });
            }

            let _ = self.event_tx.send(AppEvent::StreamDone {
                finish_reason: result.finish_reason,
            });
            return;
        }
    }

    /// Stream with retry logic. Retries up to 2 times on network/stream errors.
    async fn stream_with_retry(
        &mut self,
        messages: &[ChatMessage],
    ) -> anyhow::Result<StreamResult> {
        let mut last_error = String::new();

        for attempt in 0..=MAX_RETRIES {
            match self.client.chat_stream(messages.to_vec(), None).await {
                Ok(response) => {
                    match self.stream_response(response).await {
                        Ok(result) => {
                            // Detect incomplete stream: tool calls without finish_reason
                            if result.has_tool_calls && result.finish_reason.is_empty() {
                                if (attempt as u32) < MAX_RETRIES {
                                    let _ = self.event_tx.send(AppEvent::StreamError(
                                        "Stream ended before tool calls completed, retrying..."
                                            .to_string(),
                                    ));
                                    tokio::time::sleep(
                                        tokio::time::Duration::from_secs((attempt + 1) as u64 * 2),
                                    )
                                    .await;
                                    continue;
                                }
                            }
                            return Ok(result);
                        }
                        Err(e) => {
                            last_error = e.to_string();
                            if (try as u32) < MAX_RETRIES {
                                let _ = self.event_tx.send(AppEvent::StreamError(format!(
                                    "Stream error, retrying... ({})",
                                    e
                                )));
                                tokio::time::sleep(
                                    tokio::time::Duration::from_secs((attempt + 1) as u64 * 2),
                                )
                                .await;
                                continue;
                            }
                        }
                    }
                }
                Err(e) => {
                    last_error = e.to_string();
                    if (try as u32) < MAX_RETRIES {
                        tokio::time::sleep(
                            tokio::time::Duration::from_secs((attempt + 1) as u64 * 2),
                        )
                        .await;
                        continue;
                    }
                }
            }
            break;
        }

        Err(anyhow::anyhow!("Stream failed: {}", last_error))
    }

    async fn stream_response(
        &mut self,
        response: reqwest::Response,
    ) -> anyhow::Result<StreamResult> {
        let mut processor = StreamProcessor::new();
        let mut stream = response.bytes_stream();

        use futures::StreamExt;
        while let Some(chunk) = stream.next().await {
            let bytes = chunk?;
            for event in processor.feed_bytes(&bytes) {
                self.dispatch_event(event);
            }
        }

        // Flush remaining buffer
        for event in processor.flush() {
            self.dispatch_event(event);
        }

        Ok(processor.finish())
    }

    fn dispatch_event(&self, event: StreamEvent) {
        match event {
            StreamEvent::ContentDelta(text) => {
                let _ = self.event_tx.send(AppEvent::ContentDelta(text));
            }
            StreamEvent::ReasoningDelta(text) => {
                let _ = self.event_tx.send(AppEvent::ReasoningDelta(text));
            }
            StreamEvent::ToolCallDelta { .. } => {
                // Accumulated internally by StreamProcessor, no UI action needed
            }
            StreamEvent::StreamDone { finish_reason } => {
                let _ = self
                    .event_tx
                    .send(AppEvent::StreamDone { finish_reason });
            }
        }
    }

    async fn execute_tool_with_permission(&mut self, name: &str, args: serde_json::Value) -> String {
        // First attempt
        match self
            .client
            .execute_tool(name, args.clone(), &self.session_id)
            .await
        {
            Ok(resp) => {
                if let Some(perm) = resp.permission_required {
                    // Signal UI for permission
                    let _ = self.event_tx.send(AppEvent::PermissionRequired {
                        reason: perm.reason.clone(),
                        rule: perm.session_rule.clone(),
                    });
                    // NOTE: In the full implementation, wait for user response via a oneshot channel.
                    // For Phase 2, deny by default.
                    return format!(
                        r#"{{"success":false,"error":"PERMISSION DENIED: {}"}}"#,
                        perm.reason
                    );
                }

                format!(
                    r#"{{"success":{},"output_type":{},"content":{},"error":{}}}"#,
                    resp.success,
                    serde_json::to_string(&resp.output_type).unwrap_or_default(),
                    serde_json::to_string(&resp.content).unwrap_or_default(),
                    serde_json::to_string(&resp.error).unwrap_or_default(),
                )
            }
            Err(e) => {
                format!(r#"{{"success":false,"error":"{}"}}"#, e)
            }
        }
    }

    async fn handle_ask_user_question(&self, args: &serde_json::Value) -> String {
        // Will be connected to QuestionPopup in Phase 3.
        // For now, auto-answer with the first option.
        let options = args["options"].as_array();
        let first = options
            .and_then(|o| o.first())
            .and_then(|o| o["label"].as_str())
            .unwrap_or("ok");

        serde_json::json!({
            "success": true,
            "answers": [{"label": first, "value": first, "custom": false}]
        })
        .to_string()
    }

    async fn inject_background_results(&mut self) {
        match self.client.get_background_results().await {
            Ok(results) if !results.is_empty() => {
                let notification = results
                    .iter()
                    .map(|r| {
                        let task_id = r["task_id"].as_str().unwrap_or("unknown");
                        let success = r["success"].as_bool().unwrap_or(false);
                        format!(
                            "[Background task {} completed: {}]",
                            task_id,
                            if success { "SUCCESS" } else { "FAILED" }
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n");
                self.conversation_history
                    .push(ChatMessage::user(notification));
            }
            _ => {}
        }
    }

    // ── Compaction ──────────────────────────────────────────────────────────

    fn micro_compact(&self) -> Vec<ChatMessage> {
        // Build tool_call_id → tool_name map
        let mut id_to_name = std::collections::HashMap::new();
        for msg in &self.conversation_history {
            if msg.role == "assistant" {
                if let Some(ref tcs) = msg.tool_calls {
                    for tc in tcs {
                        id_to_name.insert(tc.id.clone(), tc.function.name.clone());
                    }
                }
            }
        }

        // Find indices of tool messages
        let tool_indices: Vec<usize> = self
            .conversation_history
            .iter()
            .enumerate()
            .filter(|(_, m)| m.role == "tool")
            .map(|(i, _)| i)
            .collect();

        let keep_count = 3usize;
        let keep_start = tool_indices.len().saturating_sub(keep_count);
        let keep_indices: std::collections::HashSet<usize> =
            tool_indices[keep_start..].iter().copied().collect();

        self.conversation_history
            .iter()
            .enumerate()
            .map(|(i, msg)| {
                if msg.role == "tool" && !keep_indices.contains(&i) {
                    // Check if it's a read_file result — always preserve
                    let tool_name = msg
                        .tool_call_id
                        .as_deref()
                        .and_then(|id| id_to_name.get(id));
                    if tool_name == Some(&"file_read".to_string())
                        || tool_name == Some(&"read_file".to_string())
                    {
                        return msg.clone();
                    }
                    ChatMessage {
                        role: "tool".to_string(),
                        content: Some(format!(
                            "[Previous: used {}]",
                            tool_name.map_or("unknown tool", |n| n)
                        )),
                        tool_call_id: msg.tool_call_id.clone(),
                        reasoning_content: None,
                        tool_calls: None,
                    }
                } else {
                    msg.clone()
                }
            })
            .collect()
    }

    fn needs_compaction(&self, messages: &[ChatMessage]) -> bool {
        let total_chars: usize = messages
            .iter()
            .map(|m| m.content.as_deref().unwrap_or("").len())
            .sum();
        total_chars / 4 > MAX_ESTIMATED_TOKENS
    }

    async fn do_auto_compact(&mut self) {
        // Save transcript to disk
        let transcript_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".wgenty-code")
            .join("transcripts");

        tokio::fs::create_dir_all(&transcript_dir).await.ok();

        let timestamp = chrono::Utc::now()
            .format("%Y-%m-%dT%H-%M-%S")
            .to_string();
        let transcript_path = transcript_dir.join(format!("session_{}.json", timestamp));

        let json = serde_json::to_string_pretty(&self.conversation_history).unwrap_or_default();
        tokio::fs::write(&transcript_path, json).await.ok();

        // Build plain-text transcript for summarization
        let transcript_text = self
            .conversation_history
            .iter()
            .map(|m| {
                format!(
                    "[{}]: {}",
                    m.role,
                    m.content.as_deref().unwrap_or("")
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n");

        let summary_messages = vec![
            ChatMessage::system(
                "You are a conversation summary assistant. Summarize the following coding assistant conversation history for an AI agent. Preserve key details: project context, files modified, decisions made, bugs found, commands executed, and any pending tasks. Keep it concise but include all important information the agent needs to continue working. Do NOT use any tools — just return the summary as plain text.",
            ),
            ChatMessage::user(format!(
                "Summarize this conversation history:\n\n{}",
                transcript_text
            )),
        ];

        if let Ok(response) = self.client.chat_stream(summary_messages, None).await {
            let mut processor = StreamProcessor::new();
            let mut stream = response.bytes_stream();
            use futures::StreamExt;
            while let Some(Ok(bytes)) = stream.next().await {
                processor.feed_bytes(&bytes);
            }
            let result = processor.finish();
            let summary = result.content;

            if !summary.is_empty() {
                self.compacted_summary = summary.clone();

                // Rebuild conversation: system prompt → summary → last user message
                let last_user = self
                    .conversation_history
                    .iter()
                    .rev()
                    .find(|m| m.role == "user")
                    .cloned();

                self.conversation_history = vec![
                    ChatMessage::system(build_system_prompt()),
                    ChatMessage::system(format!(
                        "<previous_conversation_summary>\n{}\n</previous_conversation_summary>",
                        summary
                    )),
                ];

                if let Some(user_msg) = last_user {
                    self.conversation_history.push(user_msg);
                }
            }
        }
    }

    // ── Session state ──────────────────────────────────────────────────────

    pub fn load_history(&mut self, messages: Vec<ChatMessage>) {
        self.rounds_since_todo = 0;
        self.compacted_summary.clear();
        self.conversation_history = messages;
    }

    pub fn get_history(&self) -> &[ChatMessage] {
        &self.conversation_history
    }

    pub fn reset(&mut self) {
        self.rounds_since_todo = 0;
        self.compacted_summary.clear();
        self.conversation_history = vec![ChatMessage::system(build_system_prompt())];
    }
}

fn build_system_prompt() -> String {
    include_str!("../../../packages/core/src/agent-loop.ts")
        .lines()
        .find(|l| l.contains("You are a coding agent"))
        .map(|l| {
            l.trim_start_matches(|c: char| c == '\'' || c == '"' || c == '`')
                .trim_end_matches(|c: char| c == '\'' || c == '"' || c == '`' || c == ';')
                .to_string()
        })
        .unwrap_or_else(|| "You are a coding agent with access to tools for reading/writing files, executing commands, searching code, git operations, and task tracking.".to_string())
}
```

注意：`build_system_prompt()` 先放一个临时实现。完整 system prompt 从 TS 版本手动复制。

- [ ] **Step 2: 将完整 system prompt 硬编码到 `build_system_prompt()`**

从 `packages/core/src/agent-loop.ts` 的 `buildSystemPrompt()` 函数复制完整文本，转为 Rust 字符串。

- [ ] **Step 3: 验证编译**

```bash
cargo check 2>&1
```
Expected: 编译通过。

- [ ] **Step 4: Commit**

```bash
git add src/tui/agent.rs
git commit -m "feat: add AgentLoop — Rust port of TS agent-loop.ts"
```

---

### Task 6: 创建 ChatView 组件 + 对接 AgentLoop

**Files:**
- Create: `src/tui/components/chat.rs`
- Modify: `src/tui/app.rs`（对接 AgentLoop 输入处理）

- [ ] **Step 1: 创建 `src/tui/components/chat.rs`**

```rust
use crate::tui::app::{MessageRole, UIMessage};
use crate::tui::theme;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

/// Render the chat message list.
/// `committed_messages` — already committed message history (stable during streaming)
/// `streaming_content` — current streaming tokens (changes frequently)
/// `streaming_active` — whether we're currently streaming
/// `scroll_offset` — manual scroll position (0 = bottom)
pub fn render(
    f: &mut Frame,
    area: Rect,
    committed_messages: &[UIMessage],
    streaming_content: &str,
    streaming_active: bool,
    scroll_offset: u16,
) {
    let mut lines: Vec<Line> = Vec::new();

    for msg in committed_messages {
        lines.extend(message_to_lines(msg));
    }

    // Streaming content rendered as a separate transient line
    if streaming_active && !streaming_content.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("● ", Style::default().fg(theme::ROLE_ASSISTANT)),
            Span::raw(streaming_content),
        ]));
    }

    let block = Block::default().borders(Borders::NONE);
    let para = Paragraph::new(Text::from(lines))
        .block(block)
        .scroll((scroll_offset, 0));
    f.render_widget(para, area);
}

fn message_to_lines(msg: &UIMessage) -> Vec<Line<'static>> {
    match msg.role {
        MessageRole::User => {
            vec![Line::from(vec![
                Span::styled("▸ ", Style::default().fg(theme::ROLE_USER)),
                Span::raw(msg.content.clone()),
            ])]
        }
        MessageRole::Assistant => {
            let mut lines = Vec::new();
            for line in msg.content.lines() {
                lines.push(Line::from(vec![
                    Span::styled("● ", Style::default().fg(theme::ROLE_ASSISTANT)),
                    Span::raw(line.to_string()),
                ]));
            }
            if lines.is_empty() {
                lines.push(Line::from(vec![
                    Span::styled("● ", Style::default().fg(theme::ROLE_ASSISTANT)),
                    Span::raw(""),
                ]));
            }
            lines
        }
        MessageRole::Tool => {
            let label = msg.tool_name.as_deref().unwrap_or("tool");
            let preview = if msg.content.len() > 300 {
                format!("{}...", &msg.content[..300])
            } else {
                msg.content.clone()
            };
            vec![Line::from(vec![
                Span::styled(
                    format!("⚙ {}: ", label),
                    Style::default().fg(theme::ROLE_TOOL),
                ),
                Span::styled(preview, Style::default().fg(theme::DIM)),
            ])]
        }
        MessageRole::System => {
            vec![Line::from(vec![Span::styled(
                &msg.content,
                Style::default().fg(theme::DIM),
            )])]
        }
    }
}
```

- [ ] **Step 2: 修改 `src/tui/app.rs` — 将 AgentLoop 接入输入处理**

在 `App` 中添加 `agent: Option<AgentLoop>` 字段。在 `handle_event` 的 `AppEvent::Submit(text)` 分支中调用 `agent.process_input(text).await`。

需要在 app.rs 顶部添加：
```rust
use crate::tui::agent::AgentLoop;
```

并在 `App::new()` 中初始化 agent：
```rust
let agent = AgentLoop::new(
    daemon_client.clone(),
    event_tx.clone(),
    session_id.clone(),
);
```

- [ ] **Step 3: 用 `components::chat::render` 替换 `app.rs` 中的 `render_chat`**

```rust
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
```

- [ ] **Step 4: 修改 `handle_event` 中的 Submit 处理**

将 `AppEvent::Submit(text)` 分支改为：
```rust
AppEvent::Submit(text) => {
    self.committed_messages.push(UIMessage {
        role: MessageRole::User,
        content: text.clone(),
        tool_name: None,
    });
    self.status = "thinking".to_string();
    if let Some(ref agent) = self.agent {
        let mut agent = agent.clone(); // AgentLoop needs &mut — use Arc<Mutex<>>
        // AgentLoop should be wrapped in Arc<tokio::sync::Mutex<>> for shared access
    }
}
```

注意：这要求 AgentLoop 包装在 `Arc<tokio::sync::Mutex<AgentLoop>>` 中以在 event loop 和 agent 之间共享。修改 `App` 结构体中的 agent 字段类型。

- [ ] **Step 5: 处理滚动逻辑**

添加 `scroll_to_bottom` 方法：
```rust
fn scroll_to_bottom(&mut self, area_height: u16) {
    let total_lines = self.committed_messages.len() as u16
        + if self.streaming_active { 1 } else { 0 };
    if total_lines > area_height as usize {
        self.scroll_offset = (total_lines as usize - area_height as usize) as u16;
    }
}
```

在每次 `committed_messages` 变化和 streaming 更新时调用。

- [ ] **Step 6: 验证编译**

```bash
cargo check 2>&1
```
Expected: 编译通过。

- [ ] **Step 7: Commit**

```bash
git add src/tui/components/chat.rs src/tui/app.rs
git commit -m "feat: add ChatView component and wire AgentLoop to UI"
```

---

### Task 7: 创建 PermissionPopup + QuestionPopup 组件

**Files:**
- Create: `src/tui/components/permission.rs`
- Create: `src/tui/components/question.rs`
- Modify: `src/tui/app.rs`（添加弹窗状态管理）

- [ ] **Step 1: 创建 `src/tui/components/permission.rs`**

```rust
use crate::tui::theme;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

/// Permission popup state.
pub struct PermissionState {
    pub visible: bool,
    pub reason: String,
    pub rule: String,
}

impl PermissionState {
    pub fn new() -> Self {
        Self {
            visible: false,
            reason: String::new(),
            rule: String::new(),
        }
    }

    pub fn show(&mut self, reason: String, rule: String) {
        self.visible = true;
        self.reason = reason;
        self.rule = rule;
    }

    pub fn dismiss(&mut self) -> (String, String) {
        self.visible = false;
        (std::mem::take(&mut self.reason), std::mem::take(&mut self.rule))
    }
}

/// Render the permission popup centered on screen.
pub fn render(f: &mut Frame, state: &PermissionState) {
    if !state.visible {
        return;
    }

    let area = f.area();
    let popup_area = centered_rect(60, 25, area);

    // Clear the background
    f.render_widget(Clear, popup_area);

    let text = vec![
        Line::from(Span::styled(
            " ⚠ Permission Required",
            Style::default().fg(theme::WARNING),
        )),
        Line::from(""),
        Line::from(Span::raw(format!(" {}", state.reason))),
        Line::from(""),
        Line::from(Span::styled(
            " [y] Allow once    [a] Always allow    [n] Deny",
            Style::default().fg(theme::DIM),
        )),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::WARNING))
        .title(" Permission ");

    let para = Paragraph::new(Text::from(text))
        .block(block)
        .wrap(Wrap { trim: true });

    f.render_widget(para, popup_area);
}
```

- [ ] **Step 1b: 在 `src/tui/app.rs` 中添加 `centered_rect` 辅助函数（权限弹窗和问题弹窗共用）**

```rust
/// Create a centered rectangle of the given percentage size within `area`.
fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_width = area.width * percent_x / 100;
    let popup_height = area.height * percent_y / 100;
    let x = (area.width - popup_width) / 2;
    let y = (area.height - popup_height) / 2;
    Rect::new(x, y, popup_width, popup_height)
}
```

- [ ] **Step 2: 创建 `src/tui/components/question.rs`**

```rust
use crate::tui::theme;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

/// Question popup state (for ask_user_question tool).
pub struct QuestionState {
    pub visible: bool,
    pub question: String,
    pub options: Vec<String>,
    pub multi_select: bool,
    pub selected: Vec<usize>,
}

impl QuestionState {
    pub fn new() -> Self {
        Self {
            visible: false,
            question: String::new(),
            options: Vec::new(),
            multi_select: false,
            selected: Vec::new(),
        }
    }

    pub fn show(&mut self, question: String, options: Vec<String>, multi_select: bool) {
        self.visible = true;
        self.question = question;
        self.options = options;
        self.multi_select = multi_select;
        self.selected = vec![0]; // default select first
    }

    pub fn dismiss(&mut self) -> Vec<String> {
        self.visible = false;
        self.selected
            .iter()
            .map(|&i| self.options.get(i).cloned().unwrap_or_default())
            .collect()
    }

    pub fn move_up(&mut self) {
        if !self.selected.is_empty() {
            self.selected[0] = self.selected[0].saturating_sub(1);
        }
    }

    pub fn move_down(&mut self) {
        if !self.selected.is_empty() {
            let max = self.options.len().saturating_sub(1);
            self.selected[0] = (self.selected[0] + 1).min(max);
        }
    }
}

/// Render the question popup centered on screen.
pub fn render(f: &mut Frame, state: &QuestionState) {
    if !state.visible {
        return;
    }

    let area = f.area();
    let popup_area = centered_rect(70, 30, area);
    f.render_widget(Clear, popup_area);

    let items: Vec<ListItem> = state
        .options
        .iter()
        .enumerate()
        .map(|(i, opt)| {
            let prefix = if state.selected.contains(&i) {
                "▶ "
            } else {
                "  "
            };
            ListItem::new(Span::raw(format!("{}{}", prefix, opt)))
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme::ACCENT))
                .title(format!(" {} ", state.question)),
        );

    f.render_widget(list, popup_area);
}
```

- [ ] **Step 3: 在 `src/tui/app.rs` 中添加弹窗状态和渲染**

添加字段：
```rust
permission_state: PermissionState,
question_state: QuestionState,
```

在 `render` 方法最后加入弹窗渲染：
```rust
// Render popups on top
components::permission::render(f, &self.permission_state);
components::question::render(f, &self.question_state);
```

- [ ] **Step 4: 在 AppEvent 枚举中添加权限/问题的用户响应事件**

```rust
PermissionResponse { allow: bool, always: bool },
QuestionResponse { answers: Vec<String> },
```

处理 `AppEvent::PermissionRequired` 时显示弹窗；在 key handler 中处理 y/n/a 按键来响应。

- [ ] **Step 5: 验证编译**

```bash
cargo check 2>&1
```
Expected: 编译通过。

- [ ] **Step 6: Commit**

```bash
git add src/tui/components/permission.rs src/tui/components/question.rs src/tui/app.rs
git commit -m "feat: add PermissionPopup and QuestionPopup components"
```

---

### Task 8: 创建 SessionPopup + TaskPanel 组件

**Files:**
- Create: `src/tui/components/session.rs`
- Create: `src/tui/components/task_panel.rs`
- Modify: `src/tui/app.rs`（添加会话列表加载和 todo 数据获取）

- [ ] **Step 1: 创建 `src/tui/components/session.rs`**

```rust
use crate::tui::client::SessionInfo;
use crate::tui::theme;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};
use ratatui::Frame;

pub struct SessionState {
    pub visible: bool,
    pub sessions: Vec<SessionInfo>,
    pub selected: usize,
    pub search_query: String,
}

impl SessionState {
    pub fn new() -> Self {
        Self {
            visible: false,
            sessions: Vec::new(),
            selected: 0,
            search_query: String::new(),
        }
    }

    pub fn show(&mut self, sessions: Vec<SessionInfo>) {
        self.visible = true;
        self.sessions = sessions;
        self.selected = 0;
    }

    pub fn dismiss(&mut self) {
        self.visible = false;
    }

    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn move_down(&mut self) {
        if !self.sessions.is_empty() {
            self.selected = (self.selected + 1).min(self.sessions.len() - 1);
        }
    }

    pub fn selected_session(&self) -> Option<&SessionInfo> {
        self.sessions.get(self.selected)
    }
}

/// Render session list popup.
pub fn render(f: &mut Frame, state: &SessionState) {
    if !state.visible {
        return;
    }

    let area = f.area();
    let popup_area = centered_rect(60, 50, area);
    f.render_widget(Clear, popup_area);

    let items: Vec<ListItem> = state
        .sessions
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let prefix = if i == state.selected { "▶ " } else { "  " };
            let name = if s.name.is_empty() { "(unnamed)" } else { &s.name };
            ListItem::new(format!(
                "{}{}  ({} msgs, {})",
                prefix, name, s.message_count, s.updated_at
            ))
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::PRIMARY))
            .title(" Sessions (↑↓ select, Enter load, / search) "),
    );

    f.render_widget(list, popup_area);
}
```

- [ ] **Step 2: 创建 `src/tui/components/task_panel.rs`**

```rust
use crate::tui::client::TodoItem;
use crate::tui::theme;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem};
use ratatui::Frame;

pub struct TaskPanelState {
    pub visible: bool,
    pub items: Vec<TodoItem>,
}

impl TaskPanelState {
    pub fn new() -> Self {
        Self {
            visible: false,
            items: Vec::new(),
        }
    }

    pub fn update(&mut self, items: Vec<TodoItem>) {
        self.items = items;
    }

    pub fn toggle(&mut self) {
        self.visible = !self.visible;
    }
}

/// Render the task panel on the right side of the screen.
pub fn render(f: &mut Frame, area: Rect, state: &TaskPanelState) {
    if !state.visible || state.items.is_empty() {
        return;
    }

    let items: Vec<ListItem> = state
        .items
        .iter()
        .map(|item| {
            let (icon, color) = match item.status.as_str() {
                "completed" => ("✓", theme::SUCCESS),
                "in_progress" => ("●", theme::ACCENT),
                _ => ("○", theme::DIM),
            };
            let label = if item.active_form.is_empty() {
                &item.content
            } else {
                &item.active_form
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!("{} ", icon), Style::default().fg(color)),
                Span::raw(label),
            ]))
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::DIM))
            .title(" Tasks "),
    );

    f.render_widget(list, area);
}
```

- [ ] **Step 3: 将新组件集成到 App 中**

在 `app.rs` 的 `render` 方法中添加 session popup 渲染和 task panel 渲染。TaskPanel 在右侧使用 `Layout::horizontal` 分割空间。

添加快捷键：
- `Ctrl+S` → toggle session popup
- `Ctrl+T` → toggle task panel
- 在 popup 可见时，↑↓ 导航，Enter 确认

- [ ] **Step 4: 验证编译**

```bash
cargo check 2>&1
```
Expected: 编译通过。

- [ ] **Step 5: Commit**

```bash
git add src/tui/components/session.rs src/tui/components/task_panel.rs src/tui/app.rs
git commit -m "feat: add SessionPopup and TaskPanel components"
```

---

### Task 9: 创建 InputBox 组件（tui-textarea 集成）

**Files:**
- Create: `src/tui/components/input.rs`
- Modify: `src/tui/app.rs`（用 InputBox 替换手动输入处理）

- [ ] **Step 1: 创建 `src/tui/components/input.rs`**

```rust
use crate::tui::theme;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders};
use ratatui::Frame;
use tui_textarea::TextArea;

/// Input box wrapping tui-textarea for CJK/IME-compatible text input.
pub struct InputBox {
    pub textarea: TextArea<'static>,
}

impl InputBox {
    pub fn new() -> Self {
        let mut textarea = TextArea::default();
        textarea.set_block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme::DIM)),
        );
        textarea.set_placeholder_text("Type your message...");
        Self { textarea }
    }

    pub fn render(&self, f: &mut Frame, area: Rect) {
        f.render_widget(&self.textarea, area);
    }

    pub fn take_text(&mut self) -> String {
        let text = self.textarea.lines().join("\n");
        self.textarea = TextArea::default();
        self.textarea.set_block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme::DIM)),
        );
        text
    }
}
```

- [ ] **Step 2: 在 `app.rs` 中替换手动输入**

移除 `input: String` 和 `cursor_pos: usize` 字段，替换为 `input_box: InputBox`。

在 `handle_key` 中，将所有 key events 直接传给 `self.input_box.textarea.input(event)`（通过 `crossterm::event::Event::Key`）。

Enter 提交时调用 `self.input_box.take_text()`。

- [ ] **Step 3: 更新 `render_input` 方法**

```rust
fn render_input(&self, f: &mut Frame, area: Rect) {
    self.input_box.render(f, area);
}
```

- [ ] **Step 4: 验证编译 + 测试 CJK 输入**

```bash
cargo check 2>&1
cargo run
```
手动测试：输入中文 "你好世界"，验证 Backspace 逐字删除正常。

- [ ] **Step 5: Commit**

```bash
git add src/tui/components/input.rs src/tui/app.rs
git commit -m "feat: add InputBox with tui-textarea for CJK/IME support"
```

---

### Task 10: 滚动稳定性 + 打磨

**Files:**
- Modify: `src/tui/app.rs`
- Modify: `src/tui/components/chat.rs`

- [ ] **Step 1: 实现分区渲染 — committed 消息区和 streaming 行分离**

在 `app.rs` 中，确保 `committed_messages` 只在消息提交时修改（ToolStart/ToolResult/StreamDone 事件），streaming 期间只更新 `streaming_content`。

当前设计已正确分离，验证以下点：
- `ContentDelta` 只追加到 `streaming_content`，不重建 `committed_messages`
- `StreamDone` 才 flush streaming 内容到 committed
- `ToolResult` 才追加 tool 消息到 committed

- [ ] **Step 2: 修复滚动逻辑**

添加用户手动滚动检测。在 `handle_key` 中：
```rust
KeyCode::Up => {
    self.scroll_offset = self.scroll_offset.saturating_add(1);
    self.user_scrolled = true;
}
KeyCode::Down => {
    self.scroll_offset = self.scroll_offset.saturating_sub(1);
    if self.scroll_offset == 0 {
        self.user_scrolled = false;
    }
}
```

当 `user_scrolled == false` 时，自动滚到底部。当为 true 时，保持用户滚动位置不变。

- [ ] **Step 3: 添加 Ctrl+L 清屏**

```rust
KeyCode::Char('l') if ctrl => {
    self.committed_messages.clear();
    self.streaming_content.clear();
}
```

- [ ] **Step 4: 错误恢复显示**

在 `handle_event` 的 `StreamError` 分支中，将错误信息作为 system 消息追加到 `committed_messages`：
```rust
AppEvent::StreamError(msg) => {
    self.committed_messages.push(UIMessage {
        role: MessageRole::System,
        content: format!("⚠ {}", msg),
        tool_name: None,
    });
    self.streaming_active = false;
    self.status = "idle".to_string();
}
```

- [ ] **Step 5: 最终编译和端到端测试**

```bash
cargo build --release 2>&1
cargo run
```
验证完整流程：启动 → 看到 TUI → 输入消息 → 流式回复 → 工具调用执行 → 权限弹窗 → 会话管理。

- [ ] **Step 6: Commit**

```bash
git add src/tui/app.rs src/tui/components/chat.rs
git commit -m "feat: scroll stability, error recovery, and polish"
```

---

### Task 11: 最终清理 + 文档更新

**Files:**
- Modify: `CLAUDE.md`（更新架构描述）
- Modify: `Cargo.toml`（移除 TS 前端相关的注释/引用）

- [ ] **Step 1: 更新 CLAUDE.md 架构部分**

修改 "入口流程" 节，将默认路径从 `npm run dev:ink` 改为 `cargo run` 直接启动 ratatui 前端。添加 ratatui 组件说明。

- [ ] **Step 2: 验证 `cargo clippy` 无警告**

```bash
cargo clippy --all-targets -- -D warnings 2>&1
```

- [ ] **Step 3: 验证 `cargo fmt` 通过**

```bash
cargo fmt -- --check 2>&1
```

- [ ] **Step 4: 验证 `cargo test` 通过**

```bash
cargo test 2>&1
```

- [ ] **Step 5: Commit**

```bash
git add CLAUDE.md
git commit -m "docs: update CLAUDE.md for ratatui frontend architecture"
```

---

## 补充说明

1. **AgentLoop 的可变借用问题**：`AgentLoop::process_input` 需要 `&mut self`，而 UI event loop 也需要持有它。解决方案是用 `Arc<tokio::sync::Mutex<AgentLoop>>` 包装，在 `handle_event` 中 lock 后调用。

2. **System prompt**：从 `packages/core/src/agent-loop.ts:buildSystemPrompt()` 完整复制，转为 Rust 的 `&str`。包含 TodoWrite 使用说明和 skill 加载指引。

3. **权限弹窗的完整实现**（Task 7 后续）：当 `PermissionRequired` 事件触发后，AgentLoop 需要等待用户响应。实现方式是用 `tokio::sync::oneshot` channel——AgentLoop 发事件给 UI 时附带一个 oneshot sender，UI 在用户做出选择后通过这个 channel 回复。

4. **`[DONE]` 处理**：daemon 在流的末尾发送 `[DONE]` 行，`src/agent/core.rs` 的 `process_line` 已经在 `parse_sse_line` 中处理了 `[DONE]`→None 的逻辑。但 daemon 实际发送的是 `data: [DONE]`，而 daemon 的 `chat_stream` handler 会 strip `data: ` 前缀，所以前端收到的已经是 `[DONE]`。需要确认 `StreamProcessor` 能正确处理。

5. **TypeScript 前端保留**：`packages/` 目录不做任何修改。将来如需切回去，只需运行 `npm run -w packages/cli dev:ink`。
