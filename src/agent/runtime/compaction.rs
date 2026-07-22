//! Pure conversation-compaction helpers (s06).
//!
//! These functions operate only on `ChatMessage` slices — no TUI, daemon, or
//! network dependencies — so every agent path can share the same policy.

use crate::api::ChatMessage;
use std::collections::{HashMap, HashSet};

/// Number of most-recent tool results kept verbatim by micro-compaction.
pub const MICRO_COMPACT_KEEP_TOOL_RESULTS: usize = 3;

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
pub fn split_for_compaction(history: &[ChatMessage]) -> (Vec<ChatMessage>, Vec<ChatMessage>) {
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
pub fn assemble_post_compaction_history(
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
        "Conversation history was just compacted. Continue the current task using the summary above.",
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
/// excluded — they're a fixed overhead the compaction threshold leaves
/// margin for. The previous content-only sum missed ~68% of the payload
/// (reasoning + tool_calls), so compaction never fired before the context
/// window overflowed — see change `fix-compaction-ignores-reasoning`.
pub fn request_size_chars(messages: &[ChatMessage]) -> usize {
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

/// Rough prompt-token estimate for UI progress (`chars / 4`).
///
/// Matches the same heuristic used by [`needs_compaction`]. Used after
/// successful compaction so the context bar reflects the rewritten history
/// before the next API `usage.prompt_tokens` arrives.
pub fn estimate_prompt_tokens(messages: &[ChatMessage]) -> usize {
    request_size_chars(messages) / 4
}

/// Calibration pair derived from the most recent real `usage.prompt_tokens`.
///
/// When available, the estimate uses the measured ratio
/// `last_measured_tokens / last_request_chars` instead of the default
/// `chars / 4`, which under-counts CJK text (actual ~1.5 char/token) and
/// reasoning-heavy payloads. This closes most of the gap between the local
/// estimate and the provider's real token count.
#[derive(Debug, Clone, Copy)]
pub struct Calibration {
    /// Real `prompt_tokens` reported by the provider on the last request.
    pub last_measured_tokens: usize,
    /// `request_size_chars` of the message slice that produced
    /// `last_measured_tokens`. Must be > 0 for the ratio to be usable.
    pub last_request_chars: usize,
}

/// Calibrated token estimate accounting for fixed request overhead.
///
/// Computes `(request_size_chars(messages) + fixed_overhead_chars) * ratio`
/// where `ratio = last_measured_tokens / last_request_chars` when a
/// [`Calibration`] is available, falling back to `(chars + overhead) / 4`
/// otherwise.
///
/// `fixed_overhead_chars` covers request body not present in `messages` -
/// primarily tool definitions when `use_tool_definitions` is on. The 8-layer
/// system prompt is already inside `messages` (prepended by the loop each
/// round) and therefore counted by [`request_size_chars`].
pub fn estimate_prompt_tokens_calibrated(
    messages: &[ChatMessage],
    fixed_overhead_chars: usize,
    calibration: Option<Calibration>,
) -> usize {
    let total_chars = request_size_chars(messages) + fixed_overhead_chars;
    match calibration {
        Some(c) if c.last_request_chars > 0 => {
            ((total_chars as u64) * (c.last_measured_tokens as u64) / (c.last_request_chars as u64))
                as usize
        }
        _ => total_chars / 4,
    }
}

/// Whether `messages` exceed the compaction threshold for the given window.
///
/// Compaction fires when the calibrated token estimate exceeds 80% of
/// `context_window`, *minus* `max_tokens` reserved for the model's output.
///
/// `fixed_overhead_chars` adds request-body cost not present in `messages`
/// (e.g. tool definitions). `calibration` switches the estimate from the
/// crude `chars / 4` to a ratio anchored on the last real
/// `usage.prompt_tokens`; pass `None` when no measurement is available yet
/// (first round of a turn).
///
/// Without reserving `max_tokens`, a large output budget lets input grow
/// until `input + max_tokens` overflows the window.
pub fn needs_compaction(
    messages: &[ChatMessage],
    context_window: usize,
    max_tokens: usize,
    fixed_overhead_chars: usize,
    calibration: Option<Calibration>,
) -> bool {
    let threshold = (context_window * 4 / 5).saturating_sub(max_tokens);
    estimate_prompt_tokens_calibrated(messages, fixed_overhead_chars, calibration) > threshold
}

/// Micro-compaction: replace old tool results with short markers.
///
/// Keeps the last [`MICRO_COMPACT_KEEP_TOOL_RESULTS`] tool messages as-is;
/// always preserves `file_read` / `read_file` results (reference material).
pub fn micro_compact_messages(history: &[ChatMessage]) -> Vec<ChatMessage> {
    let mut id_to_name: HashMap<String, String> = HashMap::new();
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

    let keep_start = tool_indices
        .len()
        .saturating_sub(MICRO_COMPACT_KEEP_TOOL_RESULTS);
    let keep_indices: HashSet<usize> = tool_indices[keep_start..].iter().copied().collect();

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
                if tool_name.map(|n| n.as_str()) == Some("file_read")
                    || tool_name.map(|n| n.as_str()) == Some("read_file")
                {
                    return msg.clone();
                }
                ChatMessage {
                    role: "tool".to_string(),
                    content: Some(format!(
                        "[Previous: used {}]",
                        tool_name.map_or("unknown tool", |n| n.as_str())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{ToolCall, ToolCallFunction};

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
        // And it must be strictly larger than content-only — the bug was that
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
    fn test_estimate_prompt_tokens_is_chars_div_4() {
        let msgs = vec![ChatMessage::user("abcd".repeat(10))]; // 40 chars
        assert_eq!(request_size_chars(&msgs), 40);
        assert_eq!(estimate_prompt_tokens(&msgs), 10);
    }

    #[test]
    fn test_needs_compaction_respects_max_tokens_reserve() {
        // With a small window and large max_tokens reserve, even modest history
        // should trigger compaction.
        let msgs = vec![ChatMessage::user("x".repeat(400))]; // ~100 tokens
        assert!(needs_compaction(&msgs, 200, 100, 0, None));
        // Same history under a large window stays below threshold.
        assert!(!needs_compaction(&msgs, 200_000, 4096, 0, None));
    }

    #[test]
    fn test_needs_compaction_accounts_for_fixed_overhead() {
        // context_window=1000, max_tokens=100 -> threshold = 1000*4/5-100 = 700.
        let msgs = vec![ChatMessage::user("x".repeat(400))]; // 400 chars / 4 = 100 tokens
                                                             // Without overhead: 100 < 700, no compaction.
        assert!(!needs_compaction(&msgs, 1000, 100, 0, None));
        // With 3000 chars of tool-definition overhead: (400+3000)/4 = 850 > 700,
        // triggers compaction - proving fixed overhead is counted.
        assert!(needs_compaction(&msgs, 1000, 100, 3000, None));
    }

    #[test]
    fn test_calibrated_estimate_uses_measured_ratio() {
        // CJK-like scenario: 400 chars measured as 300 real tokens (ratio
        // 0.75 tok/char) instead of the crude 0.25 (chars/4). The calibrated
        // estimate must be 3x the crude one.
        let msgs = vec![ChatMessage::user("x".repeat(400))];
        let crude = estimate_prompt_tokens(&msgs); // 100
        assert_eq!(crude, 100);
        let calibration = Calibration {
            last_measured_tokens: 300,
            last_request_chars: 400,
        };
        let calibrated = estimate_prompt_tokens_calibrated(&msgs, 0, Some(calibration));
        assert_eq!(calibrated, 300);
        // Fixed overhead is scaled by the same ratio.
        let with_overhead = estimate_prompt_tokens_calibrated(&msgs, 400, Some(calibration));
        assert_eq!(with_overhead, 600); // (400+400) * 300/400
    }

    #[test]
    fn test_calibrated_estimate_accurate_when_chars_include_overhead() {
        // Regression for the compaction inflation bug: the calibration
        // denominator (`last_request_chars`) must use the same basis as
        // `estimate_prompt_tokens_calibrated`'s `total_chars` - i.e. it must
        // include the fixed tool-definition overhead. `usage.prompt_tokens`
        // already counts the tool-definition tokens, so a denominator that
        // omits the tool bytes double-counts them and inflates the estimate,
        // triggering compaction far too early.
        //
        // Scenario: 400 bytes of messages + 400 bytes of tool defs. The
        // provider reports 300 real prompt_tokens (tools + messages combined).
        // With the fixed basis, last_request_chars = 800, so the estimate
        // equals the real count exactly. Under the old basis
        // (last_request_chars = 400, omitting overhead) the same inputs would
        // estimate 600 - 2x too high.
        let msgs = vec![ChatMessage::user("x".repeat(400))]; // 400 bytes
        let fixed_overhead = 400; // tool definitions
        let calibration = Calibration {
            last_measured_tokens: 300, // real prompt_tokens incl. tool defs
            last_request_chars: 800,   // 400 (messages) + 400 (overhead)
        };
        assert_eq!(
            estimate_prompt_tokens_calibrated(&msgs, fixed_overhead, Some(calibration)),
            300,
            "when the calibration denominator includes the overhead, the estimate must match the real token count"
        );
    }

    #[test]
    fn test_calibrated_estimate_falls_back_without_measurement() {
        let msgs = vec![ChatMessage::user("x".repeat(400))];
        // No calibration -> crude chars/4 including overhead.
        assert_eq!(
            estimate_prompt_tokens_calibrated(&msgs, 400, None),
            200 // (400+400)/4
        );
        // Calibration with zero chars is unusable -> fall back to crude.
        let bad = Calibration {
            last_measured_tokens: 300,
            last_request_chars: 0,
        };
        assert_eq!(
            estimate_prompt_tokens_calibrated(&msgs, 0, Some(bad)),
            100 // 400/4
        );
    }

    #[test]
    fn test_micro_compact_replaces_old_tools_keeps_recent_and_file_read() {
        let history = vec![
            ChatMessage::assistant_with_tools(vec![
                ToolCall {
                    id: "c1".to_string(),
                    r#type: "function".to_string(),
                    function: ToolCallFunction {
                        name: "grep".to_string(),
                        arguments: "{}".to_string(),
                    },
                },
                ToolCall {
                    id: "c2".to_string(),
                    r#type: "function".to_string(),
                    function: ToolCallFunction {
                        name: "file_read".to_string(),
                        arguments: "{}".to_string(),
                    },
                },
                ToolCall {
                    id: "c3".to_string(),
                    r#type: "function".to_string(),
                    function: ToolCallFunction {
                        name: "grep".to_string(),
                        arguments: "{}".to_string(),
                    },
                },
                ToolCall {
                    id: "c4".to_string(),
                    r#type: "function".to_string(),
                    function: ToolCallFunction {
                        name: "grep".to_string(),
                        arguments: "{}".to_string(),
                    },
                },
                ToolCall {
                    id: "c5".to_string(),
                    r#type: "function".to_string(),
                    function: ToolCallFunction {
                        name: "grep".to_string(),
                        arguments: "{}".to_string(),
                    },
                },
            ]),
            ChatMessage::tool("c1", "old grep result that should shrink"),
            ChatMessage::tool("c2", "file contents that must be preserved"),
            ChatMessage::tool("c3", "recent 1"),
            ChatMessage::tool("c4", "recent 2"),
            ChatMessage::tool("c5", "recent 3"),
        ];
        let compacted = micro_compact_messages(&history);
        // c1 is outside the last-3 window and not file_read → marker
        assert_eq!(
            compacted[1].content.as_deref(),
            Some("[Previous: used grep]")
        );
        // c2 is file_read → preserved even though outside last-3
        assert_eq!(
            compacted[2].content.as_deref(),
            Some("file contents that must be preserved")
        );
        // last 3 tool results kept
        assert_eq!(compacted[3].content.as_deref(), Some("recent 1"));
        assert_eq!(compacted[4].content.as_deref(), Some("recent 2"));
        assert_eq!(compacted[5].content.as_deref(), Some("recent 3"));
    }
}
