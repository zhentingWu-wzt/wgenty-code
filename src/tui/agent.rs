//! AgentLoop — the core agent loop: SSE streaming + tool execution + context compaction.
//! Port of TypeScript agent-loop.ts to Rust.
//!
//! Each Job (one user input → final response) creates its own AgentLoop instance
//! backed by a *shared* conversation_history (Arc<Mutex<Vec<ChatMessage>>>).
//! This allows multiple user inputs to be queued while one is processing:
//! each pending input becomes a new AgentLoop that inherits the accumulated history.

use crate::agent::{StreamEvent, StreamProcessor, StreamResult};
use crate::api::ChatMessage;
use crate::guardian::classify_risk;
use crate::tui::app::AppEvent;
use crate::tui::client::DaemonClient;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;

const MAX_RETRIES: u32 = 2;
const MAX_ESTIMATED_TOKENS: usize = 50_000;
const MAX_ROUNDS: usize = 10;

pub struct AgentLoop {
    client: DaemonClient,
    event_tx: mpsc::UnboundedSender<AppEvent>,
    /// Shared conversation history across all Jobs in this session.
    /// Each AgentLoop instance reads/writes through this Arc, so the
    /// accumulated context is inherited by the next Job in the queue.
    conversation_history: Arc<tokio::sync::Mutex<Vec<ChatMessage>>>,
    /// Pre-assembled system messages (layered instructions from PromptAssembler).
    /// Used when initializing or resetting the conversation history.
    assembled_system_messages: Vec<ChatMessage>,
    rounds_since_todo: usize,
    compacted_summary: String,
    session_id: String,
}

impl AgentLoop {
    pub fn new(
        client: DaemonClient,
        event_tx: mpsc::UnboundedSender<AppEvent>,
        session_id: String,
        conversation_history: Arc<tokio::sync::Mutex<Vec<ChatMessage>>>,
        system_messages: Vec<ChatMessage>,
    ) -> Self {
        Self {
            client,
            event_tx,
            conversation_history,
            assembled_system_messages: system_messages,
            rounds_since_todo: 0,
            compacted_summary: String::new(),
            session_id,
        }
    }

    /// Process a single user input. Handles the full agent loop (SSE + tools).
    pub async fn process_input(&mut self, input: String) {
        self.inject_background_results().await;

        {
            let mut history = self.conversation_history.lock().await;
            history.push(ChatMessage::user(&input));
        }

        for _round in 0..MAX_ROUNDS {
            let messages = self.micro_compact().await;

            if self.needs_compaction(&messages) {
                self.do_auto_compact().await;
                continue;
            }

            let result = match self.stream_with_retry(&messages).await {
                Ok(r) => r,
                Err(e) => {
                    let _ = self.event_tx.send(AppEvent::StreamError(e.to_string()));
                    return;
                }
            };

            if result.has_tool_calls && !result.tool_calls.is_empty() {
                // Build and push assistant message with tool calls
                let assistant_msg = StreamProcessor::build_assistant_message(
                    result.content,
                    result.reasoning_content,
                    result.tool_calls.clone(),
                );
                {
                    let mut history = self.conversation_history.lock().await;
                    history.push(assistant_msg);
                }

                let mut used_todo = false;
                for tc in &result.tool_calls {
                    let args: serde_json::Value =
                        serde_json::from_str(&tc.function.arguments).unwrap_or_default();

                    // Handle ask_user_question locally
                    if tc.function.name == "ask_user_question" {
                        let tool_result = self.handle_ask_user_question(&args).await;
                        {
                            let mut history = self.conversation_history.lock().await;
                            history.push(ChatMessage::tool(&tc.id, tool_result));
                        }
                        continue;
                    }

                    // Handle compact locally
                    if tc.function.name == "compact" {
                        let _ = self.event_tx.send(AppEvent::ToolStart {
                            name: "compact".to_string(),
                        });
                        self.do_auto_compact().await;
                        let _ = self.event_tx.send(AppEvent::ToolResult {
                            name: "compact".to_string(),
                            content: "Conversation history compressed.".to_string(),
                        });
                        let mut history = self.conversation_history.lock().await;
                        history.push(ChatMessage::tool(
                            &tc.id,
                            r#"{"success":true,"content":"Conversation compressed"}"#,
                        ));
                        continue;
                    }

                    // Track TodoWrite usage for nag reminder
                    if tc.function.name == "TodoWrite" {
                        used_todo = true;
                    }

                    let _ = self.event_tx.send(AppEvent::ToolStart {
                        name: tc.function.name.clone(),
                    });

                    let exec_result = self
                        .execute_tool_with_permission(&tc.function.name, args.clone())
                        .await;

                    let _ = self.event_tx.send(AppEvent::ToolResult {
                        name: tc.function.name.clone(),
                        content: exec_result.clone(),
                    });

                    {
                        let mut history = self.conversation_history.lock().await;
                        history.push(ChatMessage::tool(&tc.id, exec_result));
                    }
                }

                // s03: nag reminder — inject after 3 rounds without TodoWrite
                self.rounds_since_todo = if used_todo {
                    0
                } else {
                    self.rounds_since_todo + 1
                };
                if self.rounds_since_todo >= 3 {
                    let mut history = self.conversation_history.lock().await;
                    if let Some(last) = history.last_mut() {
                        if last.role == "tool" {
                            if let Some(ref mut content) = last.content {
                                content.push_str(
                                    "\n<reminder>Update your todos with TodoWrite.</reminder>",
                                );
                            }
                        }
                    }
                }

                let _ = self.event_tx.send(AppEvent::SaveSession);
                continue; // Continue the tool call loop
            }

            // Normal assistant response — no tool calls
            if !result.content.is_empty() {
                let reasoning = if result.reasoning_content.is_empty() {
                    None
                } else {
                    Some(result.reasoning_content)
                };
                {
                    let mut history = self.conversation_history.lock().await;
                    history.push(ChatMessage {
                        role: "assistant".to_string(),
                        content: Some(result.content),
                        reasoning_content: reasoning,
                        tool_calls: None,
                        tool_call_id: None,
                    });
                }
            }

            let _ = self.event_tx.send(AppEvent::StreamDone {
                finish_reason: result.finish_reason,
            });
            let _ = self.event_tx.send(AppEvent::SaveSession);
            return;
        }
    }

    /// Stream with retry logic. Retries up to MAX_RETRIES on network/stream errors.
    async fn stream_with_retry(
        &mut self,
        messages: &[ChatMessage],
    ) -> anyhow::Result<StreamResult> {
        let mut last_error = String::new();

        for attempt in 0..=MAX_RETRIES {
            match self.client.chat_stream(messages.to_vec(), None).await {
                Ok(response) => match self.stream_response(response).await {
                    Ok(result) => {
                        // Detect incomplete stream: has tool calls without finish_reason
                        if result.has_tool_calls
                            && result.finish_reason.is_empty()
                            && attempt < MAX_RETRIES
                        {
                            let _ = self.event_tx.send(AppEvent::StreamError(
                                "Stream ended before tool calls completed, retrying...".to_string(),
                            ));
                            tokio::time::sleep(tokio::time::Duration::from_secs(
                                (attempt + 1) as u64 * 2,
                            ))
                            .await;
                            continue;
                        }
                        return Ok(result);
                    }
                    Err(e) => {
                        last_error = e.to_string();
                        if attempt < MAX_RETRIES {
                            let _ = self.event_tx.send(AppEvent::StreamError(format!(
                                "Stream error, retrying... ({})",
                                e
                            )));
                            tokio::time::sleep(tokio::time::Duration::from_secs(
                                (attempt + 1) as u64 * 2,
                            ))
                            .await;
                            continue;
                        }
                    }
                },
                Err(e) => {
                    last_error = e.to_string();
                    if attempt < MAX_RETRIES {
                        tokio::time::sleep(tokio::time::Duration::from_secs(
                            (attempt + 1) as u64 * 2,
                        ))
                        .await;
                        continue;
                    }
                }
            }
            break;
        }

        Err(anyhow::anyhow!(
            "Stream failed after retries: {}",
            last_error
        ))
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

        // Flush remaining buffered data
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
                let _ = self.event_tx.send(AppEvent::StreamDone { finish_reason });
            }
        }
    }

    async fn execute_tool_with_permission(
        &mut self,
        name: &str,
        args: serde_json::Value,
    ) -> String {
        // Guardian: block critical-risk commands before they reach the daemon
        if name == "execute_command" || name == "exec_command" {
            if let Some(cmd) = args.get("command").and_then(|v| v.as_str()) {
                let risk = classify_risk(cmd);
                if risk >= crate::guardian::RiskLevel::Critical {
                    let msg = format!(
                        "GUARDIAN BLOCK: critical-risk command rejected. {}",
                        cmd
                    );
                    tracing::warn!("{}", msg);
                    return format!(r#"{{"success":false,"error":"{}"}}"#, msg);
                }
            }
        }

        let result = match self
            .client
            .execute_tool(name, args.clone(), &self.session_id)
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                tracing::warn!("Tool execution failed for '{}': {}", name, e);
                return format!(r#"{{"success":false,"error":"{}"}}"#, e);
            }
        };

        // If permission required, ask the user via inline panel
        if let Some(perm) = result.permission_required {
            tracing::info!(
                "🔐 Permission required for '{}': {} (rule: {})",
                name,
                perm.reason,
                perm.session_rule
            );

            let (tx, rx) = tokio::sync::oneshot::channel();

            let _ = self.event_tx.send(AppEvent::PermissionRequired {
                reason: perm.reason.clone(),
                rule: perm.session_rule.clone(),
                responder: crate::tui::app::PermissionResponder(Some(tx)),
            });

            match rx.await {
                Ok(crate::tui::app::PermissionResponse::AllowOnce) => {
                    // Approve → execute → unapprove (one-shot)
                    if self.client.approve_tool(&perm.session_rule).await.is_err() {
                        return r#"{"success":false,"error":"Failed to approve permission"}"#.to_string();
                    }

                    let result = self
                        .client
                        .execute_tool(name, args.clone(), &self.session_id)
                        .await;

                    // Remove the temporary approval
                    let _ = self.client.unapprove_tool(&perm.session_rule).await;

                    match result {
                        Ok(resp) => {
                            return format!(
                                r#"{{"success":{},"output_type":{},"content":{},"error":{}}}"#,
                                resp.success,
                                serde_json::to_string(&resp.output_type).unwrap_or_default(),
                                serde_json::to_string(&resp.content).unwrap_or_default(),
                                serde_json::to_string(&resp.error).unwrap_or_default(),
                            );
                        }
                        Err(e) => {
                            return format!(r#"{{"success":false,"error":"{}"}}"#, e);
                        }
                    }
                }
                Ok(crate::tui::app::PermissionResponse::AlwaysAllow) => {
                    // Approve the rule, then re-execute the tool
                    if self.client.approve_tool(&perm.session_rule).await.is_err() {
                        return r#"{{"success":false,"error":"Failed to approve permission"}}"#.to_string();
                    }

                    match self
                        .client
                        .execute_tool(name, args.clone(), &self.session_id)
                        .await
                    {
                        Ok(resp) => {
                            return format!(
                                r#"{{"success":{},"output_type":{},"content":{},"error":{}}}"#,
                                resp.success,
                                serde_json::to_string(&resp.output_type).unwrap_or_default(),
                                serde_json::to_string(&resp.content).unwrap_or_default(),
                                serde_json::to_string(&resp.error).unwrap_or_default(),
                            );
                        }
                        Err(e) => {
                            return format!(r#"{{"success":false,"error":"{}"}}"#, e);
                        }
                    }
                }
                Ok(crate::tui::app::PermissionResponse::Deny) | Err(_) => {
                    return format!(
                        r#"{{"success":false,"error":"PERMISSION DENIED: {}"}}"#,
                        perm.reason
                    );
                }
            }
        }

        // No permission required — return result directly
        format!(
            r#"{{"success":{},"output_type":{},"content":{},"error":{}}}"#,
            result.success,
            serde_json::to_string(&result.output_type).unwrap_or_default(),
            serde_json::to_string(&result.content).unwrap_or_default(),
            serde_json::to_string(&result.error).unwrap_or_default(),
        )
    }

    async fn handle_ask_user_question(&self, args: &serde_json::Value) -> String {
        let (tx, rx) = tokio::sync::oneshot::channel();

        let question = args["question"]
            .as_str()
            .unwrap_or("Choose an option:")
            .to_string();
        let options: Vec<String> = args["options"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|o| o["label"].as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let multi_select = args["multi_select"].as_bool().unwrap_or(false);

        let _ = self.event_tx.send(AppEvent::QuestionAsked {
            question,
            options,
            multi_select,
            responder: crate::tui::app::QuestionResponder(Some(tx)),
        });

        match rx.await {
            Ok(answers) => {
                let answers_json: Vec<serde_json::Value> = answers
                    .iter()
                    .map(|a| serde_json::json!({"label": a, "value": a, "custom": false}))
                    .collect();
                serde_json::json!({
                    "success": true,
                    "answers": answers_json
                })
                .to_string()
            }
            Err(_) => {
                // Channel closed without response (user pressed Esc)
                serde_json::json!({
                    "success": false,
                    "error": "User cancelled the question"
                })
                .to_string()
            }
        }
    }

    async fn inject_background_results(&mut self) {
        match self.client.get_background_results().await {
            Ok(results) if !results.is_empty() => {
                let notification: String = results
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
                {
                    let mut history = self.conversation_history.lock().await;
                    history.push(ChatMessage::user(notification));
                }
            }
            _ => {}
        }
    }

    // ── Compaction (s06) ───────────────────────────────────────────────────

    /// Micro-compaction: replace old tool results with short markers.
    /// Keeps the last 3 tool messages as-is; always preserves read_file results.
    async fn micro_compact(&self) -> Vec<ChatMessage> {
        let history = self.conversation_history.lock().await;
        let mut id_to_name = std::collections::HashMap::new();
        for msg in history.iter() {
            if msg.role == "assistant" {
                if let Some(ref tcs) = msg.tool_calls {
                    for tc in tcs {
                        id_to_name.insert(tc.id.clone(), tc.function.name.clone());
                    }
                }
            }
        }

        let tool_indices: Vec<usize> = history
            .iter()
            .enumerate()
            .filter(|(_, m)| m.role == "tool")
            .map(|(i, _)| i)
            .collect();

        let keep_count = 3usize;
        let keep_start = tool_indices.len().saturating_sub(keep_count);
        let keep_indices: std::collections::HashSet<usize> =
            tool_indices[keep_start..].iter().copied().collect();

        history
            .iter()
            .enumerate()
            .map(|(i, msg)| {
                if msg.role == "tool" && !keep_indices.contains(&i) {
                    let tool_name = msg
                        .tool_call_id
                        .as_deref()
                        .and_then(|id| id_to_name.get(id));
                    // Always preserve read_file results (reference material)
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
            .join(".claude-code")
            .join("transcripts");

        tokio::fs::create_dir_all(&transcript_dir).await.ok();

        let timestamp = chrono::Utc::now().format("%Y-%m-%dT%H-%M-%S").to_string();
        let transcript_path = transcript_dir.join(format!("session_{}.json", timestamp));

        let history_snapshot = {
            let history = self.conversation_history.lock().await;
            history.clone()
        };
        let json = serde_json::to_string_pretty(&history_snapshot).unwrap_or_default();
        tokio::fs::write(&transcript_path, json).await.ok();

        // Build plain-text transcript for summarization
        let transcript_text: String = history_snapshot
            .iter()
            .map(|m| format!("[{}]: {}", m.role, m.content.as_deref().unwrap_or("")))
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
            while let Some(chunk) = stream.next().await {
                if let Ok(bytes) = chunk {
                    processor.feed_bytes(&bytes);
                }
            }
            let result = processor.finish();
            let summary = result.content;

            if !summary.is_empty() {
                self.compacted_summary = summary.clone();
            
                {
                    let mut history = self.conversation_history.lock().await;
                    let mut new_history = self.assembled_system_messages.clone();
                    new_history.push(ChatMessage::system(format!(
                        "<previous_conversation_summary>\n{}\n</previous_conversation_summary>",
                        summary
                    )));
                    *history = new_history;
                }
            }
        }
    }

    // ── Session state ────────────────────────────────────────────────────

    pub async fn load_history(&self, messages: Vec<ChatMessage>) {
        let mut history = self.conversation_history.lock().await;
        *history = messages;
    }

    pub async fn get_history(&self) -> Vec<ChatMessage> {
        self.conversation_history.lock().await.clone()
    }

    pub async fn reset(&self) {
        let mut history = self.conversation_history.lock().await;
        *history = self.assembled_system_messages.clone();
    }
}

// System prompt is now assembled via crate::prompts::assemble_instructions()

