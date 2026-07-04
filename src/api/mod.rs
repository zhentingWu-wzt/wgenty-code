//! API Module - OpenAI/DeepSeek compatible API Client

pub mod anthropic;
/// Backward-compatible alias for the old module path.
pub use anthropic as anthropic_types;
pub mod error;
pub mod provider;
pub mod token_counter;
pub mod types;

// Re-export public types and error helpers so existing `crate::api::ChatMessage`
// style imports continue to work after the split. The error helpers are
// `pub(crate)`, so they're re-exported at the same visibility — `pub` would
// both warn (can't elevate `pub(crate)` items) and fail to expose them to
// `crate::api::format_api_error` callers elsewhere in the crate.
pub use types::*;
pub(crate) use error::*;

use crate::config::Settings;
use reqwest::Client;
use std::sync::Arc;
use std::time::Duration;

use anthropic::{
    convert_anthropic_response, convert_messages_to_anthropic, convert_tools_to_anthropic,
};
// `format_api_error` / `wrap_network_error` come from the `pub(crate) use error::*;`
// glob above — that single line both imports them for local use and re-exports
// them as `crate::api::format_api_error`. An explicit `use error::{...}` here
// would shadow the glob re-export with a private import and break the
// `crate::api::format_api_error` call site in `daemon/handlers.rs`.
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
            .timeout(Duration::from_secs(settings.models.transport.timeout))
            .build()
            .unwrap_or_else(|e| {
                tracing::error!(error = %e, "failed to build HTTP client, using default");
                Client::default()
            });

        let provider: Arc<dyn Provider> =
            provider::detect_provider(&settings.models.main.endpoint_base_url());

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
        self.settings.models.main.endpoint_api_key()
    }

    pub fn get_base_url(&self) -> String {
        self.settings.models.main.endpoint_base_url()
    }

    /// Build a full endpoint URL from `base_url` and a path suffix like
    /// `chat/completions` or `messages`.
    ///
    /// If `base_url` already ends in a version segment (`/v1`, `/v2`, `/v3`, …),
    /// the suffix is appended directly. Otherwise `/v1/` is inserted to keep
    /// backward compatibility with bases that point at the API root
    /// (e.g. `https://api.openai.com`).
    fn build_endpoint(&self, suffix: &str) -> String {
        let base = self.get_base_url();
        let trimmed = base.trim_end_matches('/');
        // Detect a trailing `/v<digits>` segment.
        let has_version = trimmed
            .rsplit('/')
            .next()
            .map(|seg| {
                seg.starts_with('v')
                    && seg.len() > 1
                    && seg[1..].chars().all(|c| c.is_ascii_digit())
            })
            .unwrap_or(false);
        if has_version {
            format!("{}/{}", trimmed, suffix)
        } else {
            format!("{}/v1/{}", trimmed, suffix)
        }
    }

    pub fn get_model(&self) -> &str {
        &self.settings.models.main.name
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
            model: self
                .provider
                .resolve_model_id(&self.settings.models.main.name),
            messages,
            max_tokens: self.settings.models.transport.max_tokens,
            stream: false,
            temperature: 0.7,
            tools,
            stream_options: None,
        };

        let url = self.build_endpoint("chat/completions");

        let response = self
            .http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| wrap_network_error(e, self.provider.name()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|e| format!("[failed to read error body: {}]", e));
            return Err(anyhow::anyhow!("{}", format_api_error(status, &body)));
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
            model: self
                .provider
                .resolve_model_id(&self.settings.models.main.name),
            messages: anthropic_msgs,
            max_tokens: self.settings.models.transport.max_tokens,
            system: system_prompt,
            tools: anthropic_tools,
            stream: false,
        };

        let url = self.build_endpoint("messages");

        let response = self
            .http_client
            .post(&url)
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| wrap_network_error(e, self.provider.name()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "{}",
                format_api_error(status, &body)
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
            model: self
                .provider
                .resolve_model_id(&self.settings.models.main.name),
            messages,
            max_tokens: self.settings.models.transport.max_tokens,
            stream: true,
            temperature: 0.7,
            tools,
            stream_options: Some(StreamOptions {
                include_usage: true,
            }),
        };

        let url = self.build_endpoint("chat/completions");

        let response = self
            .http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| wrap_network_error(e, self.provider.name()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|e| format!("[failed to read error body: {}]", e));
            return Err(anyhow::anyhow!("{}", format_api_error(status, &body)));
        }

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
            model: self
                .provider
                .resolve_model_id(&self.settings.models.main.name),
            messages: anthropic_msgs,
            max_tokens: self.settings.models.transport.max_tokens,
            system: system_prompt,
            tools: anthropic_tools,
            stream: true,
        };

        let url = self.build_endpoint("messages");

        let response = self
            .http_client
            .post(&url)
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| wrap_network_error(e, self.provider.name()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "{}",
                format_api_error(status, &body)
            ));
        }

        // Stream Anthropic SSE events, convert to OpenAI-compatible SSE on the fly.
        // Uses an mpsc channel so the caller can start consuming SSE events immediately,
        // rather than waiting for the entire Anthropic response body to arrive.
        let byte_stream = response.bytes_stream();
        let (tx, rx) = futures::channel::mpsc::unbounded::<
            Result<bytes::Bytes, Box<dyn std::error::Error + Send + Sync>>,
        >();

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
                            buffer.drain(..=nl);

                            if line.is_empty() || !line.starts_with("data: ") {
                                continue;
                            }
                            if let Some(chunks) =
                                anthropic_types::parse_anthropic_sse_line(&line, &mut state)
                            {
                                for chunk in &chunks {
                                    // chunk is already formatted as "data: {...}" by
                                    // process_event(); just append the SSE double-newline
                                    let sse = format!("{}\n\n", chunk);
                                    if tx.unbounded_send(Ok(bytes::Bytes::from(sse))).is_err() {
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

pub type AnthropicClient = ApiClient;
