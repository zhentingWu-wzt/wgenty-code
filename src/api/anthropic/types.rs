//! Anthropic API request/response type definitions.

use serde::{Deserialize, Serialize};

// ── Anthropic Request Types ──────────────────────────────────────────────────

/// Anthropic prompt-caching directive.  When placed on the last content block
/// of a prefix region (system prompt, tools array, or a message), the API
/// caches everything up to that point so subsequent requests with the same
/// prefix pay only a cheap "cache read" instead of full reprocessing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheControl {
    #[serde(rename = "type")]
    pub cache_type: String,
}

impl CacheControl {
    /// Create an ephemeral cache breakpoint (5-minute TTL).
    pub fn ephemeral() -> Self {
        Self {
            cache_type: "ephemeral".to_string(),
        }
    }
}

/// A single block inside the top-level `system` field.  Anthropic requires the
/// array form (rather than a plain string) to attach `cache_control`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicSystemBlock {
    #[serde(rename = "type")]
    pub block_type: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<CacheControl>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AnthropicRequest {
    pub model: String,
    pub messages: Vec<AnthropicMessage>,
    pub max_tokens: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<Vec<AnthropicSystemBlock>>,
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
    Text {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        cache_control: Option<CacheControl>,
    },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        cache_control: Option<CacheControl>,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        cache_control: Option<CacheControl>,
    },
    #[serde(rename = "server_tool_use")]
    ServerToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        cache_control: Option<CacheControl>,
    },
    #[serde(rename = "web_search_tool_result")]
    WebSearchToolResult {
        tool_use_id: String,
        content: Vec<serde_json::Value>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        cache_control: Option<CacheControl>,
    },
}

/// Anthropic tool definition — supports both custom function tools and
/// server-side tools (like web_search_20250305).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum AnthropicToolDef {
    /// Anthropic native server-side web search (no function definition needed).
    #[serde(rename = "web_search_20250305")]
    WebSearch {
        name: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        cache_control: Option<CacheControl>,
    },
    /// Standard function/custom tool with description and input schema.
    #[serde(rename = "custom")]
    Custom {
        name: String,
        description: String,
        input_schema: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        cache_control: Option<CacheControl>,
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
