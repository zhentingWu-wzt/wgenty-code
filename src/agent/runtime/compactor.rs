//! Shared auto-compaction (transcript archive + LLM summary + memory extract).
//!
//! Used by TUI and CLI headless paths via the [`Compactor`] port. Summarization
//! goes through [`LlmPort::chat_completion`] with tools disabled so the model
//! cannot answer with a tool_call (which would leave content empty).

use super::compaction::{assemble_post_compaction_history, split_for_compaction};
use super::ports::{Compactor, HistoryStore, LlmPort};
use crate::api::ChatMessage;
use crate::context::{MemoryEntry, MemoryManager, MemoryType};
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Arc;

/// System prompt for the summarizer (JSON dual-output: summary + memories).
pub const COMPACTION_SYSTEM_PROMPT: &str = "\
You are a conversation summary assistant for an AI coding agent. \
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
Do NOT use any tools — just return the JSON as plain text.";

/// Build plain-text transcript for the summarizer from already-seen messages.
pub fn build_transcript_text(to_summarize: &[ChatMessage]) -> String {
    to_summarize
        .iter()
        .map(|m| {
            let mut parts: Vec<String> = vec![format!("[{}]", m.role)];
            if let Some(rc) = m.reasoning_content.as_ref().filter(|s| !s.is_empty()) {
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
        .join("\n\n")
}

/// Parse summarizer output into `(summary, memories)`.
///
/// Falls back to the full text as summary when JSON is missing or invalid.
pub fn parse_compaction_response(full_text: &str) -> (String, Vec<MemoryEntry>) {
    match serde_json::from_str::<serde_json::Value>(full_text.trim()) {
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
                            let mem_type_str = m
                                .get("type")
                                .and_then(|v| v.as_str())
                                .unwrap_or("knowledge");
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
                            let importance =
                                m.get("importance").and_then(|v| v.as_f64()).unwrap_or(0.5) as f32;
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
            tracing::warn!(
                error = %e,
                "compaction response is not valid JSON; using full text as summary"
            );
            (full_text.trim().to_string(), Vec::new())
        }
    }
}

/// Archive full history JSON under `~/.wgenty-code/transcripts/`.
pub async fn archive_transcript(history: &[ChatMessage]) {
    let transcript_dir = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".wgenty-code")
        .join("transcripts");
    let _ = tokio::fs::create_dir_all(&transcript_dir).await;
    let timestamp = chrono::Utc::now().format("%Y-%m-%dT%H-%M-%S").to_string();
    let transcript_path = transcript_dir.join(format!("session_{}.json", timestamp));
    let json = serde_json::to_string_pretty(history).unwrap_or_default();
    let _ = tokio::fs::write(&transcript_path, json).await;
}

/// In-process auto-compactor for CLI / any path with a direct [`LlmPort`].
pub struct ApiCompactor {
    llm: Arc<dyn LlmPort>,
    system_messages: Vec<ChatMessage>,
    memory_manager: Option<Arc<MemoryManager>>,
    /// Last successful summary (chars available via this field).
    pub last_summary: Arc<tokio::sync::Mutex<String>>,
    /// Optional sink for human-readable status lines (CLI stderr).
    on_status: Option<Arc<dyn Fn(String) + Send + Sync>>,
}

impl ApiCompactor {
    pub fn new(
        llm: Arc<dyn LlmPort>,
        system_messages: Vec<ChatMessage>,
        memory_manager: Option<Arc<MemoryManager>>,
    ) -> Self {
        Self {
            llm,
            system_messages,
            memory_manager,
            last_summary: Arc::new(tokio::sync::Mutex::new(String::new())),
            on_status: None,
        }
    }

    pub fn with_status_sink(mut self, sink: impl Fn(String) + Send + Sync + 'static) -> Self {
        self.on_status = Some(Arc::new(sink));
        self
    }

    fn status(&self, msg: impl Into<String>) {
        if let Some(ref sink) = self.on_status {
            sink(msg.into());
        }
    }
}

#[async_trait]
impl Compactor for ApiCompactor {
    async fn compact(&self, history: &dyn HistoryStore) -> bool {
        let history_snapshot = history.get().await;
        archive_transcript(&history_snapshot).await;

        let (to_summarize, tail) = split_for_compaction(&history_snapshot);
        let transcript_text = build_transcript_text(&to_summarize);

        let summary_messages = vec![
            ChatMessage::system(COMPACTION_SYSTEM_PROMPT),
            ChatMessage::user(format!(
                "Process this conversation history:\n\n{}",
                transcript_text
            )),
        ];

        // No tools — same intent as TUI plan_mode=true on the daemon path.
        let completion = match self.llm.chat_completion(summary_messages, None).await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "compaction summary request failed");
                self.status("Compaction failed; continuing with full history.");
                return false;
            }
        };

        let full_text = {
            let content = completion.message.content.unwrap_or_default();
            if !content.is_empty() {
                content
            } else {
                completion.message.reasoning_content.unwrap_or_default()
            }
        };

        let (summary, extracted_memories) = parse_compaction_response(&full_text);

        if summary.trim().is_empty() {
            tracing::warn!("compaction produced an empty summary; leaving history intact");
            self.status("Compaction produced an empty summary; continuing with full history.");
            return false;
        }

        if let Some(ref mm) = self.memory_manager {
            for memory in &extracted_memories {
                if let Err(e) = mm
                    .add_memory(memory.clone(), crate::context::MemoryOrigin::Project)
                    .await
                {
                    tracing::warn!(
                        error = %e,
                        memory_id = %memory.id,
                        "failed to persist extracted memory"
                    );
                }
            }
            if !extracted_memories.is_empty() {
                tracing::info!(
                    count = extracted_memories.len(),
                    "extracted memories from compaction"
                );
            }
        }

        {
            *self.last_summary.lock().await = summary.clone();
        }
        let new_history = assemble_post_compaction_history(&self.system_messages, &summary, &tail);
        history.replace(new_history).await;

        let summary_chars = summary.chars().count();
        self.status(format!(
            "Context compacted (summary ~{} chars).",
            summary_chars
        ));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{ToolCall, ToolCallFunction};

    #[test]
    fn parse_valid_json_summary_and_memories() {
        let raw = r#"{
            "summary": "User refactored auth.",
            "memories": [
                {"type": "decision", "content": "Use JWT", "importance": 0.9},
                {"type": "knowledge", "content": "Rust project", "importance": 0.5}
            ]
        }"#;
        let (summary, memories) = parse_compaction_response(raw);
        assert_eq!(summary, "User refactored auth.");
        assert_eq!(memories.len(), 2);
        assert_eq!(memories[0].content, "Use JWT");
    }

    #[test]
    fn parse_plain_text_falls_back() {
        let (summary, memories) = parse_compaction_response("just a plain summary");
        assert_eq!(summary, "just a plain summary");
        assert!(memories.is_empty());
    }

    #[test]
    fn build_transcript_includes_tool_calls() {
        let msgs = vec![ChatMessage {
            role: "assistant".to_string(),
            content: Some("ok".to_string()),
            reasoning_content: None,
            tool_calls: Some(vec![ToolCall {
                id: "c1".to_string(),
                r#type: "function".to_string(),
                function: ToolCallFunction {
                    name: "file_read".to_string(),
                    arguments: r#"{"path":"a.rs"}"#.to_string(),
                },
            }]),
            tool_call_id: None,
        }];
        let text = build_transcript_text(&msgs);
        assert!(text.contains("tool_call: file_read"));
        assert!(text.contains("a.rs"));
    }
}
