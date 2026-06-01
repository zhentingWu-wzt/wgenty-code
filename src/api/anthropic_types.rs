//! Anthropic Messages API types and OpenAI format conversion.
//!
//! Anthropic uses a different API format from OpenAI-compatible chat/completions:
//! - Endpoint: POST /v1/messages (not /v1/chat/completions)
//! - Auth: x-api-key header (not Authorization: Bearer)
//! - System prompt is a top-level field, not a message role
//! - Messages use content blocks ([{type: "text", text: "..."}]) instead of plain strings
//! - Tool calls appear as content blocks (type: "tool_use"), not a separate tool_calls array
//! - Tool results are user messages with content blocks (type: "tool_result")
//! - web_search is a server-side tool type (web_search_20250305), not a function tool

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
    pub tools: Option<Vec<AnthropicToolDef>>,
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
    #[serde(rename = "server_tool_use")]
    ServerToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "web_search_tool_result")]
    WebSearchToolResult {
        tool_use_id: String,
        content: Vec<serde_json::Value>,
    },
}

/// Anthropic tool definition — supports both custom function tools and
/// server-side tools (like web_search_20250305).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum AnthropicToolDef {
    /// Anthropic native server-side web search (no function definition needed).
    #[serde(rename = "web_search_20250305")]
    WebSearch { name: String },
    /// Standard function/custom tool with description and input schema.
    #[serde(rename = "custom")]
    Custom {
        name: String,
        description: String,
        input_schema: serde_json::Value,
    },
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

// ── Streaming: Anthropic SSE → OpenAI SSE ───────────────────────────────────

pub struct AnthropicStreamState {
    pub message_id: Option<String>,
    pub model: Option<String>,
    pub content_blocks: Vec<AnthropicContentBlock>,
    current_tool_use_id: Option<String>,
    current_tool_use_name: Option<String>,
    current_tool_use_input: String,
    pub stop_reason: Option<String>,
    pub usage: Option<AnthropicUsage>,
}

impl AnthropicStreamState {
    pub fn new() -> Self {
        Self {
            message_id: None,
            model: None,
            content_blocks: Vec::new(),
            current_tool_use_id: None,
            current_tool_use_name: None,
            current_tool_use_input: String::new(),
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
                index: _,
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
                    };
                    sse_events.push(format!(
                        "data: {}",
                        serde_json::to_string(&chunk).unwrap_or_default()
                    ));
                }
                AnthropicContentBlock::ToolUse { id, name, input: _ } => {
                    self.current_tool_use_id = Some(id.clone());
                    self.current_tool_use_name = Some(name.clone());
                    self.current_tool_use_input.clear();
                }
                AnthropicContentBlock::ServerToolUse { id, name, input: _ } => {
                    self.current_tool_use_id = Some(id.clone());
                    self.current_tool_use_name = Some(name.clone());
                    self.current_tool_use_input.clear();
                }
                _ => {}
            },
            AnthropicSseEvent::ContentBlockDelta { index: _, delta } => match delta {
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
                    };
                    sse_events.push(format!(
                        "data: {}",
                        serde_json::to_string(&chunk).unwrap_or_default()
                    ));
                }
                AnthropicSseDelta::InputJsonDelta { partial_json } => {
                    self.current_tool_use_input.push_str(partial_json);
                }
            },
            AnthropicSseEvent::ContentBlockStop { index: _ } => {
                if let (Some(id), Some(name)) =
                    (self.current_tool_use_id.take(), self.current_tool_use_name.take())
                {
                    let arguments = std::mem::take(&mut self.current_tool_use_input);
                    let delta = Delta {
                        role: None,
                        content: None,
                        reasoning_content: None,
                        tool_calls: Some(vec![StreamToolCall {
                            index: 0,
                            id: Some(id),
                            r#type: Some("function".to_string()),
                            function: Some(StreamToolCallFunction {
                                name: Some(name),
                                arguments: Some(arguments),
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
                        input_tokens: self
                            .usage
                            .as_ref()
                            .map(|u| u.input_tokens)
                            .unwrap_or(0),
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
                };
                sse_events.push(format!(
                    "data: {}",
                    serde_json::to_string(&chunk).unwrap_or_default()
                ));
            }
            AnthropicSseEvent::MessageStop => {
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
                        finish_reason: Some("stop".to_string()),
                    }],
                };
                sse_events.push(format!(
                    "data: {}",
                    serde_json::to_string(&chunk).unwrap_or_default()
                ));
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
    fn test_convert_tools_web_search_to_anthropic() {
        let tools = vec![ToolDefinition::new("web_search", "search the web", serde_json::json!({
            "type": "object",
            "properties": {
                "query": {"type": "string"}
            }
        }))];
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
        let tools = vec![ToolDefinition::new("get_weather", "gets weather", serde_json::json!({
            "type": "object",
            "properties": {
                "location": {"type": "string"}
            }
        }))];
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
