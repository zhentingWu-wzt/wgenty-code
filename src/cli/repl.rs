//! REPL Module - Interactive Read-Eval-Print Loop
//!
//! Beautiful REPL interface matching the original Claude Code aesthetic
//!
//! Uses a dedicated stdin thread + mpsc channel for concurrent input during
//! streaming. This keeps the terminal in cooked mode so CJK/IME works normally.

use crate::api::{ApiClient, ChatMessage, ToolCall, ToolDefinition};
use crate::cli::ui;
use crate::state::AppState;
use crate::permissions::{PermissionRequest, PolicyDecision, ToolPermissionPolicy};
use crate::tools::{ToolExecutor, ToolRegistry};
use colored::Colorize;
use futures::StreamExt;
use std::io::{self, BufRead, Write};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

pub struct Repl {
    state: AppState,
    conversation_history: Vec<ChatMessage>,
    tool_executor: ToolExecutor,
    stdin_rx: mpsc::Receiver<String>,
}

impl Repl {
    pub async fn new(state: AppState) -> Self {
        ui::init_terminal();
        let tool_registry = Arc::new(ToolRegistry::new());

        let (stdin_tx, stdin_rx) = mpsc::channel::<String>(16);

        // Dedicated stdin thread: reads key events in raw mode for proper
        // CJK/Unicode backspace handling, falls back to cooked read_line.
        std::thread::spawn(move || {
            use crossterm::event::{read, Event, KeyCode, KeyModifiers};
            use crossterm::terminal::{disable_raw_mode, enable_raw_mode};

            // Try raw mode first
            if enable_raw_mode().is_err() {
                // Fallback: standard cooked mode read_line
                let stdin = io::stdin();
                loop {
                    let mut line = String::new();
                    match stdin.lock().read_line(&mut line) {
                        Ok(0) => break,
                        Ok(_) => {
                            let trimmed = line.trim().to_string();
                            if stdin_tx.blocking_send(trimmed).is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
                return;
            }

            // Re-enable OPOST so \n works correctly
            #[cfg(unix)]
            unsafe {
                let mut term: libc::termios = std::mem::zeroed();
                if libc::tcgetattr(libc::STDIN_FILENO, &mut term) == 0 {
                    term.c_oflag |= libc::OPOST;
                    libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &term);
                }
            }

            let mut line = String::new();

            loop {
                match read() {
                    Ok(Event::Key(key)) => {
                        match key.code {
                            KeyCode::Enter => {
                                let trimmed = line.trim().to_string();
                                if stdin_tx.blocking_send(trimmed).is_err() {
                                    break;
                                }
                                line.clear();
                                println!();
                            }
                            KeyCode::Backspace => {
                                if !line.is_empty() {
                                    line.pop();
                                    print!(
                                        "\r\x1B[K  {} {line}",
                                        "▸".truecolor(255, 140, 66)
                                    );
                                    io::stdout().flush().unwrap();
                                }
                            }
                            KeyCode::Char(c) => {
                                if key.modifiers.contains(KeyModifiers::CONTROL) && c == 'c' {
                                    let _ = disable_raw_mode();
                                    std::process::exit(130);
                                }
                                line.push(c);
                                print!("{c}");
                                io::stdout().flush().unwrap();
                            }
                            _ => {}
                        }
                    }
                    Ok(Event::Resize(_, _)) => {}
                    Err(_) => break,
                    _ => {}
                }
            }

            let _ = disable_raw_mode();
        });

        let policy = ToolPermissionPolicy::from_settings(&state.settings);

        info!("repl initialized");

        Self {
            state,
            conversation_history: Vec::new(),
            tool_executor: ToolExecutor::new(tool_registry.clone(), policy),
            stdin_rx,
        }
    }

    pub async fn start(&mut self, initial_prompt: Option<String>) -> anyhow::Result<()> {
        ui::print_welcome(
            &self
                .state
                .settings
                .api
                .get_model_id(&self.state.settings.model),
        );

        if let Some(prompt) = initial_prompt {
            self.process_input(&prompt).await?;
        }

        loop {
            print!("  {} ", "▸".truecolor(255, 140, 66));
            io::stdout().flush().ok();

            let input = match self.stdin_rx.recv().await {
                Some(input) => input,
                None => break, // channel closed = EOF
            };

            if input.is_empty() {
                println!();
                continue;
            }

            println!();

            match input.as_str() {
                "exit" | "quit" | ".exit" | ":q" => {
                    println!();
                    println!("  ╭────────────────────────────╮");
                    println!(
                        "  │   {} {} │",
                        "👋".yellow(),
                        "再见！祝你编码愉快".truecolor(255, 140, 66).bold()
                    );
                    println!("  ╰────────────────────────────╯");
                    println!();
                    break;
                }
                "help" | ".help" | ":h" => ui::print_help(),
                "status" | ".status" => self.print_status(),
                "clear" | ".clear" | ":c" => ui::clear_screen(),
                "history" | ".history" => self.print_history(),
                "reset" | ".reset" => self.reset_conversation(),
                "config" | ".config" => self.print_config(),
                _ => self.process_input(&input).await?,
            }
        }

        Ok(())
    }

    async fn process_input(&mut self, input: &str) -> anyhow::Result<()> {
        ui::print_user_message(input);
        info!(
            input_len = input.len(),
            conversation_messages = self.conversation_history.len(),
            "processing repl input"
        );

        let client = ApiClient::new(self.state.settings.clone());

        if client.get_api_key().is_none() {
            warn!("repl request skipped because api key is not configured");
            ui::print_error("API key not configured\n\nSet it with:\n  claude-code config set api_key \"your-api-key\"");
            return Ok(());
        }

        self.conversation_history.push(ChatMessage::user(input));
        let tools = self.get_tool_definitions().await;
        let tools_opt = if tools.is_empty() {
            None
        } else {
            Some(tools)
        };

        // Tool call loop
        let mut pending_input: Option<String> = None;
        loop {
            let indicator = ui::ThinkingIndicator::start();

            let messages = self.conversation_history.clone();
            let response = match client.chat_stream(messages, tools_opt.clone()).await {
                Ok(r) => {
                    indicator.stop().await;
                    r
                }
                Err(e) => {
                    indicator.stop().await;
                    error!(error = %e, "repl stream request failed");
                    ui::print_error(&format!("Request failed: {}", e));
                    return Ok(());
                }
            };

            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                error!(status = %status, "repl api returned non-success status");
                ui::print_error(&format!("API error ({}): {}", status, body));
                return Ok(());
            }

            // Stream with concurrent input
            let result = self.stream_with_input(response).await?;

            if result.has_tool_calls && !result.tool_calls_accum.is_empty() {
                info!(tool_call_count = result.tool_calls_accum.len(), "model requested tool calls");
                println!();
                for tc in &result.tool_calls_accum {
                    let tool_name = tc["function"]["name"].as_str().unwrap_or("unknown");
                    let args_str = tc["function"]["arguments"].as_str().unwrap_or("{}");
                    let args: serde_json::Value =
                        serde_json::from_str(args_str).unwrap_or(serde_json::json!({}));

                    let detail = match tool_name {
                        "file_read" | "file_edit" | "file_write" => {
                            args.get("path").and_then(|v| v.as_str()).map(|s| format!(" → {}", s))
                        }
                        "execute_command" => {
                            args.get("command").and_then(|v| v.as_str()).map(|s| format!(" → `{}`", s))
                        }
                        "search" => {
                            let pattern = args.get("pattern").and_then(|v| v.as_str());
                            let path = args.get("path").and_then(|v| v.as_str());
                            match (pattern, path) {
                                (Some(p), Some(dir)) => Some(format!(" → `{}` in {}", p, dir)),
                                (Some(p), None) => Some(format!(" → `{}`", p)),
                                _ => None,
                            }
                        }
                        "list_files" => {
                            args.get("path").and_then(|v| v.as_str()).map(|s| format!(" → {}", s))
                        }
                        "git_operations" => {
                            args.get("operation").and_then(|v| v.as_str()).map(|s| format!(" → {}", s))
                        }
                        "note_edit" => {
                            let op = args.get("operation").and_then(|v| v.as_str()).unwrap_or("unknown");
                            match op {
                                "create" => args.get("title").and_then(|v| v.as_str()).map(|s| format!(" → {}", s)),
                                "search" => args.get("query").and_then(|v| v.as_str()).map(|s| format!(" → `{}`", s)),
                                _ => args.get("note_id").and_then(|v| v.as_str()).map(|s| format!(" → {}", s)),
                            }
                        }
                        "task_management" => {
                            let op = args.get("operation").and_then(|v| v.as_str()).unwrap_or("unknown");
                            match op {
                                "create" => args.get("subject").and_then(|v| v.as_str()).map(|s| format!(" → {}", s)),
                                _ => args.get("task_id").and_then(|v| v.as_str()).map(|s| format!(" → {}", s)),
                            }
                        }
                        "view" => {
                            let path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
                            let depth = args.get("depth").and_then(|v| v.as_u64()).unwrap_or(3);
                            Some(format!(" → {} (depth: {})", path, depth))
                        }
                        "think" => {
                            args.get("thought")
                                .and_then(|v| v.as_str())
                                .map(|s| {
                                    let preview: String = s.chars().take(60).collect();
                                    if s.len() > 60 { format!(" → {}...", preview) } else { format!(" → {}", preview) }
                                })
                        }
                        "lsp" => {
                            let op = args.get("operation").and_then(|v| v.as_str()).unwrap_or("?");
                            let sym = args.get("symbol").and_then(|v| v.as_str()).unwrap_or("?");
                            Some(format!(" → {} `{}`", op, sym))
                        }
                        _ => None,
                    };

                    let detail_str = detail.unwrap_or_default();
                    println!(
                        "  {} {} {}",
                        "▸".truecolor(255, 200, 100),
                        tool_name.cyan().bold(),
                        detail_str.truecolor(180, 180, 180)
                    );
                }

                let tool_calls_parsed: Vec<ToolCall> = result
                    .tool_calls_accum
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

                let reasoning = if result.reasoning_content.is_empty() {
                    None
                } else {
                    Some(result.reasoning_content.clone())
                };

                let assistant_msg = ChatMessage {
                    role: "assistant".to_string(),
                    content: if result.content.is_empty() {
                        None
                    } else {
                        Some(result.content)
                    },
                    reasoning_content: reasoning,
                    tool_calls: Some(tool_calls_parsed),
                    tool_call_id: None,
                };
                self.conversation_history.push(assistant_msg);

                for tc in &self.conversation_history.last().unwrap().tool_calls.clone().unwrap() {
                    let args: serde_json::Value =
                        serde_json::from_str(&tc.function.arguments)
                            .unwrap_or(serde_json::json!({}));
                    let tool_result = if tc.function.name == "ask_user_question" {
                        self.execute_ask_user_question(args).await
                    } else {
                        self.execute_tool(&tc.function.name, args).await
                    };
                    let tool_result_msg = ChatMessage::tool(&tc.id, tool_result);
                    self.conversation_history.push(tool_result_msg);
                }

                if result.pending_input.is_some() {
                    pending_input = result.pending_input;
                }

                continue;
            }

            // Normal response — save to history
            if !result.content.is_empty() {
                info!(response_len = result.content.len(), "received streaming response");
                let mut msg = ChatMessage::assistant(result.content);
                if !result.reasoning_content.is_empty() {
                    msg.reasoning_content = Some(result.reasoning_content);
                }
                self.conversation_history.push(msg);
            }

            if result.pending_input.is_some() {
                pending_input = result.pending_input;
            }

            break;
        }

        // If user typed something during streaming, process it as a new input
        if let Some(pending) = pending_input {
            println!(
                "  {} {}",
                "▸".truecolor(255, 200, 100),
                "(input captured during streaming)".bright_black().italic()
            );
            println!("  {} {}", "▸".truecolor(255, 140, 66), pending);
            Box::pin(self.process_input(&pending)).await?;
        }

        Ok(())
    }

    /// Stream the response while allowing concurrent keyboard input.
    /// Uses tokio::select! to multiplex SSE chunks and stdin channel.
    /// Terminal stays in cooked mode — CJK/IME works normally.
    async fn stream_with_input(
        &mut self,
        response: reqwest::Response,
    ) -> anyhow::Result<StreamResult> {
        let mut line_state = ui::StreamLineState::new();
        let mut full_content = String::with_capacity(4096);
        let mut reasoning_content = String::with_capacity(4096);
        let mut tool_calls_accum: Vec<serde_json::Value> = Vec::new();
        let mut has_tool_calls = false;
        let mut pending_input: Option<String> = None;
        let mut stream_done = false;

        let mut stream = response.bytes_stream();
        let mut buffer = String::with_capacity(4096);
        let mut flush_tick = tokio::time::interval(std::time::Duration::from_millis(100));
        flush_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        while !stream_done {
            tokio::select! {
                biased;

                // 定时强制刷新，避免小 chunk 滞留缓冲
                _ = flush_tick.tick() => {
                    line_state.flush();
                }

                // SSE stream chunk
                chunk = stream.next() => {
                    match chunk {
                        Some(Ok(bytes)) => {
                            buffer.push_str(&String::from_utf8_lossy(&bytes));

                            while let Some(pos) = buffer.find('\n') {
                                let line = buffer[..pos].trim().to_string();
                                buffer = buffer[pos + 1..].to_string();

                                if let Some(stream_chunk) = crate::api::parse_sse_line(&line) {
                                    if let Some(choice) = stream_chunk.choices.first() {
                                        if let Some(content) = &choice.delta.content {
                                            line_state.print_delta(content);
                                            full_content.push_str(content);
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
                                            stream_done = true;
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                        Some(Err(e)) => {
                            error!(error = %e, "stream chunk error");
                            stream_done = true;
                        }
                        None => {
                            stream_done = true;
                        }
                    }
                }
                // User typed something during streaming (via stdin thread)
                input = self.stdin_rx.recv() => {
                    match input {
                        Some(text) => {
                            if !text.is_empty() {
                                pending_input = Some(text);
                            }
                        }
                        None => {
                            // EOF — stdin closed
                            stream_done = true;
                        }
                    }
                }
            }
        }

        line_state.finish();

        Ok(StreamResult {
            content: full_content,
            reasoning_content,
            tool_calls_accum,
            has_tool_calls,
            pending_input,
        })
    }

    /// 获取 MCP 工具定义（转换为 API 格式）
    async fn get_tool_definitions(&self) -> Vec<ToolDefinition> {
        self.tool_executor.tool_definitions()
    }

    /// 执行工具调用（含审批检查）
    async fn execute_tool(&mut self, name: &str, args: serde_json::Value) -> String {
        debug!(tool_name = name, args = %args, "dispatching repl tool call");

        // 1. Validate against policy
        match self.tool_executor.validate_tool_call(name, &args).await {
            Ok(PolicyDecision::Allow) => {
                // Safe — execute directly
                self.do_execute_tool(name, args).await
            }
            Ok(PolicyDecision::Ask(req)) => {
                // Needs approval — prompt user
                self.prompt_approval(name, &args, req).await
            }
            Err(e) => {
                error!(tool_name = name, error = ?e, "policy validation error");
                serde_json::json!({
                    "success": false,
                    "error": {
                        "message": e.message,
                        "code": e.code
                    }
                })
                .to_string()
            }
        }
    }

    /// Execute tool directly (no policy check)
    async fn do_execute_tool(&self, name: &str, args: serde_json::Value) -> String {
        let message = self.tool_executor.execute_tool_call("tool_call", name, args).await;
        let content = message.content.unwrap_or_default();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap_or_default();
        if parsed["success"].as_bool().unwrap_or(false) {
            info!(tool_name = name, "tool call succeeded");
            println!("  {} Tool succeeded", "✓".green());
        } else {
            error!(tool_name = name, error = ?parsed, "tool call failed");
            println!("  {} Tool error", "✗".red());
        }
        content
    }

    /// Show approval prompt and wait for user
    async fn prompt_approval(&mut self, name: &str, args: &serde_json::Value, req: PermissionRequest) -> String {
        let detail = match name {
            "execute_command" | "exec_command" => {
                args.get("command")
                    .and_then(|v| v.as_str())
                    .map(|s| format!("`{}`", s))
                    .unwrap_or_default()
            }
            "file_write" | "file_edit" => {
                args.get("path")
                    .and_then(|v| v.as_str())
                    .map(|s| format!("{}", s))
                    .unwrap_or_default()
            }
            "apply_patch" => {
                args.get("workdir")
                    .and_then(|v| v.as_str())
                    .unwrap_or("current directory")
                    .to_string()
            }
            _ => String::new(),
        };

        println!();
        println!("  {} {} {}", "⚠".yellow().bold(), "Permission needed:".yellow().bold(), req.tool_name.cyan());
        println!("  {} {}", "  Reason:".truecolor(180, 180, 180), req.reason.truecolor(200, 200, 200));
        if !detail.is_empty() {
            println!("  {} {}", "  Detail:".truecolor(180, 180, 180), detail.truecolor(200, 200, 200));
        }
        println!("  {}", "  ─────────────────────────────".truecolor(100, 100, 100));
        println!("  [y] Allow once   [a] Always allow   [n] Deny");

        io::stdout().flush().ok();

        // Read user input
        let input = match self.stdin_rx.recv().await {
            Some(text) => text.trim().to_lowercase(),
            None => {
                return serde_json::json!({
                    "success": false,
                    "error": {
                        "message": "Approval cancelled (EOF)",
                        "code": "approval_cancelled"
                    }
                }).to_string();
            }
        };

        match input.as_str() {
            "y" | "yes" => {
                println!("  {} Approved", "✓".green());
                self.do_execute_tool(name, args.clone()).await
            }
            "a" | "always" => {
                println!("  {} Always allowed this session", "✓".green());
                self.tool_executor.approve_rule(req.session_rule).await;
                self.do_execute_tool(name, args.clone()).await
            }
            _ => {
                println!("  {} Denied", "✗".red());
                serde_json::json!({
                    "success": false,
                    "error": {
                        "message": format!("User denied permission for {}: {}", req.tool_name, req.reason),
                        "code": "permission_denied"
                    }
                }).to_string()
            }
        }
    }

    /// 交互式执行 ask_user_question 工具
    /// 直接读取 stdin 通道获取用户答案
    async fn execute_ask_user_question(&mut self, args: serde_json::Value) -> String {
        let question = args
            .get("question")
            .and_then(|v| v.as_str())
            .unwrap_or("请选择一个选项:");
        let options = args.get("options").and_then(|v| v.as_array());
        let multi_select = args
            .get("multiSelect")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // 解析选项列表
        let mut parsed_options: Vec<(String, String)> = Vec::new();
        if let Some(opts) = options {
            for opt in opts {
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
                parsed_options.push((label, desc));
            }
        }

        // 打印问题
        ui::print_question(question, &parsed_options);
        ui::print_question_prompt(multi_select);
        io::stdout().flush().ok();

        // 读取用户输入
        let input = match self.stdin_rx.recv().await {
            Some(text) => text,
            None => {
                return serde_json::json!({
                    "success": false,
                    "error": "Failed to read user input"
                })
                .to_string();
            }
        };

        // 解析答案
        let other_idx = parsed_options.len() + 1;
        let answers: Vec<serde_json::Value> = if multi_select {
            // 多选：逗号分隔数字
            let indices: Vec<usize> = input
                .split(',')
                .filter_map(|s| s.trim().parse::<usize>().ok())
                .filter(|&n| n > 0 && n <= other_idx)
                .collect();

            indices
                .into_iter()
                .map(|idx| {
                    if idx == other_idx {
                        serde_json::json!({
                            "label": "Other",
                            "value": input.trim(),
                            "custom": true
                        })
                    } else {
                        let (label, _) = &parsed_options[idx - 1];
                        serde_json::json!({
                            "label": label,
                            "value": label,
                            "custom": false
                        })
                    }
                })
                .collect()
        } else {
            // 单选
            if let Ok(idx) = input.trim().parse::<usize>() {
                if idx > 0 && idx <= parsed_options.len() {
                    let (label, _) = &parsed_options[idx - 1];
                    vec![serde_json::json!({
                        "label": label,
                        "value": label,
                        "custom": false
                    })]
                } else if idx == other_idx {
                    vec![serde_json::json!({
                        "label": "Other",
                        "value": input.trim(),
                        "custom": true
                    })]
                } else {
                    // 无效数字，视为自定义文本
                    vec![serde_json::json!({
                        "label": "Other",
                        "value": input.trim(),
                        "custom": true
                    })]
                }
            } else {
                // 非数字输入，视为自定义文本
                vec![serde_json::json!({
                    "label": "Other",
                    "value": input.trim(),
                    "custom": true
                })]
            }
        };

        serde_json::json!({
            "success": true,
            "answers": answers
        })
        .to_string()
    }

    fn print_status(&self) {
        let status = ui::StatusInfo {
            model: self.state.settings.model.clone(),
            api_base: self.state.settings.api.base_url.clone(),
            max_tokens: self.state.settings.api.max_tokens.to_string(),
            timeout: self.state.settings.api.timeout,
            streaming: self.state.settings.api.streaming,
            message_count: self.conversation_history.len(),
            api_key_set: self.state.settings.api.get_api_key().is_some(),
        };
        ui::print_status(&status);
    }

    fn print_history(&self) {
        println!();
        if self.conversation_history.is_empty() {
            println!(
                "  {} {}",
                "◦".truecolor(100, 100, 100),
                "No conversation history".bright_black()
            );
        } else {
            println!(
                "  {} {}",
                "◦".truecolor(147, 112, 219),
                format!(
                    "Conversation history ({} messages)",
                    self.conversation_history.len()
                )
                .truecolor(147, 112, 219)
                .bold()
            );
            println!();

            for (i, msg) in self.conversation_history.iter().enumerate() {
                let (_icon, _color) = match msg.role.as_str() {
                    "user" => ("●", "truecolor(255, 140, 66)"),
                    "assistant" => ("●", "truecolor(147, 112, 219)"),
                    _ => ("●", "bright_black"),
                };

                let role_label = match msg.role.as_str() {
                    "user" => "You".truecolor(255, 180, 100),
                    "assistant" => "wgenty".truecolor(200, 150, 255),
                    _ => "Unknown".bright_black(),
                };

                let content = msg.content.as_deref().unwrap_or("");
                let preview: String = content.chars().take(50).collect();
                let suffix = if content.len() > 50 { "..." } else { "" };

                println!(
                    "  {}. {}  {}{}",
                    (i + 1).to_string().truecolor(100, 100, 100),
                    role_label,
                    preview.bright_white(),
                    suffix.bright_black()
                );
            }
        }
        println!();
    }

    fn print_config(&self) {
        println!();
        println!(
            "  {} {}",
            "⚙".truecolor(147, 112, 219),
            "Configuration".truecolor(147, 112, 219).bold()
        );
        println!();

        match serde_json::to_string_pretty(&self.state.settings) {
            Ok(json) => {
                for line in json.lines() {
                    println!("  {}", line.bright_white());
                }
            }
            Err(_) => {
                ui::print_error("Failed to serialize configuration");
            }
        }
        println!();
    }

    fn reset_conversation(&mut self) {
        self.conversation_history.clear();
        ui::print_success("Conversation reset");
        println!();
    }
}

/// Result of a streaming operation with concurrent input
struct StreamResult {
    content: String,
    reasoning_content: String,
    tool_calls_accum: Vec<serde_json::Value>,
    has_tool_calls: bool,
    /// User input typed during streaming (if any)
    pending_input: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_repl_creation() {
        let state = AppState::default();
        let repl = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(Repl::new(state));
        assert!(repl.conversation_history.is_empty());
    }
}
