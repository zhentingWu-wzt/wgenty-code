//! Anthropic SSE stream processing — converts Anthropic SSE to OpenAI format.

use std::collections::HashMap;

use super::types::*;
use crate::api::{Delta, StreamChoice, StreamChunk, StreamToolCall, StreamToolCallFunction};

/// Accumulated state for a single in-progress tool use content block.
struct ToolUseAccumulator {
    id: String,
    name: String,
    input: String,
}

pub struct AnthropicStreamState {
    pub message_id: Option<String>,
    pub model: Option<String>,
    pub content_blocks: Vec<AnthropicContentBlock>,
    /// Per-index accumulator for concurrent tool use blocks.
    /// Keyed by the content block `index` from the Anthropic SSE events.
    tool_use_accumulators: HashMap<usize, ToolUseAccumulator>,
    pub stop_reason: Option<String>,
    pub usage: Option<AnthropicUsage>,
}

impl Default for AnthropicStreamState {
    fn default() -> Self {
        Self::new()
    }
}

impl AnthropicStreamState {
    pub fn new() -> Self {
        Self {
            message_id: None,
            model: None,
            content_blocks: Vec::new(),
            tool_use_accumulators: HashMap::new(),
            stop_reason: None,
            usage: None,
        }
    }

    pub fn process_event(&mut self, event: &AnthropicSseEvent) -> Vec<String> {
        let mut sse_events = Vec::new();

        match event {
            AnthropicSseEvent::MessageStart { message } => {
                self.message_id = Some(message.id.clone());
                self.model = Some(message.model.clone());
                if let Some(usage) = &message.usage {
                    self.usage = Some(AnthropicUsage {
                        input_tokens: usage.input_tokens,
                        output_tokens: usage.output_tokens,
                    });
                }
            }
            AnthropicSseEvent::ContentBlockStart {
                index,
                content_block,
            } => match content_block {
                AnthropicContentBlock::Text { text } => {
                    let delta = Delta {
                        role: Some("assistant".to_string()),
                        content: Some(text.clone()),
                        reasoning_content: None,
                        tool_calls: None,
                    };
                    let chunk = StreamChunk {
                        id: self.message_id.clone().unwrap_or_default(),
                        object: "chat.completion.chunk".to_string(),
                        created: 0,
                        model: self.model.clone().unwrap_or_default(),
                        choices: vec![StreamChoice {
                            index: 0,
                            delta,
                            finish_reason: None,
                        }],
                        usage: None,
                    };
                    sse_events.push(format!(
                        "data: {}",
                        serde_json::to_string(&chunk).unwrap_or_default()
                    ));
                }
                AnthropicContentBlock::ToolUse { id, name, input: _ } => {
                    self.tool_use_accumulators.insert(
                        *index,
                        ToolUseAccumulator {
                            id: id.clone(),
                            name: name.clone(),
                            input: String::new(),
                        },
                    );
                }
                AnthropicContentBlock::ServerToolUse { id, name, input: _ } => {
                    self.tool_use_accumulators.insert(
                        *index,
                        ToolUseAccumulator {
                            id: id.clone(),
                            name: name.clone(),
                            input: String::new(),
                        },
                    );
                }
                _ => {}
            },
            AnthropicSseEvent::ContentBlockDelta { index, delta } => match delta {
                AnthropicSseDelta::TextDelta { text } => {
                    let d = Delta {
                        role: None,
                        content: Some(text.clone()),
                        reasoning_content: None,
                        tool_calls: None,
                    };
                    let chunk = StreamChunk {
                        id: self.message_id.clone().unwrap_or_default(),
                        object: "chat.completion.chunk".to_string(),
                        created: 0,
                        model: self.model.clone().unwrap_or_default(),
                        choices: vec![StreamChoice {
                            index: 0,
                            delta: d,
                            finish_reason: None,
                        }],
                        usage: None,
                    };
                    sse_events.push(format!(
                        "data: {}",
                        serde_json::to_string(&chunk).unwrap_or_default()
                    ));
                }
                AnthropicSseDelta::InputJsonDelta { partial_json } => {
                    if let Some(accum) = self.tool_use_accumulators.get_mut(index) {
                        accum.input.push_str(partial_json);
                    }
                }
            },
            AnthropicSseEvent::ContentBlockStop { index } => {
                if let Some(accum) = self.tool_use_accumulators.remove(index) {
                    let delta = Delta {
                        role: None,
                        content: None,
                        reasoning_content: None,
                        tool_calls: Some(vec![StreamToolCall {
                            index: *index as i32,
                            id: Some(accum.id),
                            r#type: Some("function".to_string()),
                            function: Some(StreamToolCallFunction {
                                name: Some(accum.name),
                                arguments: Some(accum.input),
                            }),
                        }]),
                    };
                    let chunk = StreamChunk {
                        id: self.message_id.clone().unwrap_or_default(),
                        object: "chat.completion.chunk".to_string(),
                        created: 0,
                        model: self.model.clone().unwrap_or_default(),
                        choices: vec![StreamChoice {
                            index: 0,
                            delta,
                            finish_reason: None,
                        }],
                        usage: None,
                    };
                    sse_events.push(format!(
                        "data: {}",
                        serde_json::to_string(&chunk).unwrap_or_default()
                    ));
                }
            }
            AnthropicSseEvent::MessageDelta { delta, usage } => {
                self.stop_reason = delta.stop_reason.clone();
                if let Some(u) = usage {
                    self.usage = Some(AnthropicUsage {
                        input_tokens: self.usage.as_ref().map(|u| u.input_tokens).unwrap_or(0),
                        output_tokens: u.output_tokens,
                    });
                }

                let finish_reason = match delta.stop_reason.as_deref() {
                    Some("end_turn") => Some("stop".to_string()),
                    Some("tool_use") => Some("tool_calls".to_string()),
                    Some("max_tokens") => Some("length".to_string()),
                    Some("stop_sequence") => Some("stop".to_string()),
                    _ => None,
                };

                let d = Delta {
                    role: None,
                    content: None,
                    reasoning_content: None,
                    tool_calls: None,
                };
                let chunk = StreamChunk {
                    id: self.message_id.clone().unwrap_or_default(),
                    object: "chat.completion.chunk".to_string(),
                    created: 0,
                    model: self.model.clone().unwrap_or_default(),
                    choices: vec![StreamChoice {
                        index: 0,
                        delta: d,
                        finish_reason,
                    }],
                    usage: self.usage.as_ref().map(|u| crate::api::Usage {
                        prompt_tokens: u.input_tokens,
                        completion_tokens: u.output_tokens,
                        total_tokens: u.input_tokens + u.output_tokens,
                    }),
                };
                sse_events.push(format!(
                    "data: {}",
                    serde_json::to_string(&chunk).unwrap_or_default()
                ));
            }
            AnthropicSseEvent::MessageStop => {
                // MessageDelta already emitted the correct finish_reason.
                // MessageStop only signals the [DONE] sentinel — don't emit
                // a duplicate chunk with a hardcoded "stop" finish_reason
                // that would override the real one (e.g. "tool_calls").
                sse_events.push("data: [DONE]".to_string());
            }
            _ => {}
        }

        sse_events
    }
}

/// Parse a single SSE line into Vec<String> of OpenAI-format SSE events.
pub fn parse_anthropic_sse_line(
    line: &str,
    state: &mut AnthropicStreamState,
) -> Option<Vec<String>> {
    let data = if let Some(rest) = line.strip_prefix("data: ") {
        rest
    } else {
        return Some(vec![]);
    };

    let event: AnthropicSseEvent = match serde_json::from_str(data) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(error = %e, raw = %data, "Failed to parse Anthropic SSE event");
            return Some(vec![]);
        }
    };
    Some(state.process_event(&event))
}

#[cfg(test)]
mod tests {
    use super::super::conversions::*;
    use super::super::types::*;
    use crate::api::{ChatMessage, ToolCall, ToolCallFunction, ToolDefinition};

    #[test]
    fn test_convert_system_message() {
        let msgs = vec![
            ChatMessage::system("You are helpful."),
            ChatMessage::user("hi"),
        ];
        let (anthropic_msgs, system) = convert_messages_to_anthropic(&msgs);
        assert_eq!(system, Some("You are helpful.".to_string()));
        assert_eq!(anthropic_msgs.len(), 1);
        assert_eq!(anthropic_msgs[0].role, "user");
    }

    #[test]
    fn test_convert_tool_result() {
        let msgs = vec![ChatMessage::tool("call_123", "result text")];
        let (anthropic_msgs, _) = convert_messages_to_anthropic(&msgs);
        assert_eq!(anthropic_msgs.len(), 1);
        assert_eq!(anthropic_msgs[0].role, "user");
        match &anthropic_msgs[0].content {
            AnthropicContentValue::Blocks(blocks) => {
                assert_eq!(blocks.len(), 1);
                match &blocks[0] {
                    AnthropicContentBlock::ToolResult {
                        tool_use_id,
                        content,
                    } => {
                        assert_eq!(tool_use_id, "call_123");
                        assert_eq!(content, "result text");
                    }
                    _ => panic!("expected ToolResult"),
                }
            }
            _ => panic!("expected Blocks"),
        }
    }

    #[test]
    fn test_convert_assistant_tool_calls() {
        let msgs = vec![ChatMessage {
            role: "assistant".to_string(),
            content: None,
            reasoning_content: None,
            tool_calls: Some(vec![ToolCall {
                id: "toolu_001".to_string(),
                r#type: "function".to_string(),
                function: ToolCallFunction {
                    name: "get_weather".to_string(),
                    arguments: r#"{"location":"NYC"}"#.to_string(),
                },
            }]),
            tool_call_id: None,
        }];
        let (anthropic_msgs, _) = convert_messages_to_anthropic(&msgs);
        assert_eq!(anthropic_msgs.len(), 1);
        match &anthropic_msgs[0].content {
            AnthropicContentValue::Blocks(blocks) => match &blocks[0] {
                AnthropicContentBlock::ToolUse { id, name, input } => {
                    assert_eq!(id, "toolu_001");
                    assert_eq!(name, "get_weather");
                    assert_eq!(input["location"], "NYC");
                }
                _ => panic!("expected ToolUse"),
            },
            _ => panic!("expected Blocks"),
        }
    }

    #[test]
    fn test_convert_tools_web_search_to_anthropic() {
        let tools = vec![ToolDefinition::new(
            "web_search",
            "search the web",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string"}
                }
            }),
        )];
        let anthropic_tools = convert_tools_to_anthropic(&tools);
        assert_eq!(anthropic_tools.len(), 1);
        match &anthropic_tools[0] {
            AnthropicToolDef::WebSearch { name } => {
                assert_eq!(name, "web_search");
            }
            _ => panic!("expected WebSearch variant"),
        }

        // Verify JSON serialization produces the right shape
        let json = serde_json::to_value(&anthropic_tools[0]).unwrap();
        assert_eq!(json["type"], "web_search_20250305");
        assert_eq!(json["name"], "web_search");
    }

    #[test]
    fn test_convert_tools_custom_to_anthropic() {
        let tools = vec![ToolDefinition::new(
            "get_weather",
            "gets weather",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "location": {"type": "string"}
                }
            }),
        )];
        let anthropic_tools = convert_tools_to_anthropic(&tools);
        assert_eq!(anthropic_tools.len(), 1);
        match &anthropic_tools[0] {
            AnthropicToolDef::Custom { name, .. } => {
                assert_eq!(name, "get_weather");
            }
            _ => panic!("expected Custom variant"),
        }
    }

    #[test]
    fn test_convert_response_text() {
        let resp = AnthropicResponse {
            id: "msg_001".to_string(),
            response_type: "message".to_string(),
            role: "assistant".to_string(),
            model: "claude-sonnet-4-6".to_string(),
            content: vec![AnthropicContentBlock::Text {
                text: "Hello!".to_string(),
            }],
            stop_reason: Some("end_turn".to_string()),
            stop_sequence: None,
            usage: AnthropicUsage {
                input_tokens: 10,
                output_tokens: 5,
            },
        };
        let chat_resp = convert_anthropic_response(&resp);
        assert_eq!(chat_resp.id, "msg_001");
        assert_eq!(
            chat_resp.choices[0].message.content,
            Some("Hello!".to_string())
        );
        assert_eq!(chat_resp.choices[0].finish_reason, Some("stop".to_string()));
        assert_eq!(chat_resp.usage.as_ref().unwrap().prompt_tokens, 10);
        assert_eq!(chat_resp.usage.as_ref().unwrap().completion_tokens, 5);
    }

    #[test]
    fn test_convert_response_tool_use() {
        let resp = AnthropicResponse {
            id: "msg_002".to_string(),
            response_type: "message".to_string(),
            role: "assistant".to_string(),
            model: "claude-sonnet-4-6".to_string(),
            content: vec![AnthropicContentBlock::ToolUse {
                id: "toolu_001".to_string(),
                name: "get_weather".to_string(),
                input: serde_json::json!({"location": "NYC"}),
            }],
            stop_reason: Some("tool_use".to_string()),
            stop_sequence: None,
            usage: AnthropicUsage {
                input_tokens: 10,
                output_tokens: 5,
            },
        };
        let chat_resp = convert_anthropic_response(&resp);
        assert_eq!(
            chat_resp.choices[0].finish_reason,
            Some("tool_calls".to_string())
        );
        let tool_calls = chat_resp.choices[0].message.tool_calls.as_ref().unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].id, "toolu_001");
        assert_eq!(tool_calls[0].function.name, "get_weather");
    }
}
