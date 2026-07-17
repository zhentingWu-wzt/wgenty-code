//! Shared auto-compaction (transcript archive + LLM summary + memory extract).
//!
//! Used by TUI and CLI headless paths via the [`Compactor`] port. Summarization
//! goes through [`LlmPort::chat_completion`] with tools disabled so the model
//! cannot answer with a tool_call (which would leave content empty).

use super::compaction::{
    assemble_post_compaction_history, micro_compact_messages, request_size_chars,
    split_for_compaction,
};
use super::ports::{Compactor, HistoryStore, LlmPort};
use crate::api::ChatMessage;
use crate::context::{MemoryEntry, MemoryManager, MemoryOrigin, MemoryType};
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Arc;

/// Soft cap on summarizer transcript text (characters).
///
/// Keeps the compaction chat request well under reverse-proxy / daemon body
/// limits and model context budgets. Head + tail are retained when truncated.
pub const COMPACTION_TRANSCRIPT_CHAR_CAP: usize = 100_000;

/// System prompt for the summarizer (JSON dual-output: summary + memories).
pub const COMPACTION_SYSTEM_PROMPT: &str = "\
You are a conversation summary assistant for an AI coding agent. \
Your task is to:\n\
1. Summarize the conversation history, preserving key details: \
project context, files modified, decisions made, bugs found, \
commands executed, and any pending tasks.\n\
2. Extract only durable, high-value memories as structured JSON.\n\n\
Output format — respond with a single JSON object (no markdown fences, no extra text):\n\
{\n\
  \"summary\": \"<concise summary string>\",\n\
  \"memories\": [\n\
    {\n\
      \"type\": \"decision|error|preference|insight|knowledge|task\",\n\
      \"scope\": \"project|global\",\n\
      \"content\": \"<what to remember>\",\n\
      \"importance\": <0.0 to 1.0>\n\
    }\n\
  ]\n\
}\n\n\
The \"scope\" field classifies where the memory should be stored:\n\
- \"project\": specific to the current project (architecture decisions, stable conventions, non-obvious bug conclusions).\n\
- \"global\": applies across all projects (user communication preferences, general workflow habits, cross-cutting insights about the user).\n\
When uncertain, default to \"project\".\n\n\
Memory quality rules (strict):\n\
- Prefer 0–3 memories. Empty is better than noise.\n\
- Only extract facts that will still matter in a future session.\n\
- importance >= 0.7 for durable decisions/preferences; use lower scores only when unsure.\n\
- DO NOT remember: current todo/task progress, one-off commands, temporary file paths, \
tool outputs, in-progress work, session chronology, or information already obvious from the repo.\n\
- DO remember: stable architecture choices, user long-term preferences, project conventions, \
non-obvious bug root causes and their fixes.\n\
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
                // Cap per-message content so a single huge tool result cannot
                // dominate the summarizer payload.
                const CONTENT_CAP: usize = 8_000;
                let mut chars = c.chars();
                let snippet: String = chars.by_ref().take(CONTENT_CAP).collect();
                let truncated = chars.next().is_some();
                parts.push(if truncated {
                    format!("{snippet}…(truncated)")
                } else {
                    snippet
                });
            }
            if let Some(tcs) = m.tool_calls.as_ref() {
                for tc in tcs {
                    const ARG_CAP: usize = 2_000;
                    let args = &tc.function.arguments;
                    let mut chars = args.chars();
                    let snippet: String = chars.by_ref().take(ARG_CAP).collect();
                    let truncated = chars.next().is_some();
                    parts.push(format!(
                        "tool_call: {}({}{})",
                        tc.function.name,
                        snippet,
                        if truncated { "…(truncated)" } else { "" }
                    ));
                }
            }
            parts.join("\n")
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Truncate a transcript to at most `max_chars`, keeping head + tail.
pub fn truncate_transcript_text(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_string();
    }
    if max_chars < 64 {
        return text.chars().take(max_chars).collect();
    }
    let marker = "\n\n…[transcript truncated for compaction request size]…\n\n";
    let marker_len = marker.chars().count();
    let budget = max_chars.saturating_sub(marker_len);
    let head_len = budget * 2 / 3;
    let tail_len = budget - head_len;
    let head: String = text.chars().take(head_len).collect();
    let tail: String = text
        .chars()
        .rev()
        .take(tail_len)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("{head}{marker}{tail}")
}

/// Whether an error string looks like HTTP 413 / body-size rejection.
pub fn is_payload_too_large_error(err: &str) -> bool {
    let lower = err.to_ascii_lowercase();
    lower.contains("413")
        || lower.contains("payload too large")
        || lower.contains("length limit exceeded")
        || lower.contains("body limit")
        || lower.contains("request body too large")
}

/// Micro-compact in place when LLM summary cannot run (e.g. 413).
///
/// Returns `true` if history was rewritten to a smaller form.
pub async fn fallback_micro_compact(history: &dyn HistoryStore) -> bool {
    let snap = history.get().await;
    let before = request_size_chars(&snap);
    let micro = micro_compact_messages(&snap);
    let after = request_size_chars(&micro);
    if after < before {
        history.replace(micro).await;
        tracing::info!(
            before_chars = before,
            after_chars = after,
            "compaction fallback: applied micro-compact after summary failure"
        );
        true
    } else {
        false
    }
}

/// Prepare summarizer input: micro-compact first, split tail, build + truncate transcript.
pub fn prepare_compaction_transcript(history: &[ChatMessage]) -> (Vec<ChatMessage>, String) {
    let micro = micro_compact_messages(history);
    let (to_summarize, tail) = split_for_compaction(&micro);
    let raw = build_transcript_text(&to_summarize);
    let transcript = truncate_transcript_text(&raw, COMPACTION_TRANSCRIPT_CHAR_CAP);
    (tail, transcript)
}

/// Default write-time importance gate for extracted memories.
pub const DEFAULT_WRITE_IMPORTANCE_THRESHOLD: f32 = 0.6;

/// Default cap on memories accepted from one compaction extract.
pub const DEFAULT_MAX_EXTRACT_PER_COMPACTION: usize = 3;

/// Filter + rank extracted memories before persistence.
///
/// Drops empty/low-importance entries and ephemeral task noise, then keeps at
/// most `max_extract` highest-importance memories.
pub fn filter_extracted_memories(
    memories: Vec<(MemoryEntry, MemoryOrigin)>,
    write_importance_threshold: f32,
    max_extract: usize,
) -> Vec<(MemoryEntry, MemoryOrigin)> {
    let mut kept: Vec<(MemoryEntry, MemoryOrigin)> = memories
        .into_iter()
        .filter(|(memory, _)| {
            if memory.content.trim().is_empty() {
                return false;
            }
            if memory.importance < write_importance_threshold {
                return false;
            }
            // Task-type extracts are almost always session-ephemeral noise.
            if memory.memory_type == MemoryType::Task {
                return false;
            }
            !is_ephemeral_memory_content(&memory.content)
        })
        .collect();

    kept.sort_by(|a, b| {
        b.0.importance
            .partial_cmp(&a.0.importance)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    if max_extract == 0 {
        return Vec::new();
    }
    kept.truncate(max_extract);
    kept
}

/// Heuristic noise detector for common ephemeral extracts.
fn is_ephemeral_memory_content(content: &str) -> bool {
    let lower = content.to_ascii_lowercase();
    const NOISE_MARKERS: &[&str] = &[
        "todo list",
        "todo:",
        "in_progress",
        "in progress",
        "current task",
        "this session",
        "user asked",
        "user wants me to",
        "pending tasks",
        "working on",
        "next step",
        "i will now",
        "going to run",
    ];
    NOISE_MARKERS.iter().any(|marker| lower.contains(marker))
}

/// Parse summarizer output into `(summary, memories)`.
///
/// Falls back to the full text as summary when JSON is missing or invalid.
/// Does **not** apply write-time quality filters — callers should pass the
/// result through [`filter_extracted_memories`] before persisting.
pub fn parse_compaction_response(full_text: &str) -> (String, Vec<(MemoryEntry, MemoryOrigin)>) {
    match serde_json::from_str::<serde_json::Value>(full_text.trim()) {
        Ok(json) => {
            let summary = json
                .get("summary")
                .and_then(|v| v.as_str())
                .unwrap_or(full_text.trim())
                .to_string();
            let memories: Vec<(MemoryEntry, MemoryOrigin)> = json
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
                            let scope = match m
                                .get("scope")
                                .and_then(|v| v.as_str())
                                .unwrap_or("project")
                            {
                                "global" => MemoryOrigin::Global,
                                _ => MemoryOrigin::Project,
                            };
                            if content.is_empty() {
                                return None;
                            }
                            Some((
                                MemoryEntry::new(mem_type, content).with_importance(importance),
                                scope,
                            ))
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

        let (tail, transcript_text) = prepare_compaction_transcript(&history_snapshot);

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
                let err = e.to_string();
                tracing::warn!(error = %err, "compaction summary request failed");
                if is_payload_too_large_error(&err) && fallback_micro_compact(history).await {
                    self.status(
                        "Compaction summary hit a size limit; applied micro-compact fallback.",
                    );
                    return true;
                }
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
            if fallback_micro_compact(history).await {
                self.status(
                    "Compaction produced an empty summary; applied micro-compact fallback.",
                );
                return true;
            }
            self.status("Compaction produced an empty summary; continuing with full history.");
            return false;
        }

        if let Some(ref mm) = self.memory_manager {
            let filtered = filter_extracted_memories(
                extracted_memories,
                mm.write_importance_threshold(),
                mm.max_extract_per_compaction(),
            );
            for (memory, scope) in &filtered {
                if let Err(e) = mm.add_memory(memory.clone(), *scope).await {
                    tracing::warn!(
                        error = %e,
                        memory_id = %memory.id,
                        "failed to persist extracted memory"
                    );
                }
            }
            if !filtered.is_empty() {
                tracing::info!(count = filtered.len(), "extracted memories from compaction");
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
        assert_eq!(memories[0].0.content, "Use JWT");
        // Missing scope defaults to Project (conservative).
        assert_eq!(memories[0].1, MemoryOrigin::Project);
    }

    #[test]
    fn parse_scope_classification() {
        let raw = r#"{
            "summary": "Mixed work.",
            "memories": [
                {"type": "decision", "scope": "project", "content": "Use tokio", "importance": 0.8},
                {"type": "preference", "scope": "global", "content": "Reply in Chinese", "importance": 0.9},
                {"type": "knowledge", "scope": "unknown", "content": "Falls back to project", "importance": 0.5}
            ]
        }"#;
        let (_summary, memories) = parse_compaction_response(raw);
        assert_eq!(memories.len(), 3);
        assert_eq!(memories[0].1, MemoryOrigin::Project);
        assert_eq!(memories[1].1, MemoryOrigin::Global);
        // Unknown scope string defaults to Project.
        assert_eq!(memories[2].1, MemoryOrigin::Project);
    }

    #[test]
    fn parse_plain_text_falls_back() {
        let (summary, memories) = parse_compaction_response("just a plain summary");
        assert_eq!(summary, "just a plain summary");
        assert!(memories.is_empty());
    }

    #[test]
    fn filter_extracted_memories_drops_noise_and_caps() {
        let raw = vec![
            (
                MemoryEntry::new(MemoryType::Task, "current task: fix bug").with_importance(0.95),
                MemoryOrigin::Project,
            ),
            (
                MemoryEntry::new(MemoryType::Knowledge, "this session user asked for help")
                    .with_importance(0.9),
                MemoryOrigin::Project,
            ),
            (
                MemoryEntry::new(MemoryType::Knowledge, "noise low").with_importance(0.2),
                MemoryOrigin::Project,
            ),
            (
                MemoryEntry::new(MemoryType::Decision, "use dual-scope memory")
                    .with_importance(0.9),
                MemoryOrigin::Project,
            ),
            (
                MemoryEntry::new(MemoryType::Preference, "reply in Chinese").with_importance(0.85),
                MemoryOrigin::Global,
            ),
            (
                MemoryEntry::new(MemoryType::Insight, "keep prompts lean").with_importance(0.7),
                MemoryOrigin::Project,
            ),
            (
                MemoryEntry::new(MemoryType::Knowledge, "extra fact").with_importance(0.65),
                MemoryOrigin::Project,
            ),
        ];
        let kept = filter_extracted_memories(raw, 0.6, 3);
        assert_eq!(kept.len(), 3);
        assert!(kept.iter().all(|(m, _)| m.importance >= 0.6));
        assert!(kept.iter().all(|(m, _)| m.memory_type != MemoryType::Task));
        assert!(kept[0].0.importance >= kept[1].0.importance);
        assert!(kept.iter().any(|(m, _)| m.content.contains("dual-scope")));
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

    #[test]
    fn truncate_transcript_keeps_head_and_tail() {
        let text: String = (0..10_000).map(|_| 'x').collect();
        let out = truncate_transcript_text(&text, 200);
        assert!(out.chars().count() <= 200);
        assert!(out.contains("truncated"));
        assert!(out.starts_with('x'));
        assert!(out.ends_with('x'));
    }

    #[test]
    fn truncate_noop_when_under_cap() {
        let text = "short";
        assert_eq!(truncate_transcript_text(text, 100), "short");
    }

    #[test]
    fn detects_413_payload_errors() {
        assert!(is_payload_too_large_error(
            "API error (413 Payload Too Large): Failed to buffer the request body: length limit exceeded"
        ));
        assert!(is_payload_too_large_error("request body too large"));
        assert!(!is_payload_too_large_error("timeout connecting to host"));
    }

    #[test]
    fn build_transcript_caps_huge_content() {
        let huge: String = (0..20_000).map(|_| 'a').collect();
        let msgs = vec![ChatMessage::tool("id1", &huge)];
        let text = build_transcript_text(&msgs);
        assert!(text.contains("truncated"));
        assert!(text.chars().count() < huge.chars().count());
    }
}
