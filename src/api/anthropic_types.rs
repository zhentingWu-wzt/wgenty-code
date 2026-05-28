//! Anthropic Messages API types and OpenAI format conversion.
//!
//! Anthropic uses a different API format from OpenAI-compatible chat/completions:
//! - Endpoint: POST /v1/messages (not /v1/chat/completions)
//! - Auth: x-api-key header (not Authorization: Bearer)
//! - System prompt is a top-level field, not a message role
//! - Messages use content blocks ([{type: "text", text: "..."}]) instead of plain strings
//! - Tool calls appear as content blocks (type: "tool_use"), not a separate tool_calls array
//! - Tool results are user messages with content blocks (type: "tool_result")

use super::{
    ChatMessage, ChatResponse, Choice, Delta, StreamChoice, StreamChunk, StreamToolCall,
    StreamToolCallFunction, ToolCall, ToolCallFunction, ToolDefinition, Usage,
};
use serde::{Deserialize, Serialize};

// ── Anthropic Request Types ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct AnthropicRequest {
    pub model: String,
    pub messages: Vec<AnthropicMessage>,
    pub max_tokens: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<AnthropicTool>>,
    pub stream: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct AnthropicMessage {
    pub role: String,
    pub content: AnthropicContentValue,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum AnthropicContentValue {
    String(String),
    Blocks(Vec<AnthropicContentBlock>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AnthropicContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct AnthropicTool {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

// ── Anthropic Response Types ─────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct AnthropicResponse {
    pub id: String,
    #[serde(rename = "type")]
    pub response_type: String,
    pub role: String,
    pub model: String,
    pub content: Vec<AnthropicContentBlock>,
    pub stop_reason: Option<String>,
    pub stop_sequence: Option<String>,
    pub usage: AnthropicUsage,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AnthropicUsage {
    pub input_tokens: usize,
    pub output_tokens: usize,
}

// ── Anthropic SSE Event Types ────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum AnthropicSseEvent {
    #[serde(rename = "message_start")]
    MessageStart { message: AnthropicSseMessage },
    #[serde(rename = "content_block_start")]
    ContentBlockStart {
        index: usize,
        content_block: AnthropicContentBlock,
    },
    #[serde(rename = "content_block_delta")]
    ContentBlockDelta {
        index: usize,
        delta: AnthropicSseDelta,
    },
    #[serde(rename = "content_block_stop")]
    ContentBlockStop { index: usize },
    #[serde(rename = "message_delta")]
    MessageDelta {
        delta: AnthropicMessageDelta,
        usage: Option<AnthropicSseUsage>,
    },
    #[serde(rename = "message_stop")]
    MessageStop,
    #[serde(rename = "ping")]
    Ping,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AnthropicSseMessage {
    pub id: String,
    pub model: String,
    pub role: String,
    pub usage: Option<AnthropicUsage>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum AnthropicSseDelta {
    #[serde(rename = "text_delta")]
    TextDelta { text: String },
    #[serde(rename = "input_json_delta")]
    InputJsonDelta { partial_json: String },
}

#[derive(Debug, Clone, Deserialize)]
pub struct AnthropicMessageDelta {
    pub stop_reason: Option<String>,
    pub stop_sequence: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AnthropicSseUsage {
    pub output_tokens: usize,
}

// ── Conversion: ChatMessage → Anthropic Messages ────────────────────────────

/// Convert our ChatMessage list to Anthropic messages + optional system prompt.
pub fn convert_messages_to_anthropic(
    messages: &[ChatMessage],
) -> (Vec<AnthropicMessage>, Option<String>) {
    let mut anthropic_msgs = Vec::new();
    let mut system_prompt = None;

    for msg in messages {
        if msg.role == "system" {
            // Anthropic uses a top-level system field; only one system message is allowed
            let content = msg.content.as_deref().unwrap_or("");
            system_prompt = Some(match system_prompt {
                Some(existing) => format!("{}\n\n{}", existing, content),
                None => content.to_string(),
            });
        } else if msg.role == "tool" {
            // Tool results → user message with tool_result content block
            anthropic_msgs.push(AnthropicMessage {
                role: "user".to_string(),
                content: AnthropicContentValue::Blocks(vec![AnthropicContentBlock::ToolResult {
                    tool_use_id: msg.tool_call_id.clone().unwrap_or_default(),
                    content: msg.content.clone().unwrap_or_default(),
                }]),
            });
        } else if msg.role == "assistant" && msg.tool_calls.is_some() {
            // Assistant with tool_calls → assistant message with tool_use blocks
            let blocks: Vec<AnthropicContentBlock> = msg
                .tool_calls
                .as_ref()
                .unwrap()
                .iter()
                .map(|tc| AnthropicContentBlock::ToolUse {
                    id: tc.id.clone(),
                    name: tc.function.name.clone(),
                    input: serde_json::from_str(&tc.function.arguments)
                        .unwrap_or(serde_json::Value::Null),
                })
                .collect();
            anthropic_msgs.push(AnthropicMessage {
                role: "assistant".to_string(),
                content: AnthropicContentValue::Blocks(blocks),
            });
        } else {
            // User / assistant with text → simple string content
            let content = msg.content.as_deref().unwrap_or("");
            anthropic_msgs.push(AnthropicMessage {
                role: msg.role.clone(),
                content: AnthropicContentValue::String(content.to_string()),
            });
        }
    }

    (anthropic_msgs, system_prompt)
}

/// Convert ToolDefinition (OpenAI format) to Anthropic tool format.
pub fn convert_tools_to_anthropic(tools: &[ToolDefinition]) -> Vec<AnthropicTool> {
    tools
        .iter()
        .map(|t| AnthropicTool {
            name: t.function.name.clone(),
            description: t.function.description.clone(),
            input_schema: t.function.parameters.clone(),
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
            _ => {}
        }
    }

    let content = if text_parts.is_empty() {
        None
    } else {
        Some(text_parts.join(""))
    };

    let finish_reason = match resp.stop_reason.as_deref() {
        Some("end_turn") => Some("stop".to_string()),
        Some("tool_use") => Some("tool_calls".to_string()),
        Some("max_tokens") => Some("length".to_string()),
        Some("stop_sequence") => Some("stop".to_string()),
        _ => None,
    };

    let message = if tool_calls.is_empty() {
        ChatMessage::assistant(content.unwrap_or_default())
    } else {
        ChatMessage {
            role: "assistant".to_string(),
            content,
            reasoning_content: None,
            tool_calls: Some(tool_calls),
            tool_call_id: None,
        }
    };

    let created = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    ChatResponse {
        id: resp.id.clone(),
        object: "chat.completion".to_string(),
        created,
        model: resp.model.clone(),
        choices: vec![Choice {
            index: 0,
            message,
            finish_reason,
        }],
        usage: Some(Usage {
            prompt_tokens: resp.usage.input_tokens,
            completion_tokens: resp.usage.output_tokens,
            total_tokens: resp.usage.input_tokens + resp.usage.output_tokens,
        }),
    }
}

// ── Anthropic SSE → OpenAI StreamChunk conversion ───────────────────────────

/// State machine for converting Anthropic SSE events into OpenAI-compatible StreamChunks.
#[derive(Debug, Default)]
pub struct AnthropicStreamState {
    message_id: String,
    model: String,
    created: i64,
    /// Accumulated text per content block index
    text_by_index: std::collections::HashMap<usize, String>,
    /// Active tool_use id per content block index
    tool_id_by_index: std::collections::HashMap<usize, String>,
    /// Active tool_use name per content block index
    tool_name_by_index: std::collections::HashMap<usize, String>,
    /// Accumulated tool_use arguments JSON per content block index
    tool_args_by_index: std::collections::HashMap<usize, String>,
    /// Current content block type per index ("text" or "tool_use")
    block_type_by_index: std::collections::HashMap<usize, String>,
    /// Final message-level stop_reason
    stop_reason: Option<String>,
}

impl AnthropicStreamState {
    pub fn new() -> Self {
        Self {
            created: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64,
            ..Default::default()
        }
    }

    /// Process a single Anthropic SSE event and return zero or more OpenAI-compatible StreamChunks.
    pub fn process_event(&mut self, event: &AnthropicSseEvent) -> Vec<StreamChunk> {
        match event {
            AnthropicSseEvent::MessageStart { message } => {
                self.message_id = message.id.clone();
                self.model = message.model.clone();
                vec![StreamChunk {
                    id: self.message_id.clone(),
                    object: "chat.completion.chunk".to_string(),
                    created: self.created,
                    model: self.model.clone(),
                    choices: vec![StreamChoice {
                        index: 0,
                        delta: Delta {
                            role: Some("assistant".to_string()),
                            content: None,
                            reasoning_content: None,
                            tool_calls: None,
                        },
                        finish_reason: None,
                    }],
                }]
            }
            AnthropicSseEvent::ContentBlockStart {
                index,
                content_block,
            } => {
                match content_block {
                    AnthropicContentBlock::Text { text } => {
                        self.block_type_by_index.insert(*index, "text".to_string());
                        self.text_by_index.insert(*index, text.clone());
                    }
                    AnthropicContentBlock::ToolUse { id, name, input: _ } => {
                        self.block_type_by_index
                            .insert(*index, "tool_use".to_string());
                        self.tool_id_by_index.insert(*index, id.clone());
                        self.tool_name_by_index.insert(*index, name.clone());
                        self.tool_args_by_index.insert(*index, String::new());
                        // Emit a tool_call start chunk
                        return vec![StreamChunk {
                            id: self.message_id.clone(),
                            object: "chat.completion.chunk".to_string(),
                            created: self.created,
                            model: self.model.clone(),
                            choices: vec![StreamChoice {
                                index: 0,
                                delta: Delta {
                                    role: None,
                                    content: None,
                                    reasoning_content: None,
                                    tool_calls: Some(vec![StreamToolCall {
                                        index: *index as i32,
                                        id: Some(id.clone()),
                                        r#type: Some("function".to_string()),
                                        function: Some(StreamToolCallFunction {
                                            name: Some(name.clone()),
                                            arguments: Some(String::new()),
                                        }),
                                    }]),
                                },
                                finish_reason: None,
                            }],
                        }];
                    }
                    _ => {}
                }
                vec![]
            }
            AnthropicSseEvent::ContentBlockDelta { index, delta } => match delta {
                AnthropicSseDelta::TextDelta { text } => {
                    let entry = self.text_by_index.entry(*index).or_default();
                    entry.push_str(text);
                    vec![StreamChunk {
                        id: self.message_id.clone(),
                        object: "chat.completion.chunk".to_string(),
                        created: self.created,
                        model: self.model.clone(),
                        choices: vec![StreamChoice {
                            index: 0,
                            delta: Delta {
                                role: None,
                                content: Some(text.clone()),
                                reasoning_content: None,
                                tool_calls: None,
                            },
                            finish_reason: None,
                        }],
                    }]
                }
                AnthropicSseDelta::InputJsonDelta { partial_json } => {
                    let args = self.tool_args_by_index.entry(*index).or_default();
                    args.push_str(partial_json);
                    vec![StreamChunk {
                        id: self.message_id.clone(),
                        object: "chat.completion.chunk".to_string(),
                        created: self.created,
                        model: self.model.clone(),
                        choices: vec![StreamChoice {
                            index: 0,
                            delta: Delta {
                                role: None,
                                content: None,
                                reasoning_content: None,
                                tool_calls: Some(vec![StreamToolCall {
                                    index: *index as i32,
                                    id: None,
                                    r#type: None,
                                    function: Some(StreamToolCallFunction {
                                        name: None,
                                        arguments: Some(partial_json.clone()),
                                    }),
                                }]),
                            },
                            finish_reason: None,
                        }],
                    }]
                }
            },
            AnthropicSseEvent::ContentBlockStop { .. } => {
                // End of a content block — no chunk needed
                vec![]
            }
            AnthropicSseEvent::MessageDelta { delta, .. } => {
                self.stop_reason = delta.stop_reason.clone();
                vec![]
            }
            AnthropicSseEvent::MessageStop => {
                let finish_reason = match self.stop_reason.as_deref() {
                    Some("end_turn") => Some("stop".to_string()),
                    Some("tool_use") => Some("tool_calls".to_string()),
                    Some("max_tokens") => Some("length".to_string()),
                    Some("stop_sequence") => Some("stop".to_string()),
                    _ => Some("stop".to_string()),
                };
                vec![StreamChunk {
                    id: self.message_id.clone(),
                    object: "chat.completion.chunk".to_string(),
                    created: self.created,
                    model: self.model.clone(),
                    choices: vec![StreamChoice {
                        index: 0,
                        delta: Delta {
                            role: None,
                            content: None,
                            reasoning_content: None,
                            tool_calls: None,
                        },
                        finish_reason,
                    }],
                }]
            }
            AnthropicSseEvent::Ping | AnthropicSseEvent::Unknown => {
                vec![]
            }
        }
    }
}

/// Convert a line of Anthropic SSE into zero or more OpenAI-compatible StreamChunks.
///
/// Returns None for unparseable lines. Returns Some(vec![]) for events that don't
/// produce output chunks (e.g., ping, content_block_stop).
/// Returns Some(vec![...]) for events that produce one or more chunks.
pub fn parse_anthropic_sse_line(
    line: &str,
    state: &mut AnthropicStreamState,
) -> Option<Vec<StreamChunk>> {
    // Anthropic SSE format: "event: <type>\ndata: <json>"
    let data = if let Some(rest) = line.strip_prefix("data: ") {
        rest
    } else {
        // Not a data line; skip event type lines
        return Some(vec![]);
    };

    let event: AnthropicSseEvent = serde_json::from_str(data).ok()?;
    Some(state.process_event(&event))
}

#[cfg(test)]
mod tests {
    use super::*;

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
