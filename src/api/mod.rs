//! API Module - OpenAI/DeepSeek compatible API Client

pub mod anthropic;
/// Backward-compatible alias for the old module path.
pub use anthropic as anthropic_types;
pub mod provider;
pub mod token_counter;

use crate::config::Settings;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

use anthropic::{
    convert_anthropic_response, convert_messages_to_anthropic, convert_tools_to_anthropic,
};
use provider::Provider;

#[derive(Clone)]
pub struct ApiClient {
    settings: Settings,
    http_client: Arc<Client>,
    provider: Arc<dyn Provider>,
}

impl ApiClient {
    pub fn new(settings: Settings) -> Self {
        let http_client = Client::builder()
            .timeout(Duration::from_secs(settings.api.timeout))
            .build()
            .unwrap_or_default();

        let provider: Arc<dyn Provider> =
            Arc::from(provider::detect_provider(&settings.api.get_base_url()));

        Self {
            settings,
            http_client: Arc::new(http_client),
            provider,
        }
    }

    /// The provider used by this client
    pub fn provider(&self) -> &dyn Provider {
        self.provider.as_ref()
    }

    pub fn get_api_key(&self) -> Option<String> {
        self.settings.api.get_api_key()
    }

    pub fn get_base_url(&self) -> String {
        self.settings.api.get_base_url()
    }

    pub fn get_model(&self) -> &str {
        &self.settings.model
    }

    pub async fn chat(
        &self,
        messages: Vec<ChatMessage>,
        tools: Option<Vec<ToolDefinition>>,
    ) -> anyhow::Result<ChatResponse> {
        let api_key = self
            .get_api_key()
            .ok_or_else(|| anyhow::anyhow!("API key not configured"))?;

        if self.provider.is_openai_compat() {
            self.chat_openai_compat(&api_key, messages, tools).await
        } else {
            self.chat_anthropic(&api_key, messages, tools).await
        }
    }

    async fn chat_openai_compat(
        &self,
        api_key: &str,
        messages: Vec<ChatMessage>,
        tools: Option<Vec<ToolDefinition>>,
    ) -> anyhow::Result<ChatResponse> {
        let request = ChatRequest {
            model: self.provider.resolve_model_id(&self.settings.model),
            messages,
            max_tokens: self.settings.api.max_tokens,
            stream: false,
            temperature: 0.7,
            tools,
        };

        let url = format!("{}/v1/chat/completions", self.get_base_url());

        let response = self
            .http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("API error ({}): {}", status, body));
        }

        let chat_response: ChatResponse = response.json().await?;
        Ok(chat_response)
    }

    async fn chat_anthropic(
        &self,
        api_key: &str,
        messages: Vec<ChatMessage>,
        tools: Option<Vec<ToolDefinition>>,
    ) -> anyhow::Result<ChatResponse> {
        let (anthropic_msgs, system_prompt) = convert_messages_to_anthropic(&messages);
        let anthropic_tools = tools.as_ref().map(|t| convert_tools_to_anthropic(t));

        let request = anthropic::AnthropicRequest {
            model: self.provider.resolve_model_id(&self.settings.model),
            messages: anthropic_msgs,
            max_tokens: self.settings.api.max_tokens,
            system: system_prompt,
            tools: anthropic_tools,
            stream: false,
        };

        let url = format!("{}/v1/messages", self.get_base_url());

        let response = self
            .http_client
            .post(&url)
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "Anthropic API error ({}): {}",
                status,
                body
            ));
        }

        let anthropic_resp: anthropic::AnthropicResponse = response.json().await?;
        Ok(convert_anthropic_response(&anthropic_resp))
    }

    pub async fn chat_stream(
        &self,
        messages: Vec<ChatMessage>,
        tools: Option<Vec<ToolDefinition>>,
    ) -> anyhow::Result<reqwest::Response> {
        let api_key = self
            .get_api_key()
            .ok_or_else(|| anyhow::anyhow!("API key not configured"))?;

        if self.provider.is_openai_compat() {
            self.chat_stream_openai_compat(&api_key, messages, tools)
                .await
        } else {
            self.chat_stream_anthropic(&api_key, messages, tools).await
        }
    }

    async fn chat_stream_openai_compat(
        &self,
        api_key: &str,
        messages: Vec<ChatMessage>,
        tools: Option<Vec<ToolDefinition>>,
    ) -> anyhow::Result<reqwest::Response> {
        let request = ChatRequest {
            model: self.provider.resolve_model_id(&self.settings.model),
            messages,
            max_tokens: self.settings.api.max_tokens,
            stream: true,
            temperature: 0.7,
            tools,
        };

        let url = format!("{}/v1/chat/completions", self.get_base_url());

        let response = self
            .http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;

        Ok(response)
    }

    /// Anthropic streaming: convert Anthropic SSE events to OpenAI-compatible SSE bytes,
    /// then return as a synthetic reqwest::Response so the REPL can parse uniformly.
    async fn chat_stream_anthropic(
        &self,
        api_key: &str,
        messages: Vec<ChatMessage>,
        tools: Option<Vec<ToolDefinition>>,
    ) -> anyhow::Result<reqwest::Response> {
        use anthropic::{
            convert_messages_to_anthropic, convert_tools_to_anthropic, AnthropicRequest,
        };

        let (anthropic_msgs, system_prompt) = convert_messages_to_anthropic(&messages);
        let anthropic_tools = tools.as_ref().map(|t| convert_tools_to_anthropic(t));

        let request = AnthropicRequest {
            model: self.provider.resolve_model_id(&self.settings.model),
            messages: anthropic_msgs,
            max_tokens: self.settings.api.max_tokens,
            system: system_prompt,
            tools: anthropic_tools,
            stream: true,
        };

        let url = format!("{}/v1/messages", self.get_base_url());

        let response = self
            .http_client
            .post(&url)
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "Anthropic API error ({}): {}",
                status,
                body
            ));
        }

        // Stream Anthropic SSE events, convert to OpenAI-compatible SSE on the fly.
        // Uses an mpsc channel so the caller can start consuming SSE events immediately,
        // rather than waiting for the entire Anthropic response body to arrive.
        let byte_stream = response.bytes_stream();
        let (tx, rx) = futures::channel::mpsc::unbounded::<
            Result<bytes::Bytes, Box<dyn std::error::Error + Send + Sync>>,
        >();

<<<<<<< HEAD
        tokio::spawn(async move {
            use anthropic_types::AnthropicStreamState;
            use futures::StreamExt;
            let mut stream = byte_stream;
            let mut state = AnthropicStreamState::new();
            let mut buffer = String::new();

            while let Some(chunk_result) = stream.next().await {
                match chunk_result {
                    Ok(b) => {
                        buffer.push_str(&String::from_utf8_lossy(&b));
                        // Process all complete lines from the buffer
                        while let Some(nl) = buffer.find('\n') {
                            let line = buffer[..nl].trim().to_string();
                            buffer = buffer[nl + 1..].to_string();

                            if line.is_empty() || !line.starts_with("data: ") {
                                continue;
                            }
                            if let Some(chunks) =
                                anthropic_types::parse_anthropic_sse_line(&line, &mut state)
                            {
                                for chunk in &chunks {
                                    let mut sse = String::from("data: ");
                                    sse.push_str(
                                        &serde_json::to_string(chunk).unwrap_or_default(),
                                    );
                                    sse.push_str("\n\n");
                                    if tx
                                        .unbounded_send(Ok(bytes::Bytes::from(sse)))
                                        .is_err()
                                    {
                                        return; // receiver dropped
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let _ = tx.unbounded_send(Err(Box::new(e)));
                        return;
                    }
=======
        for line in body.lines() {
            let line = line.trim().to_string();
            if line.is_empty() || !line.starts_with("data: ") {
                continue;
            }
            if let Some(chunks) = anthropic::parse_anthropic_sse_line(&line, &mut state) {
                for chunk in &chunks {
                    sse_out.push_str("data: ");
                    sse_out.push_str(&serde_json::to_string(chunk).unwrap_or_default());
                    sse_out.push('\n');
                    sse_out.push('\n');
>>>>>>> 5307f1d (优化代码)
                }
            }
            // Signal end of stream
            let _ = tx.unbounded_send(Ok(bytes::Bytes::from("data: [DONE]\n\n")));
        });

        let body = reqwest::Body::wrap_stream(rx);
        let http_resp = http::Response::builder()
            .status(200)
            .header("Content-Type", "text/event-stream")
            .body(body)
            .map_err(|e| anyhow::anyhow!("Failed to build response: {}", e))?;

        Ok(reqwest::Response::from(http_resp))
    }
}

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

#[derive(Debug, Clone, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    max_tokens: usize,
    stream: bool,
    temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ToolDefinition>>,
}

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

#[derive(Debug, Clone, Deserialize)]
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

/// Parse a single SSE line into a StreamChunk.
/// Returns None for `data: [DONE]` or unparseable lines.
pub fn parse_sse_line(line: &str) -> Option<StreamChunk> {
    let line = line.strip_prefix("data: ")?;
    if line == "[DONE]" {
        return None;
    }
    serde_json::from_str(line).ok()
}

pub type AnthropicClient = ApiClient;

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
        let msg = ChatMessage::tool("call_123", r#"{"success":true,"content":"..."}"#);
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""tool_call_id":"call_123""#));
        assert!(
            !json.contains(r#""tool_calls":null"#),
            "tool_calls should be omitted, got: {json}"
        );
    }

    #[test]
    fn test_user_message_serialization() {
        let msg = ChatMessage::user("hello");
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""content":"hello""#));
        assert!(!json.contains(r#"tool_calls"#));
        assert!(!json.contains(r#"tool_call_id"#));
    }
}
