use super::{AgentLoop, MAX_ESTIMATED_TOKENS};
use crate::agent::StreamProcessor;
use crate::api::ChatMessage;
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

    pub(super) async fn do_auto_compact(&mut self) {
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
}
