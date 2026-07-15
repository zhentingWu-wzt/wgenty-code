//! Anthropic format conversion functions - OpenAI ↔ Anthropic.

use super::types::*;
use crate::api::{
    ChatMessage, ChatResponse, Choice, ToolCall, ToolCallFunction, ToolDefinition, Usage,
};

// ── Conversion: ChatMessage -> Anthropic Messages ────────────────────────────

/// Convert our ChatMessage list to Anthropic messages + optional system prompt.
///
/// The system prompt is returned as a single-block array with an ephemeral
/// `cache_control` breakpoint so that Anthropic prompt-caching can reuse the
/// (large, static) system instructions across turns.
pub fn convert_messages_to_anthropic(
    messages: &[ChatMessage],
) -> (Vec<AnthropicMessage>, Option<Vec<AnthropicSystemBlock>>) {
    let mut anthropic_msgs = Vec::new();
    // Anthropic's `system` is a single top-level string, but our message list
    // can carry several system messages (assembled instructions + a
    // post-compaction summary, etc.). Concatenate them instead of letting each
    // one overwrite the last - otherwise compaction's summary would silently
    // erase the original system prompt on the Anthropic path.
    let mut system_parts: Vec<String> = Vec::new();

    for msg in messages {
        if msg.role == "system" {
            let content = msg.content.clone().unwrap_or_default();
            if !content.is_empty() {
                system_parts.push(content);
            }
            continue;
        }

        if msg.role == "assistant" {
            if let Some(tool_calls) = &msg.tool_calls {
                // Assistant message with tool calls -> content blocks
                let mut blocks = Vec::new();

                // Add text content if present
                if let Some(ref text) = msg.content {
                    if !text.is_empty() {
                        blocks.push(AnthropicContentBlock::Text {
                            text: text.clone(),
                            cache_control: None,
                        });
                    }
                }

                // Add tool_use blocks
                for tc in tool_calls {
                    let input: serde_json::Value =
                        serde_json::from_str(&tc.function.arguments).unwrap_or_default();
                    blocks.push(AnthropicContentBlock::ToolUse {
                        id: tc.id.clone(),
                        name: tc.function.name.clone(),
                        input,
                        cache_control: None,
                    });
                }

                anthropic_msgs.push(AnthropicMessage {
                    role: "assistant".to_string(),
                    content: AnthropicContentValue::Blocks(blocks),
                });
            } else {
                // Plain text assistant message
                let text = msg.content.clone().unwrap_or_default();
                anthropic_msgs.push(AnthropicMessage {
                    role: "assistant".to_string(),
                    content: AnthropicContentValue::String(text),
                });
            }
        } else if msg.role == "tool" {
            // Tool result -> user message with tool_result blocks
            let content = msg.content.clone().unwrap_or_default();
            let tool_call_id = msg.tool_call_id.clone().unwrap_or_default();

            anthropic_msgs.push(AnthropicMessage {
                role: "user".to_string(),
                content: AnthropicContentValue::Blocks(vec![AnthropicContentBlock::ToolResult {
                    tool_use_id: tool_call_id,
                    content,
                    cache_control: None,
                }]),
            });
        } else {
            // User message
            let text = msg.content.clone().unwrap_or_default();
            anthropic_msgs.push(AnthropicMessage {
                role: "user".to_string(),
                content: AnthropicContentValue::String(text),
            });
        }
    }

    let system_prompt = if system_parts.is_empty() {
        None
    } else {
        Some(vec![AnthropicSystemBlock {
            block_type: "text".to_string(),
            text: system_parts.join("\n\n"),
            cache_control: Some(CacheControl::ephemeral()),
        }])
    };
    (anthropic_msgs, system_prompt)
}

/// Convert ToolDefinition (OpenAI format) to Anthropic tool format.
/// Automatically detects web_search tools and uses the server-side
/// `web_search_20250305` type instead of a custom function tool.
///
/// An ephemeral `cache_control` breakpoint is attached to the **last** tool so
/// that the entire (static) tools array is cached by Anthropic prompt-caching.
pub fn convert_tools_to_anthropic(tools: &[ToolDefinition]) -> Vec<AnthropicToolDef> {
    let total = tools.len();
    tools
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let is_last = i + 1 == total;
            let cache = if is_last {
                Some(CacheControl::ephemeral())
            } else {
                None
            };
            if t.function.name == "web_search" {
                // Anthropic-native server-side web search
                AnthropicToolDef::WebSearch {
                    name: "web_search".to_string(),
                    cache_control: cache,
                }
            } else {
                // Standard custom function tool
                AnthropicToolDef::Custom {
                    name: t.function.name.clone(),
                    description: t.function.description.clone(),
                    input_schema: t.function.parameters.clone(),
                    cache_control: cache,
                }
            }
        })
        .collect()
}

/// Attach an ephemeral `cache_control` breakpoint to the last content block of
/// the second-to-last message in `messages`.
///
/// In the agent loop the final message is always the new user input; everything
/// before it is stable conversation history.  By marking the end of that prefix
/// we let Anthropic cache the entire history (system + tools + prior turns) and
/// only pay for processing the new user message.
pub fn apply_conversation_cache_breakpoint(messages: &mut [AnthropicMessage]) {
    // Need at least 2 messages: the cached prefix + the new user message.
    if messages.len() < 2 {
        return;
    }
    let idx = messages.len() - 2;
    let target = &mut messages[idx];

    match &mut target.content {
        AnthropicContentValue::Blocks(blocks) => {
            if let Some(last) = blocks.last_mut() {
                set_cache_control(last);
            }
        }
        AnthropicContentValue::String(text) => {
            // Convert plain-string content to a single text block so we can
            // attach cache_control (Anthropic requires block form for this).
            let block = AnthropicContentBlock::Text {
                text: std::mem::take(text),
                cache_control: Some(CacheControl::ephemeral()),
            };
            target.content = AnthropicContentValue::Blocks(vec![block]);
        }
    }
}

/// Set `cache_control` to ephemeral on any content block variant.
fn set_cache_control(block: &mut AnthropicContentBlock) {
    match block {
        AnthropicContentBlock::Text { cache_control, .. }
        | AnthropicContentBlock::ToolUse { cache_control, .. }
        | AnthropicContentBlock::ToolResult { cache_control, .. }
        | AnthropicContentBlock::ServerToolUse { cache_control, .. }
        | AnthropicContentBlock::WebSearchToolResult { cache_control, .. } => {
            *cache_control = Some(CacheControl::ephemeral());
        }
    }
}

// ── Conversion: Anthropic Response -> ChatResponse (OpenAI format) ───────────

pub fn convert_anthropic_response(resp: &AnthropicResponse) -> ChatResponse {
    let mut text_parts = Vec::new();
    let mut tool_calls = Vec::new();

    for block in &resp.content {
        match block {
            AnthropicContentBlock::Text { text, .. } => {
                text_parts.push(text.clone());
            }
            AnthropicContentBlock::ToolUse {
                id, name, input, ..
            } => {
                tool_calls.push(ToolCall {
                    id: id.clone(),
                    r#type: "function".to_string(),
                    function: ToolCallFunction {
                        name: name.clone(),
                        arguments: serde_json::to_string(input).unwrap_or_default(),
                    },
                });
            }
            AnthropicContentBlock::ServerToolUse {
                id, name, input, ..
            } => {
                // Anthropic server-side tool (e.g., web_search) - treat like ToolUse
                tool_calls.push(ToolCall {
                    id: id.clone(),
                    r#type: "function".to_string(),
                    function: ToolCallFunction {
                        name: name.clone(),
                        arguments: serde_json::to_string(input).unwrap_or_default(),
                    },
                });
            }
            _ => {}
        }
    }

    let content = if text_parts.is_empty() {
        None
    } else {
        Some(text_parts.join("\n"))
    };

    let finish_reason = match resp.stop_reason.as_deref() {
        Some("end_turn") => Some("stop".to_string()),
        Some("tool_use") => Some("tool_calls".to_string()),
        Some("max_tokens") => Some("length".to_string()),
        Some("stop_sequence") => Some("stop".to_string()),
        _ => None,
    };

    ChatResponse {
        id: resp.id.clone(),
        object: "chat.completion".to_string(),
        created: 0,
        model: resp.model.clone(),
        choices: vec![Choice {
            index: 0,
            message: ChatMessage {
                role: "assistant".to_string(),
                content,
                reasoning_content: None,
                tool_calls: if tool_calls.is_empty() {
                    None
                } else {
                    Some(tool_calls)
                },
                tool_call_id: None,
            },
            finish_reason,
        }],
        usage: Some(Usage {
            prompt_tokens: resp.usage.input_tokens,
            completion_tokens: resp.usage.output_tokens,
            total_tokens: resp.usage.input_tokens + resp.usage.output_tokens,
        }),
    }
}
