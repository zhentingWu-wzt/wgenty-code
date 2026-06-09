//! Anthropic format conversion functions — OpenAI ↔ Anthropic.

use super::types::*;
use crate::api::{
    ChatMessage, ChatResponse, Choice, ToolCall, ToolCallFunction, ToolDefinition, Usage,
};

// ── Conversion: ChatMessage → Anthropic Messages ────────────────────────────

/// Convert our ChatMessage list to Anthropic messages + optional system prompt.
pub fn convert_messages_to_anthropic(
    messages: &[ChatMessage],
) -> (Vec<AnthropicMessage>, Option<String>) {
    let mut anthropic_msgs = Vec::new();
    let mut system_prompt = None;

    for msg in messages {
        if msg.role == "system" {
            system_prompt = Some(msg.content.clone().unwrap_or_default());
            continue;
        }

        if msg.role == "assistant" {
            if let Some(tool_calls) = &msg.tool_calls {
                // Assistant message with tool calls → content blocks
                let mut blocks = Vec::new();

                // Add text content if present
                if let Some(ref text) = msg.content {
                    if !text.is_empty() {
                        blocks.push(AnthropicContentBlock::Text {
                            text: text.clone(),
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
            // Tool result → user message with tool_result blocks
            let content = msg.content.clone().unwrap_or_default();
            let tool_call_id = msg.tool_call_id.clone().unwrap_or_default();

            anthropic_msgs.push(AnthropicMessage {
                role: "user".to_string(),
                content: AnthropicContentValue::Blocks(vec![
                    AnthropicContentBlock::ToolResult {
                        tool_use_id: tool_call_id,
                        content,
                    },
                ]),
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

    (anthropic_msgs, system_prompt)
}

/// Convert ToolDefinition (OpenAI format) to Anthropic tool format.
/// Automatically detects web_search tools and uses the server-side
/// `web_search_20250305` type instead of a custom function tool.
pub fn convert_tools_to_anthropic(tools: &[ToolDefinition]) -> Vec<AnthropicToolDef> {
    tools
        .iter()
        .map(|t| {
            if t.function.name == "web_search" {
                // Anthropic-native server-side web search
                AnthropicToolDef::WebSearch {
                    name: "web_search".to_string(),
                }
            } else {
                // Standard custom function tool
                AnthropicToolDef::Custom {
                    name: t.function.name.clone(),
                    description: t.function.description.clone(),
                    input_schema: t.function.parameters.clone(),
                }
            }
        })
        .collect()
}

// ── Conversion: Anthropic Response → ChatResponse (OpenAI format) ───────────

pub fn convert_anthropic_response(resp: &AnthropicResponse) -> ChatResponse {
    let mut text_parts = Vec::new();
    let mut tool_calls = Vec::new();

    for block in &resp.content {
        match block {
            AnthropicContentBlock::Text { text } => {
                text_parts.push(text.clone());
            }
            AnthropicContentBlock::ToolUse { id, name, input } => {
                tool_calls.push(ToolCall {
                    id: id.clone(),
                    r#type: "function".to_string(),
                    function: ToolCallFunction {
                        name: name.clone(),
                        arguments: serde_json::to_string(input).unwrap_or_default(),
                    },
                });
            }
            AnthropicContentBlock::ServerToolUse { id, name, input } => {
                // Anthropic server-side tool (e.g., web_search) — treat like ToolUse
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
