//! TUI REPL — full-screen Ratatui agent interface.
//!
//! Drives the agent loop with streaming rendering, tool execution,
//! and interactive question overlays.

use crate::api::{ApiClient, ChatMessage, ToolCall, ToolDefinition};
use crate::cli::tui_history::{ChatHistory, HistoryEntry};
use crate::cli::tui_ime::{ImeAction, ImeHandler};
use crate::cli::tui_input::{InputBox, InputResult};
use crate::permissions::ToolPermissionPolicy;
use crate::state::AppState;
use crate::tools::{ToolExecutor, ToolRegistry};
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
    layout::{Alignment, Constraint, Direction, Layout, Margin},
    prelude::*,
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
    Terminal,
};
use std::io::Stdout;
use std::sync::Arc;
use tracing::{error, info};

/// TUI 运行模式
enum TuiMode {
    /// 正常聊天模式
    Normal,
    /// 提问模式：展示问题和选项，等待用户选择
    Question {
        question: String,
        options: Vec<(String, String)>, // (label, description)
        selected: usize,                // 当前选中索引（0-based，0 = Other）
        multi_select: bool,
        confirmed: bool,                // 用户是否已确认
        answers: Vec<usize>,            // 已选中的选项索引（多选用）
    },
}

/// TUI-based REPL application
pub struct TuiRepl {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    input: InputBox,
    history: ChatHistory,
    ime: ImeHandler,
    state: AppState,
    conversation_history: Vec<ChatMessage>,
    tool_registry: Arc<ToolRegistry>,
    tool_executor: ToolExecutor,
    should_quit: bool,
    is_processing: bool,
    /// Height of the chat area (for scroll calculations)
    chat_area_height: u16,
    /// Current TUI mode (normal or question)
    mode: TuiMode,
}

impl TuiRepl {
    pub async fn new(state: AppState) -> anyhow::Result<Self> {
        enable_raw_mode()?;
        let mut stdout = std::io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;

        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;

        let tool_registry = Arc::new(ToolRegistry::new());
        let policy = ToolPermissionPolicy::from_settings(&state.settings);

        Ok(Self {
            terminal,
            input: InputBox::new(),
            history: ChatHistory::new(),
            ime: ImeHandler::new(),
            state,
            conversation_history: Vec::new(),
            tool_executor: ToolExecutor::new(tool_registry.clone(), policy),
            tool_registry,
            should_quit: false,
            is_processing: false,
            chat_area_height: 0,
            mode: TuiMode::Normal,
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
            let area = f.area();
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Min(1),   // Chat history
                    Constraint::Length(8), // Input box
                ])
                .split(area);

            self.chat_area_height = chunks[0].height;

            // Render chat history
            self.history.render(chunks[0], f.buffer_mut());

            // Render input box
            self.input.render(chunks[1], f.buffer_mut());

            // If in question mode, render overlay
            if let TuiMode::Question { question, options, selected, multi_select, answers, .. } = &self.mode {
                Self::render_question_overlay(f, area, question, options, *selected, *multi_select, answers);
            }
        })?;

        Ok(())
    }

    async fn handle_events(&mut self) -> anyhow::Result<()> {
        if event::poll(std::time::Duration::from_millis(50))? {
            let event = event::read()?;

            // Question mode takes priority over everything
            if self.handle_question_event(&event).await? {
                return Ok(());
            }

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
            let mut reasoning_content = String::new();
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

                            if let Some(rc) = &choice.delta.reasoning_content {
                                reasoning_content.push_str(rc);
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

                let rc = if reasoning_content.is_empty() { None } else { Some(reasoning_content.clone()) };

                let assistant_msg = ChatMessage {
                    role: "assistant".to_string(),
                    content: if full_content.is_empty() {
                        None
                    } else {
                        Some(full_content)
                    },
                    reasoning_content: rc,
                    tool_calls: Some(tool_calls_parsed),
                    tool_call_id: None,
                };
                self.conversation_history.push(assistant_msg);

                // Execute each tool and show status
                let tool_calls = self.conversation_history.last().unwrap().tool_calls.clone().unwrap();
                for tc in &tool_calls {
                    let tool_name = tc.function.name.clone();
                    let tool_call_id = tc.id.clone();

                    // Special handling for ask_user_question (interactive)
                    if tool_name == "ask_user_question" {
                        let args: serde_json::Value =
                            serde_json::from_str(&tc.function.arguments)
                                .unwrap_or(serde_json::json!({}));
                        let options = Self::parse_options(args.get("options"));
                        let question = args
                            .get("question")
                            .and_then(|v| v.as_str())
                            .unwrap_or("请选择一个选项:")
                            .to_string();
                        let multi_select = args
                            .get("multiSelect")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);

                        // Switch to question mode
                        self.mode = TuiMode::Question {
                            question,
                            options,
                            selected: 0,
                            multi_select,
                            confirmed: false,
                            answers: Vec::new(),
                        };

                        // Wait for user to answer (draw + handle events)
                        while let TuiMode::Question { confirmed: false, .. } = self.mode {
                            self.draw()?;
                            Box::pin(self.handle_events()).await?;
                        }

                        // Build answer JSON
                        let result = match &self.mode {
                            TuiMode::Question {
                                answers,
                                options,
                                multi_select,
                                ..
                            } => {
                                let answer_objs: Vec<serde_json::Value> = if *multi_select {
                                    answers
                                        .iter()
                                        .map(|idx| {
                                            if *idx >= options.len() {
                                                serde_json::json!({
                                                    "label": "Other",
                                                    "value": "",
                                                    "custom": true
                                                })
                                            } else {
                                                let (label, _) = &options[*idx];
                                                serde_json::json!({
                                                    "label": label,
                                                    "value": label,
                                                    "custom": false
                                                })
                                            }
                                        })
                                        .collect()
                                } else {
                                    if let Some(idx) = answers.first() {
                                        if *idx >= options.len() {
                                            vec![serde_json::json!({
                                                "label": "Other",
                                                "value": "",
                                                "custom": true
                                            })]
                                        } else {
                                            let (label, _) = &options[*idx];
                                            vec![serde_json::json!({
                                                "label": label,
                                                "value": label,
                                                "custom": false
                                            })]
                                        }
                                    } else {
                                        vec![]
                                    }
                                };
                                serde_json::json!({
                                    "success": true,
                                    "answers": answer_objs
                                })
                                .to_string()
                            }
                            _ => unreachable!(),
                        };

                        // Return to normal mode
                        self.mode = TuiMode::Normal;

                        // Show what the user selected in history
                        let answer_labels = match &result {
                            r if r.contains("answers") => {
                                let parsed: serde_json::Value = serde_json::from_str(r).unwrap_or_default();
                                parsed["answers"]
                                    .as_array()
                                    .map(|arr| {
                                        arr.iter()
                                            .filter_map(|a| a["label"].as_str())
                                            .collect::<Vec<_>>()
                                            .join(", ")
                                    })
                                    .unwrap_or_default()
                            }
                            _ => String::new(),
                        };
                        if !answer_labels.is_empty() {
                            self.history.add(HistoryEntry::System(format!(
                                "选择了: {}",
                                answer_labels
                            )));
                        }

                        let tool_result_msg = ChatMessage::tool(&tool_call_id, result);
                        self.conversation_history.push(tool_result_msg);
                        continue;
                    }

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
                let mut msg = ChatMessage::assistant(&full_content);
                if !reasoning_content.is_empty() {
                    msg.reasoning_content = Some(reasoning_content);
                }
                self.conversation_history.push(msg);
            }

            self.draw()?;
            break;
        }

        Ok(())
    }

    async fn get_tool_definitions(&self) -> Vec<ToolDefinition> {
        self.tool_executor.tool_definitions()
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

    /// Parse question options from JSON args
    fn parse_options(value: Option<&serde_json::Value>) -> Vec<(String, String)> {
        let mut options = Vec::new();
        if let Some(serde_json::Value::Array(arr)) = value {
            for opt in arr {
                let label = opt
                    .get("label")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Option")
                    .to_string();
                let desc = opt
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                options.push((label, desc));
            }
        }
        options
    }

    /// Render a question overlay popup in the center of the terminal
    fn render_question_overlay(
        frame: &mut Frame,
        area: Rect,
        question: &str,
        options: &[(String, String)],
        selected: usize,
        multi_select: bool,
        answers: &[usize],
    ) {
        let popup_width = ((area.width as f32 * 0.8).max(60.0).min(120.0) as u16)
            .min(area.width.saturating_sub(4));
        let option_count = options.len() + 1; // +1 for Other
        let min_height = 10u16;
        let content_height = (question.lines().count() as u16)
            .saturating_add(option_count as u16 * 2 + 4)
            .max(min_height)
            .min(area.height.saturating_sub(4));
        let popup_area = Rect {
            x: area.x + (area.width - popup_width) / 2,
            y: area.y + (area.height - content_height) / 2,
            width: popup_width,
            height: content_height,
        };

        Clear.render(popup_area, frame.buffer_mut());

        let block = Block::default()
            .title(" ❓ Question ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Rgb(255, 200, 100)))
            .style(Style::default().bg(Color::Rgb(30, 30, 35)));
        block.render(popup_area, frame.buffer_mut());

        let inner = popup_area.inner(Margin::new(2, 1));

        // Question text
        let question_para = Paragraph::new(question)
            .style(Style::default().fg(Color::Rgb(220, 200, 255)).add_modifier(Modifier::BOLD))
            .wrap(Wrap { trim: true });
        let question_height = question.lines().count() as u16;
        let question_area = Rect::new(inner.x, inner.y, inner.width, question_height);
        question_para.render(question_area, frame.buffer_mut());

        // Options list
        let mut list_items: Vec<ListItem> = Vec::new();
        for (i, (label, desc)) in options.iter().enumerate() {
            let is_selected = i == selected;
            let is_checked = answers.contains(&i);
            let check_mark = if multi_select {
                if is_checked { "[x] " } else { "[ ] " }
            } else {
                if is_selected { "● " } else { "○ " }
            };
            let num = format!("{:2}. ", i + 1);
            let text = format!("{}{}{} - {}", check_mark, num, label, desc);
            let style = if is_selected {
                Style::default()
                    .fg(Color::White)
                    .bg(Color::Rgb(80, 60, 120))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Rgb(180, 180, 180))
            };
            list_items.push(ListItem::new(text).style(style));
        }
        // Other option
        let other_idx = options.len();
        let is_other_selected = selected == other_idx;
        let is_other_checked = answers.contains(&other_idx);
        let check_mark = if multi_select {
            if is_other_checked { "[x] " } else { "[ ] " }
        } else {
            if is_other_selected { "● " } else { "○ " }
        };
        let text = format!("{:2}. Other - 输入自定义答案", other_idx + 1);
        let style = if is_other_selected {
            Style::default()
                .fg(Color::White)
                .bg(Color::Rgb(80, 60, 120))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Rgb(180, 180, 180))
        };
        list_items.push(ListItem::new(format!("{}{}", check_mark, text)).style(style));

        let list = List::new(list_items);
        let list_area = Rect::new(
            inner.x,
            inner.y + question_height + 1,
            inner.width,
            content_height.saturating_sub(question_height + 3),
        );
        ratatui::widgets::Widget::render(list, list_area, frame.buffer_mut());

        // Bottom hint
        let hint = if multi_select {
            "↑↓选择 · 空格勾选 · Enter确认 · Esc取消 · 1-9直接选择"
        } else {
            "↑↓选择 · Enter确认 · Esc取消 · 1-9直接选择"
        };
        let hint_para = Paragraph::new(hint)
            .alignment(Alignment::Center)
            .style(Style::default().fg(Color::Rgb(150, 150, 150)).add_modifier(Modifier::ITALIC));
        let hint_area = Rect::new(
            inner.x,
            inner.y + content_height.saturating_sub(2),
            inner.width,
            1,
        );
        hint_para.render(hint_area, frame.buffer_mut());
    }

    /// Handle keyboard/mouse events when in question mode.
    /// Returns true if the event was consumed (question mode handled it).
    async fn handle_question_event(&mut self, event: &Event) -> anyhow::Result<bool> {
        let mut should_confirm = false;
        let mut canceled = false;

        if let TuiMode::Question {
            ref mut selected,
            ref mut answers,
            multi_select,
            ref mut confirmed,
            ref options,
            ..
        } = self.mode
        {
            match event {
                Event::Key(key) => {
                    match key.code {
                        KeyCode::Up => {
                            *selected = selected.saturating_sub(1);
                        }
                        KeyCode::Down => {
                            let max_idx = options.len();
                            if *selected < max_idx {
                                *selected += 1;
                            }
                        }
                        KeyCode::Enter => {
                            if multi_select {
                                // 如果 answers 为空但 selected 有效，自动加入
                                if answers.is_empty() && !options.is_empty() {
                                    answers.push(*selected);
                                }
                            } else {
                                answers.clear();
                                answers.push(*selected);
                            }
                            should_confirm = true;
                        }
                        KeyCode::Esc => {
                            answers.clear();
                            canceled = true;
                        }
                        KeyCode::Char(' ') if multi_select => {
                            if answers.contains(&*selected) {
                                answers.retain(|x| *x != *selected);
                            } else {
                                answers.push(*selected);
                            }
                        }
                        KeyCode::Char(c) if c.is_ascii_digit() => {
                            let num = c.to_digit(10).unwrap() as usize;
                            let max_num = options.len() + 1;
                            if num > 0 && num <= max_num {
                                *selected = num - 1;
                                if !multi_select {
                                    answers.clear();
                                    answers.push(*selected);
                                    should_confirm = true;
                                }
                            }
                        }
                        _ => {}
                    }
                }
                Event::Mouse(mouse) => {
                    if let MouseEventKind::Down(_) = mouse.kind {
                        // Simple: click anywhere confirms current selection
                        // (accurate hit-testing would require knowing rendered positions)
                        if !multi_select {
                            answers.clear();
                            answers.push(*selected);
                            should_confirm = true;
                        }
                    }
                }
                _ => {}
            }

            if should_confirm || canceled {
                *confirmed = true;
            }
            self.draw()?;
            return Ok(true);
        }

        Ok(false)
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
