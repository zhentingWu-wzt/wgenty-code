//! Terminal Module - TUI REPL with Ratatui

pub mod history;
pub mod ime;
pub mod input;

use crate::api::{ApiClient, ChatMessage, ToolCall, ToolDefinition};
use crate::state::AppState;
use crate::terminal::history::{ChatHistory, HistoryEntry};
use crate::terminal::ime::{ImeAction, ImeHandler};
use crate::terminal::input::{InputBox, InputResult};
use crate::tools::ToolRegistry;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers, MouseEvent,
        MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures::StreamExt;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    prelude::*,
    Terminal,
};
use std::io::Stdout;
use std::sync::Arc;
use tracing::{error, info};

/// TUI-based REPL application
pub struct TuiRepl {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    input: InputBox,
    history: ChatHistory,
    ime: ImeHandler,
    state: AppState,
    conversation_history: Vec<ChatMessage>,
    tool_registry: Arc<ToolRegistry>,
    should_quit: bool,
    is_processing: bool,
    /// Height of the chat area (for scroll calculations)
    chat_area_height: u16,
}

impl TuiRepl {
    pub async fn new(state: AppState) -> anyhow::Result<Self> {
        enable_raw_mode()?;
        let mut stdout = std::io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;

        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;

        let tool_registry = Arc::new(ToolRegistry::new());

        Ok(Self {
            terminal,
            input: InputBox::new(),
            history: ChatHistory::new(),
            ime: ImeHandler::new(),
            state,
            conversation_history: Vec::new(),
            tool_registry,
            should_quit: false,
            is_processing: false,
            chat_area_height: 0,
        })
    }

    pub async fn run(&mut self) -> anyhow::Result<()> {
        self.history.add(HistoryEntry::Welcome {
            model: self.state.settings.api.get_model_id(&self.state.settings.model),
        });

        while !self.should_quit {
            self.draw()?;
            self.handle_events().await?;
        }

        Ok(())
    }

    fn draw(&mut self) -> anyhow::Result<()> {
        self.terminal.draw(|f| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Min(1),   // Chat history
                    Constraint::Length(8), // Input box
                ])
                .split(f.area());

            self.chat_area_height = chunks[0].height;

            // Render chat history
            self.history.render(chunks[0], f.buffer_mut());

            // Render input box
            self.input.render(chunks[1], f.buffer_mut());
        })?;

        Ok(())
    }

    async fn handle_events(&mut self) -> anyhow::Result<()> {
        if event::poll(std::time::Duration::from_millis(50))? {
            let event = event::read()?;

            match self.ime.handle_event(&event) {
                ImeAction::Committed(text) => {
                    self.input.insert_ime_text(&text);
                    self.draw()?;
                }
                ImeAction::Passthrough => {
                    match event {
                        Event::Key(key) => {
                            match key.code {
                                KeyCode::Char('c')
                                    if key.modifiers.contains(KeyModifiers::CONTROL) =>
                                {
                                    self.should_quit = true;
                                    return Ok(());
                                }
                                // Scrolling keys
                                KeyCode::PageUp => {
                                    let scroll_amount = self.chat_area_height.saturating_sub(2);
                                    self.history.scroll_up(scroll_amount);
                                }
                                KeyCode::PageDown => {
                                    let scroll_amount = self.chat_area_height.saturating_sub(2);
                                    self.history.scroll_down(scroll_amount);
                                }
                                KeyCode::Up if key.modifiers.contains(KeyModifiers::SHIFT) => {
                                    self.history.scroll_up(3);
                                }
                                KeyCode::Down if key.modifiers.contains(KeyModifiers::SHIFT) => {
                                    self.history.scroll_down(3);
                                }
                                KeyCode::Home => {
                                    self.history.scroll_to_top();
                                }
                                KeyCode::End => {
                                    self.history.scroll_to_bottom();
                                }
                                _ => {
                                    if !self.is_processing {
                                        match self.input.input(key) {
                                            InputResult::Submitted(text) => {
                                                if !text.trim().is_empty() {
                                                    self.submit(text).await?;
                                                }
                                            }
                                            InputResult::Continue => {}
                                        }
                                    }
                                }
                            }
                            self.draw()?;
                        }
                        Event::Mouse(mouse) => {
                            self.handle_mouse_event(mouse);
                            self.draw()?;
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }

        Ok(())
    }

    fn handle_mouse_event(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::ScrollUp => {
                self.history.scroll_up(3);
            }
            MouseEventKind::ScrollDown => {
                self.history.scroll_down(3);
            }
            _ => {}
        }
    }

    async fn submit(&mut self, text: String) -> anyhow::Result<()> {
        let trimmed = text.trim().to_string();

        self.history.add(HistoryEntry::User(trimmed.clone()));

        match trimmed.as_str() {
            "exit" | "quit" | ".exit" | ":q" => {
                self.should_quit = true;
                return Ok(());
            }
            "clear" | ".clear" | ":c" => {
                self.history.clear();
                return Ok(());
            }
            "reset" | ".reset" => {
                self.conversation_history.clear();
                self.history.add(HistoryEntry::System("对话已重置".to_string()));
                return Ok(());
            }
            _ => {}
        }

        self.is_processing = true;
        self.draw()?;

        self.process_loop(&trimmed).await?;

        self.is_processing = false;
        Ok(())
    }

    async fn process_loop(&mut self, input: &str) -> anyhow::Result<()> {
        let client = ApiClient::new(self.state.settings.clone());

        if client.get_api_key().is_none() {
            self.history
                .add(HistoryEntry::System("API key 未配置".to_string()));
            return Ok(());
        }

        self.conversation_history.push(ChatMessage::user(input));

        let tools = self.get_tool_definitions().await;
        let tools_opt = if tools.is_empty() {
            None
        } else {
            Some(tools)
        };

        loop {
            // Show thinking indicator while waiting for API response
            self.history.add(HistoryEntry::Thinking { frame: 0, elapsed_secs: 0 });
            self.draw()?;

            let messages = self.conversation_history.clone();
            let response_fut = client.chat_stream(messages, tools_opt.clone());

            // Drive the thinking animation while waiting for the response
            let mut response = Box::pin(response_fut);
            let mut thinking_tick = tokio::time::interval(std::time::Duration::from_millis(120));
            // Consume the first immediate tick
            thinking_tick.tick().await;

            let response = loop {
                tokio::select! {
                    r = &mut response => {
                        break r;
                    }
                    _ = thinking_tick.tick() => {
                        self.history.advance_thinking();
                        self.draw()?;
                    }
                }
            };

            // Remove thinking indicator before showing result
            self.history.remove_thinking();

            let response = match response {
                Ok(r) => r,
                Err(e) => {
                    error!(error = %e, "tui stream request failed");
                    self.history
                        .add(HistoryEntry::System(format!("请求失败: {}", e)));
                    self.draw()?;
                    return Ok(());
                }
            };

            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                error!(status = %status, "tui api returned non-success status");
                self.history
                    .add(HistoryEntry::System(format!("API 错误 ({}): {}", status, body)));
                self.draw()?;
                return Ok(());
            }

            // Add streaming entry — but only if there's content to show
            // When the model only returns tool_calls with no content, skip the empty bubble
            let mut full_content = String::new();
            let mut tool_calls_accum: Vec<serde_json::Value> = Vec::new();
            let mut has_tool_calls = false;
            let mut streaming_entry_added = false;

            let mut stream = response.bytes_stream();
            let mut buffer = String::new();

            while let Some(chunk) = stream.next().await {
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(e) => {
                        error!(error = %e, "stream chunk error");
                        break;
                    }
                };
                buffer.push_str(&String::from_utf8_lossy(&chunk));

                while let Some(pos) = buffer.find('\n') {
                    let line = buffer[..pos].trim().to_string();
                    buffer = buffer[pos + 1..].to_string();

                    if let Some(stream_chunk) = crate::api::parse_sse_line(&line) {
                        if let Some(choice) = stream_chunk.choices.first() {
                            if let Some(content) = &choice.delta.content {
                                full_content.push_str(content);
                                if !streaming_entry_added {
                                    self.history.add(HistoryEntry::AssistantStreaming(String::new()));
                                    streaming_entry_added = true;
                                }
                                self.history.update_last_streaming(&full_content);
                                self.draw()?;
                            }

                            if let Some(tc_deltas) = &choice.delta.tool_calls {
                                has_tool_calls = true;
                                for tc in tc_deltas {
                                    let idx = tc.index as usize;
                                    while tool_calls_accum.len() <= idx {
                                        tool_calls_accum.push(serde_json::json!({
                                            "id": null,
                                            "type": "function",
                                            "function": {"name": "", "arguments": ""}
                                        }));
                                    }
                                    let entry = &mut tool_calls_accum[idx];
                                    if let Some(id) = &tc.id {
                                        entry["id"] = serde_json::Value::String(id.clone());
                                    }
                                    if let Some(func) = &tc.function {
                                        if let Some(name) = &func.name {
                                            entry["function"]["name"] =
                                                serde_json::Value::String(name.clone());
                                        }
                                        if let Some(args) = &func.arguments {
                                            if let Some(existing) = entry["function"]["arguments"].as_str() {
                                                let mut combined = existing.to_string();
                                                combined.push_str(args);
                                                entry["function"]["arguments"] =
                                                    serde_json::Value::String(combined);
                                            }
                                        }
                                    }
                                }
                            }

                            if choice.finish_reason.as_deref() == Some("tool_calls")
                                || choice.finish_reason.as_deref() == Some("stop")
                            {
                                break;
                            }
                        }
                    }
                }
            }

            // Finalize streaming entry (only if one was added)
            if streaming_entry_added {
                self.history.finalize_last_streaming();
            }

            if has_tool_calls && !tool_calls_accum.is_empty() {
                info!(tool_call_count = tool_calls_accum.len(), "model requested tool calls");

                let tool_calls_parsed: Vec<ToolCall> = tool_calls_accum
                    .iter()
                    .filter_map(|call| {
                        let id = call.get("id")?.as_str()?.to_string();
                        let r#type = call.get("type")?.as_str()?.to_string();
                        let func = call.get("function")?;
                        let name = func.get("name")?.as_str()?.to_string();
                        let arguments = func.get("arguments")?.as_str()?.to_string();
                        Some(ToolCall {
                            id,
                            r#type,
                            function: crate::api::ToolCallFunction { name, arguments },
                        })
                    })
                    .collect();

                let assistant_msg = ChatMessage {
                    role: "assistant".to_string(),
                    content: if full_content.is_empty() {
                        None
                    } else {
                        Some(full_content)
                    },
                    tool_calls: Some(tool_calls_parsed),
                    tool_call_id: None,
                };
                self.conversation_history.push(assistant_msg);

                // Execute each tool and show status
                let tool_calls = self.conversation_history.last().unwrap().tool_calls.clone().unwrap();
                for tc in &tool_calls {
                    // Show thinking indicator while executing tool
                    self.history.add(HistoryEntry::Thinking { frame: 0, elapsed_secs: 0 });
                    self.draw()?;

                    let args: serde_json::Value =
                        serde_json::from_str(&tc.function.arguments)
                            .unwrap_or(serde_json::json!({}));
                    let tool_name = tc.function.name.clone();
                    let tool_summary = Self::summarize_tool_args(&tool_name, &args);
                    let tool_call_id = tc.id.clone();
                    let registry = self.tool_registry.clone();

                    // Drive thinking animation during tool execution
                    let tool_name_for_fut = tool_name.clone();
                    let tool_fut = registry.execute(&tool_name_for_fut, args);
                    let mut tool_fut = Box::pin(tool_fut);
                    let mut thinking_tick = tokio::time::interval(std::time::Duration::from_millis(120));
                    thinking_tick.tick().await;

                    let result = loop {
                        tokio::select! {
                            r = &mut tool_fut => {
                                break r;
                            }
                            _ = thinking_tick.tick() => {
                                self.history.advance_thinking();
                                self.draw()?;
                            }
                        }
                    };

                    // Remove thinking, show tool result
                    self.history.remove_thinking();

                    let success = result.is_ok();
                    self.history.add(HistoryEntry::ToolCall {
                        name: tool_name,
                        summary: tool_summary,
                        success,
                    });
                    self.draw()?;

                    let tool_result_str = match result {
                        Ok(v) => serde_json::json!({
                            "success": true,
                            "output_type": v.output_type,
                            "content": v.content,
                            "metadata": v.metadata
                        })
                        .to_string(),
                        Err(e) => serde_json::json!({
                            "success": false,
                            "error": {
                                "message": e.message,
                                "code": e.code
                            }
                        })
                        .to_string(),
                    };
                    let tool_result_msg = ChatMessage::tool(&tool_call_id, tool_result_str);
                    self.conversation_history.push(tool_result_msg);
                }

                continue;
            }

            if !full_content.is_empty() {
                info!(response_len = full_content.len(), "received streaming response in tui");
                self.conversation_history
                    .push(ChatMessage::assistant(&full_content));
            }

            self.draw()?;
            break;
        }

        Ok(())
    }

    async fn get_tool_definitions(&self) -> Vec<ToolDefinition> {
        let tools = self.tool_registry.list();
        tools
            .into_iter()
            .map(|t| ToolDefinition::new(t.name(), t.description(), t.input_schema()))
            .collect()
    }

    fn summarize_tool_args(name: &str, args: &serde_json::Value) -> Option<String> {
        fn quoted(value: &str) -> String {
            format!("\"{}\"", value)
        }

        fn truncate(value: &str, max_chars: usize) -> String {
            let mut chars = value.chars();
            let truncated: String = chars.by_ref().take(max_chars).collect();
            if chars.next().is_some() {
                format!("{}...", truncated)
            } else {
                truncated
            }
        }

        match name {
            "file_read" | "file_write" | "file_edit" => args["path"]
                .as_str()
                .map(|path| format!("path={}", quoted(path))),
            "search" => {
                let pattern = args["pattern"].as_str();
                let path = args["path"].as_str();
                match (pattern, path) {
                    (Some(pattern), Some(path)) => Some(format!(
                        "pattern={} path={}",
                        quoted(&truncate(pattern, 40)),
                        quoted(path)
                    )),
                    (Some(pattern), None) => {
                        Some(format!("pattern={}", quoted(&truncate(pattern, 40))))
                    }
                    (None, Some(path)) => Some(format!("path={}", quoted(path))),
                    (None, None) => None,
                }
            }
            "execute_command" => args["command"]
                .as_str()
                .map(|command| format!("command={}", quoted(&truncate(command, 60)))),
            "list_files" => args["path"]
                .as_str()
                .map(|path| format!("path={}", quoted(path))),
            "git_operations" => {
                let operation = args["operation"].as_str();
                let path = args["path"].as_str();
                match (operation, path) {
                    (Some(operation), Some(path)) => {
                        Some(format!("operation={} path={}", quoted(operation), quoted(path)))
                    }
                    (Some(operation), None) => Some(format!("operation={}", quoted(operation))),
                    (None, Some(path)) => Some(format!("path={}", quoted(path))),
                    (None, None) => None,
                }
            }
            _ => None,
        }
    }
}

impl Drop for TuiRepl {
    fn drop(&mut self) {
        execute!(self.terminal.backend_mut(), DisableMouseCapture).ok();
        disable_raw_mode().ok();
        execute!(self.terminal.backend_mut(), LeaveAlternateScreen).ok();
        self.terminal.show_cursor().ok();
    }
}
