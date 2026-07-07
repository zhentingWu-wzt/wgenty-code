use super::{AgentLoop, MAX_ESTIMATED_TOKENS};
use crate::agent::{StreamEvent, StreamProcessor};
use crate::api::ChatMessage;
use crate::tui::app::AppEvent;
use std::path::PathBuf;

impl AgentLoop {
    pub(super) async fn inject_background_results(&mut self) {
        match self.client.get_background_results().await {
            Ok(results) if !results.is_empty() => {
                let notification: String = results
                    .iter()
                    .map(|r| {
                        let task_id = r["task_id"].as_str().unwrap_or("unknown");
                        let result_type = r["result_type"].as_str().unwrap_or("command");
                        if result_type == "subagent" {
                            let result = r["stdout"].as_str().unwrap_or("");
                            format!("[Subagent {} completed]\n{}", task_id, result)
                        } else {
                            let success = r["success"].as_bool().unwrap_or(false);
                            format!(
                                "[Background task {} completed: {}]",
                                task_id,
                                if success { "SUCCESS" } else { "FAILED" }
                            )
                        }
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
    pub(super) async fn micro_compact(&self) -> Vec<ChatMessage> {
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

    pub(super) fn needs_compaction(&self, messages: &[ChatMessage]) -> bool {
        let total_chars: usize = messages
            .iter()
            .map(|m| m.content.as_deref().unwrap_or("").len())
            .sum();
        total_chars / 4 > MAX_ESTIMATED_TOKENS
    }

    /// Run conversation compaction: archive the transcript, ask the model for a
    /// summary, and replace `conversation_history` with
    /// `[system_messages, system(summary), user(continue)]`.
    ///
    /// Returns `true` on success. Returns `false` (and leaves history intact)
    /// on any failure — stream error, empty summary, etc. The caller must not
    /// retry unbounded on `false`: a failed compaction means the next request
    /// proceeds with the micro-compacted history, surfacing a real upstream
    /// error if the context is still too large, instead of spinning here.
    pub(super) async fn do_auto_compact(&mut self) -> bool {
        // Save transcript to disk
        let transcript_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".wgenty-code")
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

        // Build plain-text transcript for summarization. Include tool_calls
        // and (truncated) reasoning_content — a transcript that only carries
        // `content` loses what tools the assistant invoked and what it was
        // planning, so the summary can't faithfully represent the work done.
        let transcript_text: String = history_snapshot
            .iter()
            .map(|m| {
                let mut parts: Vec<String> = vec![format!("[{}]", m.role)];
                if let Some(rc) = m.reasoning_content.as_ref().filter(|s| !s.is_empty()) {
                    // Reasoning can be very long; cap per-message (by chars,
                    // char-boundary safe) so the summary request itself
                    // doesn't blow the context window.
                    const RC_CAP: usize = 1000;
                    let mut chars = rc.chars();
                    let snippet: String = chars.by_ref().take(RC_CAP).collect();
                    let truncated = chars.next().is_some();
                    parts.push(format!(
                        "reasoning: {}{}",
                        snippet,
                        if truncated { "…(truncated)" } else { "" }
                    ));
                }
                if let Some(c) = m.content.as_ref().filter(|s| !s.is_empty()) {
                    parts.push(c.clone());
                }
                if let Some(tcs) = m.tool_calls.as_ref() {
                    for tc in tcs {
                        parts.push(format!(
                            "tool_call: {}({})",
                            tc.function.name, tc.function.arguments
                        ));
                    }
                }
                parts.join("\n")
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

        // plan_mode = Some(true) makes the daemon omit tool definitions from
        // the summarization request. Without this the model is offered the
        // full tool set and may answer with a tool_call, leaving `content`
        // empty — a silent compaction failure (the system prompt asks for no
        // tools, but they were still being offered).
        let response = match self
            .client
            .chat_stream_with_plan(summary_messages, None, Some(true))
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "compaction summary request failed");
                let _ = self.event_tx.send(AppEvent::StreamError(
                    "Compaction failed; continuing with full history.".to_string(),
                ));
                return false;
            }
        };

        let mut processor = StreamProcessor::new();
        let mut stream = response.bytes_stream();
        use futures::StreamExt;
        let mut stream_error: Option<String> = None;
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    for ev in processor.feed_bytes(&bytes) {
                        if let StreamEvent::StreamError(msg) = ev {
                            stream_error = Some(msg);
                        }
                    }
                }
                Err(e) => {
                    stream_error = Some(format!("summary stream read error: {e}"));
                    break;
                }
            }
        }
        // Drain any trailing partial line so its content is accumulated.
        for ev in processor.flush() {
            if let StreamEvent::StreamError(msg) = ev {
                stream_error = Some(msg);
            }
        }
        if let Some(reason) = stream_error {
            tracing::warn!(reason = %reason, "compaction summary stream errored");
            let _ = self.event_tx.send(AppEvent::StreamError(
                "Compaction failed; continuing with full history.".to_string(),
            ));
            return false;
        }

        let result = processor.finish();
        // Reasoning models may emit the summary as `reasoning_content` only,
        // leaving `content` empty — fall back so a valid summary isn't
        // discarded (which would otherwise look like an empty summary and
        // cause the loop to spin).
        let summary = if !result.content.is_empty() {
            result.content
        } else {
            result.reasoning_content
        };

        if summary.trim().is_empty() {
            tracing::warn!("compaction produced an empty summary; leaving history intact");
            let _ = self.event_tx.send(AppEvent::StreamError(
                "Compaction produced an empty summary; continuing with full history.".to_string(),
            ));
            return false;
        }

        self.compacted_summary = summary.clone();
        {
            let mut history = self.conversation_history.lock().await;
            let mut new_history = self.assembled_system_messages.clone();
            new_history.push(ChatMessage::system(format!(
                "<previous_conversation_summary>\n{}\n</previous_conversation_summary>",
                summary
            )));
            // Append a user turn so the next request is valid.
            // OpenAI-compatible endpoints (Ark included) reject an
            // all-system-messages request, and the user turn also
            // prompts the model to continue the task from the summary
            // instead of stalling.
            new_history.push(ChatMessage::user(
                "Conversation history was just compacted. Continue the current task using the summary above."
            ));
            *history = new_history;
        }
        true
    }
}
