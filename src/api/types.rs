//! API request/response types shared across providers.
//!
//! These structs model the OpenAI-compatible chat/completions wire format.
//! The Anthropic provider converts to/from this shape via [`super::anthropic`].

use serde::{Deserialize, Serialize};

// ── Tool definitions (request side) ──────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub r#type: String,
    pub function: ToolFunction,
}

impl ToolDefinition {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: serde_json::Value,
    ) -> Self {
        Self {
            r#type: "function".to_string(),
            function: ToolFunction {
                name: name.into(),
                description: description.into(),
                parameters,
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolFunction {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

// ── Tool calls (response side) ───────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub r#type: String,
    pub function: ToolCallFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallFunction {
    pub name: String,
    pub arguments: String,
}

// ── Chat messages ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".to_string(),
            content: Some(content.into()),
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".to_string(),
            content: Some(content.into()),
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn assistant_with_tools(tool_calls: Vec<ToolCall>) -> Self {
        Self {
            role: "assistant".to_string(),
            content: None,
            reasoning_content: None,
            tool_calls: Some(tool_calls),
            tool_call_id: None,
        }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".to_string(),
            content: Some(content.into()),
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn tool(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: "tool".to_string(),
            content: Some(content.into()),
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
        }
    }
}

/// Synthetic tool result injected when a tool call's execution was interrupted
/// (e.g. the user pressed Esc / Ctrl-C) so the message sequence stays
/// API-compliant: every assistant `tool_calls` block must be followed by a
/// matching `tool_result` for each call id.
pub const INTERRUPTED_TOOL_RESULT: &str = "[Tool execution interrupted by user]";

/// Ensure every assistant `tool_calls` block is followed by a matching
/// `tool_result` (role `"tool"` carrying the same `tool_call_id`).
///
/// A turn aborted mid-execution (Esc / Ctrl-C) leaves the shared
/// `conversation_history` with an orphaned assistant message whose
/// `tool_calls` have no results yet. If that history is saved to a session
/// and later restored, the next API request fails with
/// `MissingParameter: missing messages.tool_call_id`. This function repairs
/// such sequences by appending a synthetic [`INTERRUPTED_TOOL_RESULT`] for
/// every tool call id that lacks a result.
///
/// The scan is a single O(n) pass and is safe to run on every request as a
/// defensive boundary; it is a no-op for already well-formed histories.
pub fn sanitize_tool_call_pairing(messages: &mut Vec<ChatMessage>) {
    use std::collections::HashSet;

    let original = std::mem::take(messages);
    let mut result: Vec<ChatMessage> = Vec::with_capacity(original.len());
    let mut i = 0;
    while i < original.len() {
        let has_tool_calls = original[i]
            .tool_calls
            .as_ref()
            .is_some_and(|tc| !tc.is_empty());
        if original[i].role == "assistant" && has_tool_calls {
            let tool_calls = original[i].tool_calls.as_ref().unwrap();
            result.push(original[i].clone());

            // Collect tool_call_ids answered between this assistant and the
            // next assistant (inclusive of any role="tool" messages).
            let mut answered: HashSet<String> = HashSet::new();
            let mut j = i + 1;
            while j < original.len() && original[j].role != "assistant" {
                if original[j].role == "tool" {
                    if let Some(id) = &original[j].tool_call_id {
                        answered.insert(id.clone());
                    }
                }
                j += 1;
            }

            // Preserve the intervening messages verbatim.
            for msg in original.iter().take(j).skip(i + 1) {
                result.push(msg.clone());
            }

            // Backfill synthetic results for any unanswered tool call.
            for tc in tool_calls {
                if !answered.contains(&tc.id) {
                    result.push(ChatMessage::tool(&tc.id, INTERRUPTED_TOOL_RESULT));
                }
            }
            i = j;
        } else {
            result.push(original[i].clone());
            i += 1;
        }
    }
    *messages = result;
}

/// Demote `role="tool"` messages whose `tool_call_id` has no matching
/// preceding assistant `tool_calls` entry.
///
/// Sessions persisted by older builds dropped both `tool_call_id` and
/// `tool_calls` during save (the `SessionMessage` struct lacked those
/// fields). On restore the `tool` result arrives with `tool_call_id = None`
/// while the preceding assistant message lost its `tool_calls`, so replaying
/// the history verbatim makes the provider reject the request with
/// `MissingParameter: missing messages.tool_call_id` - and even if the id
/// were present, "tool message must follow a tool call". Demoting such an
/// orphan to a `user` message preserves the tool output as plain context
/// without breaking the call/result pairing contract the API enforces.
///
/// Well-formed histories (every `tool` message matches a preceding assistant
/// `tool_call`) are left untouched - this is a no-op for new sessions.
pub fn demote_orphan_tool_results(messages: &mut Vec<ChatMessage>) {
    use std::collections::HashSet;

    let original = std::mem::take(messages);
    let mut result: Vec<ChatMessage> = Vec::with_capacity(original.len());
    // tool_call ids emitted by preceding assistant messages still awaiting a
    // matching tool result. A tool message is "paired" iff its id is in here.
    let mut pending: HashSet<String> = HashSet::new();
    for msg in original {
        if msg.role == "assistant" {
            if let Some(calls) = &msg.tool_calls {
                for tc in calls {
                    pending.insert(tc.id.clone());
                }
            }
            result.push(msg);
            continue;
        }
        if msg.role == "tool" {
            let paired = msg
                .tool_call_id
                .as_ref()
                .is_some_and(|id| pending.remove(id));
            if paired {
                result.push(msg);
            } else {
                let content = msg.content.unwrap_or_default();
                tracing::warn!(
                    tool_call_id = ?msg.tool_call_id,
                    content_len = content.len(),
                    "demote orphan tool result to user message so the replayed \
                     history stays API-compliant (likely a session saved by an \
                     older build that lost tool_call_id/tool_calls)"
                );
                result.push(ChatMessage::user(format!(
                    "[Previous tool result, pairing lost on restore: {content}]"
                )));
            }
            continue;
        }
        result.push(msg);
    }
    *messages = result;
}

// ── Request types ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub max_tokens: usize,
    pub stream: bool,
    pub temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_options: Option<StreamOptions>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StreamOptions {
    pub include_usage: bool,
}

// ── Response types ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct ChatResponse {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub model: String,
    pub choices: Vec<Choice>,
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Choice {
    pub index: i32,
    pub message: ChatMessage,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: usize,
    pub completion_tokens: usize,
    pub total_tokens: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamChunk {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub model: String,
    pub choices: Vec<StreamChoice>,
    #[serde(default)]
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamChoice {
    pub index: i32,
    pub delta: Delta,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Delta {
    pub role: Option<String>,
    pub content: Option<String>,
    #[serde(default)]
    pub reasoning_content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<StreamToolCall>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamToolCall {
    pub index: i32,
    pub id: Option<String>,
    pub r#type: Option<String>,
    pub function: Option<StreamToolCallFunction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamToolCallFunction {
    pub name: Option<String>,
    pub arguments: Option<String>,
}

// ── SSE parsing ──────────────────────────────────────────────────────────────

/// Parse a single SSE line into a StreamChunk.
/// Returns None for `data: [DONE]` or unparseable lines.
pub fn parse_sse_line(line: &str) -> Option<StreamChunk> {
    let line = line.strip_prefix("data: ")?;
    if line == "[DONE]" {
        return None;
    }
    match serde_json::from_str(line) {
        Ok(chunk) => Some(chunk),
        Err(e) => {
            tracing::warn!(error = %e, raw = %line, "Failed to parse SSE chunk");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chat_message_serialization() {
        // Assistant with tool_calls, content=None → should NOT serialize content:null
        let msg = ChatMessage {
            role: "assistant".to_string(),
            content: None,
            reasoning_content: None,
            tool_calls: Some(vec![ToolCall {
                id: "call_123".to_string(),
                r#type: "function".to_string(),
                function: ToolCallFunction {
                    name: "file_read".to_string(),
                    arguments: r#"{"path":"README.md"}"#.to_string(),
                },
            }]),
            tool_call_id: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(
            !json.contains(r#""content":null"#),
            "content:null should not appear for tool_calls message, got: {json}"
        );
        assert!(json.contains(r#""tool_calls""#));
    }

    #[test]
    fn test_tool_result_message_serialization() {
        // Tool result message: role=tool, tool_call_id set, content set
        let msg = ChatMessage::tool("call_456", "file contents here");
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""role":"tool""#));
        assert!(json.contains(r#""tool_call_id":"call_456""#));
        assert!(json.contains(r#""content":"file contents here""#));
        // Should not have tool_calls or reasoning_content
        assert!(!json.contains(r#""tool_calls""#));
        assert!(!json.contains(r#""reasoning_content""#));
    }

    #[test]
    fn test_user_message_serialization() {
        let msg = ChatMessage::user("hello");
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""content":"hello""#));
        assert!(!json.contains(r#"tool_calls"#));
        assert!(!json.contains(r#"tool_call_id"#));
    }

    fn tool_call(id: &str, name: &str) -> ToolCall {
        ToolCall {
            id: id.to_string(),
            r#type: "function".to_string(),
            function: ToolCallFunction {
                name: name.to_string(),
                arguments: "{}".to_string(),
            },
        }
    }

    #[test]
    fn sanitize_trailing_orphan_assistant_gets_synthetic_results() {
        let mut msgs = vec![
            ChatMessage::user("please read the file"),
            ChatMessage::assistant_with_tools(vec![
                tool_call("a", "file_read"),
                tool_call("b", "grep"),
            ]),
        ];
        sanitize_tool_call_pairing(&mut msgs);
        assert_eq!(msgs.len(), 4, "two synthetic results appended");
        assert_eq!(msgs[2].role, "tool");
        assert_eq!(msgs[2].tool_call_id.as_deref(), Some("a"));
        assert_eq!(msgs[3].role, "tool");
        assert_eq!(msgs[3].tool_call_id.as_deref(), Some("b"));
        assert!(
            msgs[2].content.as_deref().unwrap().contains("interrupted"),
            "synthetic result should mention interruption"
        );
    }

    #[test]
    fn sanitize_partial_results_completed() {
        let mut msgs = vec![
            ChatMessage::assistant_with_tools(vec![
                tool_call("a", "file_read"),
                tool_call("b", "grep"),
            ]),
            ChatMessage::tool("a", "file content"),
        ];
        sanitize_tool_call_pairing(&mut msgs);
        assert_eq!(msgs.len(), 3, "only the missing 'b' result is added");
        assert_eq!(msgs[2].role, "tool");
        assert_eq!(msgs[2].tool_call_id.as_deref(), Some("b"));
    }

    #[test]
    fn sanitize_fully_paired_untouched() {
        let mut msgs = vec![
            ChatMessage::assistant_with_tools(vec![tool_call("a", "file_read")]),
            ChatMessage::tool("a", "file content"),
            ChatMessage::assistant("done"),
        ];
        let before = msgs.clone();
        sanitize_tool_call_pairing(&mut msgs);
        assert_eq!(msgs.len(), before.len(), "well-formed history unchanged");
        assert_eq!(msgs[0].role, "assistant");
        assert_eq!(msgs[1].role, "tool");
        assert_eq!(msgs[2].role, "assistant");
    }

    #[test]
    fn sanitize_no_tool_calls_untouched() {
        let mut msgs = vec![ChatMessage::user("hi"), ChatMessage::assistant("hello")];
        sanitize_tool_call_pairing(&mut msgs);
        assert_eq!(msgs.len(), 2);
    }

    #[test]
    fn sanitize_middle_orphan_between_assistants() {
        let mut msgs = vec![
            ChatMessage::assistant_with_tools(vec![tool_call("a", "file_read")]),
            // missing tool result for "a" before the next assistant turn
            ChatMessage::assistant("next turn"),
        ];
        sanitize_tool_call_pairing(&mut msgs);
        assert_eq!(
            msgs.len(),
            3,
            "synthetic result inserted between assistants"
        );
        assert_eq!(msgs[1].role, "tool");
        assert_eq!(msgs[1].tool_call_id.as_deref(), Some("a"));
        assert_eq!(msgs[2].role, "assistant");
    }

    /// Build a `role="tool"` message with NO `tool_call_id` - the exact shape a
    /// session saved by an older build (which dropped tool_call_id/tool_calls)
    /// produces on restore.
    fn tool_result_no_id(content: &str) -> ChatMessage {
        ChatMessage {
            role: "tool".to_string(),
            content: Some(content.to_string()),
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    #[test]
    fn demote_old_session_orphan_tool_without_id_becomes_user() {
        // Old-session shape: the assistant lost its tool_calls and the tool
        // result lost its tool_call_id during save by an older build.
        let mut msgs = vec![
            ChatMessage::assistant(""),
            tool_result_no_id("file contents"),
        ];
        demote_orphan_tool_results(&mut msgs);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[1].role, "user", "orphan tool demoted to user");
        assert!(
            msgs[1]
                .content
                .as_deref()
                .unwrap()
                .contains("file contents"),
            "tool output preserved as context"
        );
        assert!(
            msgs.iter().all(|m| m.role != "tool"),
            "no role=tool messages remain (would fail API validation)"
        );
    }

    #[test]
    fn demote_paired_tool_result_untouched() {
        let mut msgs = vec![
            ChatMessage::assistant_with_tools(vec![tool_call("a", "file_read")]),
            ChatMessage::tool("a", "file content"),
        ];
        let before = msgs.clone();
        demote_orphan_tool_results(&mut msgs);
        assert_eq!(msgs.len(), before.len());
        assert_eq!(msgs[1].role, "tool", "paired tool result kept");
        assert_eq!(msgs[1].tool_call_id.as_deref(), Some("a"));
    }

    #[test]
    fn demote_tool_with_unmatched_id_becomes_user() {
        // tool_call_id present but no preceding assistant tool_call matches it.
        let mut msgs = vec![
            ChatMessage::assistant("hi"),
            ChatMessage::tool("ghost_id", "result"),
        ];
        demote_orphan_tool_results(&mut msgs);
        assert_eq!(msgs[1].role, "user", "unmatched-id tool demoted");
        assert!(msgs.iter().all(|m| m.role != "tool"));
    }

    #[test]
    fn demote_then_sanitize_yields_api_compliant_history() {
        // Full old-session repair pipeline: demote orphans, then backfill any
        // remaining assistant tool_calls missing results. After both passes no
        // role=tool message may lack a tool_call_id.
        let mut msgs = vec![
            ChatMessage::user("read the file"),
            // assistant that lost its tool_calls on save
            ChatMessage::assistant(""),
            // tool result that lost its tool_call_id on save
            tool_result_no_id("the file contents"),
        ];
        demote_orphan_tool_results(&mut msgs);
        sanitize_tool_call_pairing(&mut msgs);
        for m in &msgs {
            if m.role == "tool" {
                assert!(
                    m.tool_call_id.is_some(),
                    "tool message must have tool_call_id after repair"
                );
            }
        }
    }
}
