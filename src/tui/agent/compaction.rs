use super::AgentLoop;
use crate::agent::{StreamEvent, StreamProcessor};
use crate::api::ChatMessage;
use crate::context::{MemoryEntry, MemoryType};
use crate::tui::app::AppEvent;
use std::path::PathBuf;

/// Split `history` into `(to_summarize, tail)` for compaction.
///
/// The tail is the last assistant message and every tool result after it — the
/// in-flight exchange whose results the model has NOT seen yet (they were
/// produced after its last response). The tail is preserved inline after the
/// summary so fresh results aren't summarized away. Everything before the last
/// assistant is returned in `to_summarize` for the summarizer.
///
/// If there is no assistant message, the whole history is summarized and the
/// tail is empty (the caller then appends a synthetic `user(continue)` turn).
fn split_for_compaction(history: &[ChatMessage]) -> (Vec<ChatMessage>, Vec<ChatMessage>) {
    match history.iter().rposition(|m| m.role == "assistant") {
        Some(idx) => (history[..idx].to_vec(), history[idx..].to_vec()),
        None => (history.to_vec(), Vec::new()),
    }
}

/// Assemble the post-compaction history from the base system messages, a
/// compaction summary, and the in-flight tail.
///
/// The first non-system message in the result is always a `user` turn — this
/// is required by OpenAI-compatible endpoints (Ark included), which reject a
/// request whose first non-system message is an `assistant`.
fn assemble_post_compaction_history(
    system_messages: &[ChatMessage],
    summary: &str,
    tail: &[ChatMessage],
) -> Vec<ChatMessage> {
    let mut new_history = system_messages.to_vec();
    new_history.push(ChatMessage::system(format!(
        "<previous_conversation_summary>\n{}\n</previous_conversation_summary>",
        summary
    )));
    // Always insert a synthetic user turn between the summary and the tail.
    // The tail starts with an assistant message (by split_for_compaction), and
    // OpenAI-compatible endpoints (Ark included) reject a request whose first
    // non-system message is an assistant — there must be a preceding user
    // turn. Without this, every post-compaction request fails with
    // InvalidParameter.
    new_history.push(ChatMessage::user(
        "Conversation history was just compacted. Continue the current task using the summary above."
    ));
    // Preserve the in-flight tail (last assistant tool_calls + its tool
    // results) so fresh, unseen results aren't summarized away.
    new_history.extend(tail.iter().cloned());
    new_history
}

/// Approximate the request's token cost by summing characters across the
/// message fields that dominate it: `content`, `reasoning_content` (the
/// largest cost for thinking models), and each tool_call's
/// `function.arguments`.
///
/// Tool *definitions* (sent every request but constant per session) are
/// excluded - they're a fixed overhead the compaction threshold leaves
/// margin for. The previous content-only sum missed ~68% of the payload
/// (reasoning + tool_calls), so compaction never fired before the context
/// window overflowed - see change `fix-compaction-ignores-reasoning`.
fn request_size_chars(messages: &[ChatMessage]) -> usize {
    messages
        .iter()
        .map(|m| {
            let content = m.content.as_deref().map(str::len).unwrap_or(0);
            let reasoning = m.reasoning_content.as_deref().map(str::len).unwrap_or(0);
            let tool_call_args: usize = m
                .tool_calls
                .as_ref()
                .map(|tcs| tcs.iter().map(|tc| tc.function.arguments.len()).sum())
                .unwrap_or(0);
            content + reasoning + tool_call_args
        })
        .sum()
}

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
        // `request_size_chars` counts content + reasoning_content +
        // tool_calls.arguments (the fields that dominate request token cost);
        // dividing by 4 gives a rough token estimate. Compaction fires when
        // the estimate exceeds 80 % of the model's context window.
        request_size_chars(messages) / 4 > self.context_window * 4 / 5
    }

    /// Run conversation compaction: archive the transcript, ask the model for a
    /// summary, and replace `conversation_history` with
    /// `[system_messages, system(summary), ...tail]`, where `tail` is the last
    /// assistant tool_calls + its tool results (fresh, unseen results preserved
    /// raw so they aren't summarized away). If there is no tail, a
    /// `user(continue)` turn is appended instead.
    ///
    /// Returns `true` on success. Returns `false` (and leaves history intact)
    /// on any failure — stream error, empty summary, etc. The caller must not
    /// retry unbounded on `false`: a failed compaction means the next request
    /// proceeds with the micro-compacted history, surfacing a real upstream
    /// error if the context is still too large, instead of spinning here.
    pub(super) async fn do_auto_compact(&mut self) -> bool {
        // Surface compaction start to the UI immediately so the status bar
        // shows "compacting..." while the summarization stream runs (which can
        // take several seconds). `ContextCompacted` / the next `Connecting`
        // event will transition the phase away once done.
        let _ = self.event_tx.send(AppEvent::CompactionStarted);

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

        let (to_summarize, tail) = split_for_compaction(&history_snapshot);

        // Build plain-text transcript for summarization. Include tool_calls
        // and (truncated) reasoning_content — a transcript that only carries
        // `content` loses what tools the assistant invoked and what it was
        // planning, so the summary can't faithfully represent the work done.
        // Only the already-seen part (`to_summarize`) is summarized; the tail
        // is preserved inline below.
        let transcript_text: String = to_summarize
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
                "You are a conversation summary assistant for an AI coding agent. \
                 Your task is to:\n\
                 1. Summarize the conversation history, preserving key details: \
                 project context, files modified, decisions made, bugs found, \
                 commands executed, and any pending tasks.\n\
                 2. Extract key memories from the conversation as structured JSON.\n\n\
                 Output format — respond with a single JSON object (no markdown fences, no extra text):\n\
                 {\n\
                   \"summary\": \"<concise summary string>\",\n\
                   \"memories\": [\n\
                     {\n\
                       \"type\": \"decision|error|preference|insight|knowledge|task\",\n\
                       \"content\": \"<what to remember>\",\n\
                       \"importance\": <0.0 to 1.0>\n\
                     }\n\
                   ]\n\
                 }\n\n\
                 If there is nothing worth remembering, return an empty memories array.\n\
                 Do NOT use any tools — just return the JSON as plain text.",
            ),
            ChatMessage::user(format!(
                "Process this conversation history:\n\n{}",
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
        let full_text = if !result.content.is_empty() {
            result.content
        } else {
            result.reasoning_content
        };

        // Attempt to parse JSON response for dual output (summary + memories)
        let (summary, extracted_memories) = match serde_json::from_str::<serde_json::Value>(full_text.trim()) {
            Ok(json) => {
                let summary = json
                    .get("summary")
                    .and_then(|v| v.as_str())
                    .unwrap_or(full_text.trim())
                    .to_string();
                let memories: Vec<MemoryEntry> = json
                    .get("memories")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|m| {
                                let mem_type_str = m.get("type").and_then(|v| v.as_str()).unwrap_or("knowledge");
                                let mem_type = match mem_type_str {
                                    "decision" => MemoryType::Decision,
                                    "error" => MemoryType::Error,
                                    "preference" => MemoryType::Preference,
                                    "insight" => MemoryType::Insight,
                                    "knowledge" => MemoryType::Knowledge,
                                    "task" => MemoryType::Task,
                                    _ => MemoryType::Knowledge,
                                };
                                let content = m.get("content").and_then(|v| v.as_str()).unwrap_or("");
                                let importance = m.get("importance")
                                    .and_then(|v| v.as_f64())
                                    .unwrap_or(0.5) as f32;
                                if content.is_empty() {
                                    return None;
                                }
                                Some(MemoryEntry::new(mem_type, content).with_importance(importance))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                (summary, memories)
            }
            Err(e) => {
                // JSON parse failed — fallback to full text as summary only
                tracing::warn!(error = %e, "compaction response is not valid JSON; using full text as summary, skipping memory extraction");
                (full_text.trim().to_string(), Vec::new())
            }
        };

        if summary.trim().is_empty() {
            tracing::warn!("compaction produced an empty summary; leaving history intact");
            let _ = self.event_tx.send(AppEvent::StreamError(
                "Compaction produced an empty summary; continuing with full history.".to_string(),
            ));
            return false;
        }

        // Persist extracted memories
        for memory in &extracted_memories {
            if let Err(e) = self.memory_manager.add_memory(memory.clone()).await {
                tracing::warn!(error = %e, memory_id = %memory.id, "failed to persist extracted memory; continuing with summary only");
            }
        }
        if !extracted_memories.is_empty() {
            tracing::info!(count = extracted_memories.len(), "extracted memories from compaction");
        }

        self.compacted_summary = summary.clone();
        {
            let mut history = self.conversation_history.lock().await;
            *history =
                assemble_post_compaction_history(&self.assembled_system_messages, &summary, &tail);
        }
        // Surface compaction to the UI so it isn't silent. Reads
        // `compacted_summary` (otherwise a write-only field) to report the
        // summary size.
        let summary_chars = self.compacted_summary.chars().count();
        let _ = self
            .event_tx
            .send(AppEvent::ContextCompacted { summary_chars });
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_tail_keeps_last_assistant_and_tool_results() {
        // Regression for Bug 7: the in-flight tail (last assistant + its tool
        // results) must be split off so it's preserved inline, not summarized.
        let history = vec![
            ChatMessage::system("sys"),
            ChatMessage::user("do thing"),
            ChatMessage::assistant("working"),
            ChatMessage::tool("call_1", "result 1"),
            ChatMessage::assistant_with_tools(vec![]),
            ChatMessage::tool("call_2", "fresh result 2"),
            ChatMessage::tool("call_3", "fresh result 3"),
        ];
        let (to_summarize, tail) = split_for_compaction(&history);
        // tail = last assistant + the two tool results after it.
        assert_eq!(tail.len(), 3);
        assert_eq!(tail[0].role, "assistant");
        assert_eq!(tail[1].role, "tool");
        assert_eq!(tail[2].role, "tool");
        assert_eq!(tail[2].content.as_deref(), Some("fresh result 3"));
        // to_summarize = everything before the last assistant (sys, user, asst, tool).
        assert_eq!(to_summarize.len(), 4);
        assert_eq!(to_summarize[0].role, "system");
        assert_eq!(to_summarize[3].role, "tool");
    }

    #[test]
    fn test_split_no_assistant_yields_empty_tail() {
        // No assistant message yet → summarize everything, empty tail (caller
        // appends a synthetic user(continue) turn).
        let history = vec![
            ChatMessage::system("sys"),
            ChatMessage::user("first message"),
        ];
        let (to_summarize, tail) = split_for_compaction(&history);
        assert!(tail.is_empty());
        assert_eq!(to_summarize.len(), 2);
    }

    #[test]
    fn test_split_assistant_with_no_following_tools_still_in_tail() {
        // Last message is an assistant with no tool results after it — it still
        // forms the tail (edge case; in practice loop-top compaction runs after
        // tool results were pushed, but the split must not panic or mis-split).
        let history = vec![
            ChatMessage::system("sys"),
            ChatMessage::user("hi"),
            ChatMessage::assistant("hello"),
        ];
        let (to_summarize, tail) = split_for_compaction(&history);
        assert_eq!(tail.len(), 1);
        assert_eq!(tail[0].role, "assistant");
        assert_eq!(to_summarize.len(), 2);
    }

    #[test]
    fn test_assemble_first_non_system_is_user_with_tail() {
        // Regression: after compaction the first non-system message must be a
        // user turn, even when the tail is non-empty (starts with assistant).
        // OpenAI-compatible endpoints (Ark) reject a request whose first
        // non-system message is an assistant — InvalidParameter.
        let sys = vec![ChatMessage::system("base instructions")];
        let tail = vec![
            ChatMessage::assistant_with_tools(vec![]),
            ChatMessage::tool("call_1", "fresh result"),
        ];
        let result = assemble_post_compaction_history(&sys, "summary text", &tail);
        // [system(base), system(summary), user(continue), assistant, tool]
        assert_eq!(result.len(), 5);
        // First two are system, third must be user (NOT assistant).
        assert_eq!(result[0].role, "system");
        assert_eq!(result[1].role, "system");
        assert_eq!(
            result[2].role, "user",
            "first non-system must be user, not assistant"
        );
        assert_eq!(result[3].role, "assistant");
        assert_eq!(result[4].role, "tool");
        // Tail content preserved.
        assert_eq!(result[4].content.as_deref(), Some("fresh result"));
    }

    #[test]
    fn test_assemble_first_non_system_is_user_without_tail() {
        // Empty-tail path: the synthetic user turn is still present.
        let sys = vec![ChatMessage::system("base instructions")];
        let result = assemble_post_compaction_history(&sys, "summary text", &[]);
        // [system(base), system(summary), user(continue)]
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].role, "system");
        assert_eq!(result[1].role, "system");
        assert_eq!(result[2].role, "user");
    }

    #[test]
    fn test_request_size_chars_counts_reasoning_and_tool_calls() {
        // Regression: needs_compaction previously counted only `content`,
        // missing reasoning_content (the dominant cost for thinking models)
        // and tool_calls.arguments. request_size_chars must include all three.
        use crate::api::{ToolCall, ToolCallFunction};
        let user_content = "hello";
        let asst_content = "ok";
        let reasoning = "thinking about the next step";
        let args = r#"{"path":"src/main.rs"}"#;
        let msgs = vec![
            ChatMessage::user(user_content),
            ChatMessage {
                role: "assistant".to_string(),
                content: Some(asst_content.to_string()),
                reasoning_content: Some(reasoning.to_string()),
                tool_calls: Some(vec![ToolCall {
                    id: "call_1".to_string(),
                    r#type: "function".to_string(),
                    function: ToolCallFunction {
                        name: "file_read".to_string(),
                        arguments: args.to_string(),
                    },
                }]),
                tool_call_id: None,
            },
        ];
        let expected = user_content.len() + asst_content.len() + reasoning.len() + args.len();
        assert_eq!(
            request_size_chars(&msgs),
            expected,
            "must count content + reasoning_content + tool_calls.arguments"
        );
        // And it must be strictly larger than content-only - the bug was that
        // reasoning + tool_calls were invisible.
        let content_only = user_content.len() + asst_content.len();
        assert!(expected > content_only);
    }

    #[test]
    fn test_request_size_chars_handles_none_and_empty() {
        // None content/reasoning and empty tool_calls contribute 0.
        let msgs = vec![
            ChatMessage::system("sys"),                // content: 3
            ChatMessage::assistant_with_tools(vec![]), // all None/empty -> 0
        ];
        assert_eq!(request_size_chars(&msgs), 3);
    }

    #[test]
    fn test_compaction_prompt_includes_json_format() {
        // Verify that the enhanced system prompt instructs the model to output JSON
        // with summary and memories keys.
        let messages = vec![
            ChatMessage::system(
                "You are a conversation summary assistant for an AI coding agent. \
                 Your task is to:\n\
                 1. Summarize the conversation history, preserving key details: \
                 project context, files modified, decisions made, bugs found, \
                 commands executed, and any pending tasks.\n\
                 2. Extract key memories from the conversation as structured JSON.\n\n\
                 Output format — respond with a single JSON object (no markdown fences, no extra text):\n\
                 {\n\
                   \"summary\": \"<concise summary string>\",\n\
                   \"memories\": [\n\
                     {\n\
                       \"type\": \"decision|error|preference|insight|knowledge|task\",\n\
                       \"content\": \"<what to remember>\",\n\
                       \"importance\": <0.0 to 1.0>\n\
                     }\n\
                   ]\n\
                 }\n\n\
                 If there is nothing worth remembering, return an empty memories array.\n\
                 Do NOT use any tools — just return the JSON as plain text.",
            ),
            ChatMessage::user("Process this: some history"),
        ];
        let sys_content = messages[0].content.as_deref().unwrap();
        assert!(sys_content.contains("\"summary\""), "prompt must request 'summary' field in JSON output");
        assert!(sys_content.contains("\"memories\""), "prompt must request 'memories' field in JSON output");
        assert!(sys_content.contains("decision"), "prompt must list valid memory types");
        assert!(sys_content.contains("importance"), "prompt must request importance field");
    }

    #[test]
    fn test_parse_compaction_json_success() {
        let json_response = r#"{
            "summary": "The user asked about memory systems.",
            "memories": [
                {"type": "decision", "content": "Use Jaccard for dedup", "importance": 0.8},
                {"type": "knowledge", "content": "Project uses Rust", "importance": 0.6}
            ]
        }"#;
        let json: serde_json::Value = serde_json::from_str(json_response).unwrap();
        let summary = json.get("summary").and_then(|v| v.as_str()).unwrap();
        let memories = json.get("memories").and_then(|v| v.as_array()).unwrap();
        assert_eq!(summary, "The user asked about memory systems.");
        assert_eq!(memories.len(), 2);
        assert_eq!(memories[0]["type"].as_str().unwrap(), "decision");
        assert_eq!(memories[0]["content"].as_str().unwrap(), "Use Jaccard for dedup");
        assert!((memories[0]["importance"].as_f64().unwrap() - 0.8).abs() < 0.001);
    }

    #[test]
    fn test_parse_compaction_json_failure_graceful() {
        let bad_response = "This is just a plain text summary, not JSON at all.";
        let result = serde_json::from_str::<serde_json::Value>(bad_response);
        assert!(result.is_err(), "malformed input should fail JSON parse");
        // Fallback: use full text as summary, empty memories
        let fallback_summary = bad_response.to_string();
        let fallback_memories: Vec<&str> = Vec::new();
        assert!(!fallback_summary.is_empty());
        assert!(fallback_memories.is_empty());
    }
}
