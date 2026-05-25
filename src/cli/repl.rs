//! REPL Module - Interactive Read-Eval-Print Loop
//!
//! Beautiful REPL interface matching the original Claude Code aesthetic
//!
//! Uses a dedicated stdin thread + mpsc channel for concurrent input during
//! streaming. This keeps the terminal in cooked mode so CJK/IME works normally.

use crate::api::{ApiClient, ChatMessage, ToolCall, ToolDefinition};
use crate::cli::ui;
use crate::mcp::ToolRegistry;
use crate::state::AppState;
use colored::Colorize;
use futures::StreamExt;
use std::io::{self, BufRead, Write};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

pub struct Repl {
    state: AppState,
    conversation_history: Vec<ChatMessage>,
    tool_registry: Arc<ToolRegistry>,
    stdin_rx: mpsc::Receiver<String>,
}

impl Repl {
    pub async fn new(state: AppState) -> Self {
        ui::init_terminal();
        let tool_registry = Arc::new(ToolRegistry::new());
        tool_registry.register_builtin_tools().await;

        let (stdin_tx, stdin_rx) = mpsc::channel::<String>(16);

        // Dedicated stdin thread: reads lines in cooked mode (IME works)
        // and sends them through the channel.
        std::thread::spawn(move || {
            let stdin = io::stdin();
            loop {
                let mut line = String::new();
                match stdin.lock().read_line(&mut line) {
                    Ok(0) => break, // EOF
                    Ok(_) => {
                        let trimmed = line.trim().to_string();
                        if stdin_tx.blocking_send(trimmed).is_err() {
                            break; // channel closed
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        info!("repl initialized");

        Self {
            state,
            conversation_history: Vec::new(),
            tool_registry,
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
            ui::print_input_border_top();
            ui::print_prompt();
            io::stdout().flush().ok();

            let input = match self.stdin_rx.recv().await {
                Some(input) => input,
                None => break, // channel closed = EOF
            };

            if input.is_empty() {
                ui::complete_input_line("");
                ui::print_input_border_bottom();
                continue;
            }

            ui::complete_input_line(&input);
            ui::print_input_border_bottom();

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
            ui::print_typing_indicator();

            let messages = self.conversation_history.clone();
            let response = match client.chat_stream(messages, tools_opt.clone()).await {
                Ok(r) => r,
                Err(e) => {
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

            ui::print_assistant_border_bottom();
            println!();

            if result.has_tool_calls && !result.tool_calls_accum.is_empty() {
                info!(tool_call_count = result.tool_calls_accum.len(), "model requested tool calls");
                for tc in &result.tool_calls_accum {
                    let tool_name = tc["function"]["name"].as_str().unwrap_or("unknown");
                    println!(
                        "  {} Executing tool: {}",
                        "🔧".truecolor(255, 200, 100),
                        tool_name.cyan().bold()
                    );
                }
                println!();

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

                let assistant_msg = ChatMessage {
                    role: "assistant".to_string(),
                    content: if result.content.is_empty() {
                        None
                    } else {
                        Some(result.content)
                    },
                    tool_calls: Some(tool_calls_parsed),
                    tool_call_id: None,
                };
                self.conversation_history.push(assistant_msg);

                for tc in &self.conversation_history.last().unwrap().tool_calls.clone().unwrap() {
                    let args: serde_json::Value =
                        serde_json::from_str(&tc.function.arguments)
                            .unwrap_or(serde_json::json!({}));
                    let tool_result = self.execute_tool(&tc.function.name, args).await;
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
                self.conversation_history
                    .push(ChatMessage::assistant(result.content));
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
            ui::print_input_border_top();
            ui::complete_input_line(&pending);
            ui::print_input_border_bottom();
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
        let mut full_content = String::new();
        let mut tool_calls_accum: Vec<serde_json::Value> = Vec::new();
        let mut has_tool_calls = false;
        let mut pending_input: Option<String> = None;
        let mut stream_done = false;

        ui::print_assistant_border_top();

        let mut stream = response.bytes_stream();
        let mut buffer = String::new();

        while !stream_done {
            tokio::select! {
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

        Ok(StreamResult {
            content: full_content,
            tool_calls_accum,
            has_tool_calls,
            pending_input,
        })
    }

    /// 获取 MCP 工具定义（转换为 API 格式）
    async fn get_tool_definitions(&self) -> Vec<ToolDefinition> {
        let tools = self.tool_registry.list().await;
        tools
            .into_iter()
            .map(|t| ToolDefinition::new(t.name, t.description, t.input_schema))
            .collect()
    }

    /// 执行工具调用
    async fn execute_tool(&self, name: &str, args: serde_json::Value) -> String {
        debug!(tool_name = name, args = %args, "dispatching repl tool call");
        match self.tool_registry.execute(name, args).await {
            Ok(result) => {
                // 打印工具结果摘要
                if let Some(success) = result.get("success").and_then(|s| s.as_bool()) {
                    if success {
                        info!(tool_name = name, "tool call succeeded");
                        println!("  {} Tool succeeded", "✓".green());
                    } else {
                        warn!(tool_name = name, "tool call reported failure");
                        println!("  {} Tool failed", "✗".red());
                    }
                }
                result.to_string()
            }
            Err(e) => {
                error!(tool_name = name, error = %e, "tool call failed");
                println!("  {} Tool error: {}", "✗".red(), e);
                serde_json::json!({"error": e.to_string()}).to_string()
            }
        }
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
