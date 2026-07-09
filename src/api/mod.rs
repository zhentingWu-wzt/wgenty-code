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
pub(crate) use error::*;
pub use types::*;

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

/// Sanitize tool-call arguments before replaying a conversation to the API.
///
/// When the model generates a tool call with very long arguments (e.g. a large
/// `file_write`), the response can be truncated by `max_tokens` or a stream
/// interruption, leaving `tool_calls[].function.arguments` as invalid JSON.
/// Replaying that assistant message verbatim makes the provider reject every
/// subsequent request with `InvalidParameter: Invalid request body`.
///
/// This walks the messages and, for any assistant tool_call whose `arguments`
/// is not valid JSON, replaces it with a valid JSON object (lenient-extracted
/// partial fields, stripped of `_parse_error`/`_raw_arguments` meta keys,
/// falling back to `{}`). The `tool_call.id` is preserved so paired `tool`
/// response messages stay linked - dropping the tool_call would orphan its
/// tool response and trigger a *different* `InvalidParameter`.
fn sanitize_tool_call_args_for_replay(messages: &mut [ChatMessage]) {
    for msg in messages.iter_mut() {
        if msg.role != "assistant" {
            continue;
        }
        let Some(tool_calls) = msg.tool_calls.as_mut() else {
            continue;
        };
        for tc in tool_calls.iter_mut() {
            if serde_json::from_str::<serde_json::Value>(&tc.function.arguments).is_ok() {
                continue;
            }
            // Arguments are invalid JSON (truncated). Replace with a valid
            // object built from whatever fields the lenient parser recovered.
            let (partial, err) = crate::utils::lenient_json::parse_tool_args_lenient(
                &tc.function.arguments,
                &tc.function.name,
            );
            let cleaned = match partial {
                serde_json::Value::Object(mut map) => {
                    map.remove("_parse_error");
                    map.remove("_raw_arguments");
                    serde_json::Value::Object(map)
                }
                _ => serde_json::Value::Object(serde_json::Map::new()),
            };
            tracing::warn!(
                tool = %tc.function.name,
                error = %err.unwrap_or_default(),
                "replay: replaced truncated/invalid tool_call arguments with valid JSON \
                 to avoid InvalidParameter on subsequent requests"
            );
            tc.function.arguments = cleaned.to_string();
        }
    }
}

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

        let provider: Arc<dyn Provider> = provider::resolve_provider(
            &settings.models.main.endpoint_base_url(),
            settings.models.main.provider.as_deref(),
        );

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

    /// POST `body` to `url` with retry on transient network errors.
    ///
    /// Retries the send on connection failures, timeouts, and DNS errors — the
    /// flakiness that benefits from retry — with exponential backoff (2s, 4s,
    /// 8s, 16s; 5 total attempts). HTTP status errors (4xx/5xx) are NOT retried
    /// here; the caller inspects the returned `Response` status.
    ///
    /// Lives at the ApiClient layer so EVERY caller (main loop, subagents,
    /// planner, RLM, daemon, voice, MCP, knowledge) retries uniformly —
    /// previously only the main agent loop retried via `stream_with_retry`.
    async fn send_with_retry(
        &self,
        endpoint: &str,
        url: &str,
        headers: &[(&str, String)],
        body: &impl serde::Serialize,
    ) -> anyhow::Result<reqwest::Response> {
        const API_MAX_RETRIES: u32 = 4; // 5 total attempts
        let mut delay = 2u64;
        for attempt in 0..=API_MAX_RETRIES {
            let mut req = self.http_client.post(url);
            for (k, v) in headers {
                req = req.header(*k, v.as_str());
            }
            req = req.header("Content-Type", "application/json").json(body);
            match req.send().await {
                Ok(resp) => return Ok(resp),
                Err(e) => {
                    let msg = e.to_string().to_lowercase();
                    let transient = e.is_connect()
                        || e.is_timeout()
                        || msg.contains("dns")
                        || msg.contains("resolve")
                        || msg.contains("name or service");
                    if !transient || attempt == API_MAX_RETRIES {
                        return Err(wrap_network_error(e, self.provider.name()));
                    }
                    tracing::warn!(
                        attempt = attempt + 1,
                        max_attempts = API_MAX_RETRIES + 1,
                        endpoint = %endpoint,
                        retry_in_secs = delay,
                        error = %e,
                        "transient network error contacting {} API, retrying",
                        self.provider.name()
                    );
                    tokio::time::sleep(Duration::from_secs(delay)).await;
                    delay = delay.saturating_mul(2);
                }
            }
        }
        unreachable!()
    }

    pub fn get_model(&self) -> &str {
        &self.settings.models.main.name
    }

    pub async fn chat(
        &self,
        mut messages: Vec<ChatMessage>,
        tools: Option<Vec<ToolDefinition>>,
    ) -> anyhow::Result<ChatResponse> {
        sanitize_tool_call_args_for_replay(&mut messages);
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
            .send_with_retry(
                "chat/completions",
                &url,
                &[("Authorization", format!("Bearer {}", api_key))],
                &request,
            )
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|e| format!("[failed to read error body: {}]", e));
            // Diagnostic: Ark returns an empty `param` on InvalidParameter, so
            // the request body is the only way to identify the rejected field.
            tracing::warn!(
                "upstream rejected request — request body for diagnosis:\n{}",
                serde_json::to_string_pretty(&request).unwrap_or_default()
            );
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
            .send_with_retry(
                "messages",
                &url,
                &[
                    ("x-api-key", api_key.to_string()),
                    ("anthropic-version", "2023-06-01".to_string()),
                ],
                &request,
            )
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("{}", format_api_error(status, &body)));
        }

        let anthropic_resp: anthropic::AnthropicResponse = response.json().await?;
        Ok(convert_anthropic_response(&anthropic_resp))
    }

    pub async fn chat_stream(
        &self,
        mut messages: Vec<ChatMessage>,
        tools: Option<Vec<ToolDefinition>>,
    ) -> anyhow::Result<reqwest::Response> {
        sanitize_tool_call_args_for_replay(&mut messages);
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
            .send_with_retry(
                "chat/completions",
                &url,
                &[("Authorization", format!("Bearer {}", api_key))],
                &request,
            )
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|e| format!("[failed to read error body: {}]", e));
            // Diagnostic: Ark returns an empty `param` on InvalidParameter, so
            // the request body is the only way to identify the rejected field.
            tracing::warn!(
                "upstream rejected request — request body for diagnosis:\n{}",
                serde_json::to_string_pretty(&request).unwrap_or_default()
            );
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
            .send_with_retry(
                "messages",
                &url,
                &[
                    ("x-api-key", api_key.to_string()),
                    ("anthropic-version", "2023-06-01".to_string()),
                ],
                &request,
            )
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("{}", format_api_error(status, &body)));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn asst_with_args(id: &str, name: &str, args: &str) -> ChatMessage {
        ChatMessage {
            role: "assistant".to_string(),
            content: None,
            reasoning_content: None,
            tool_calls: Some(vec![ToolCall {
                id: id.to_string(),
                r#type: "function".to_string(),
                function: ToolCallFunction {
                    name: name.to_string(),
                    arguments: args.to_string(),
                },
            }]),
            tool_call_id: None,
        }
    }

    #[test]
    fn sanitize_replay_leaves_valid_args() {
        let mut msgs = vec![asst_with_args(
            "call_1",
            "file_write",
            r#"{"path":"a.txt","content":"hi"}"#,
        )];
        sanitize_tool_call_args_for_replay(&mut msgs);
        let args = &msgs[0].tool_calls.as_ref().unwrap()[0].function.arguments;
        let v: serde_json::Value = serde_json::from_str(args).unwrap();
        assert_eq!(v["path"], "a.txt");
        assert_eq!(v["content"], "hi");
    }

    #[test]
    fn sanitize_replay_replaces_truncated_args_and_keeps_linkage() {
        // Truncated mid-content: arguments is invalid JSON (no closing quote,
        // no `path` key). Before the fix this would be replayed verbatim and
        // make the provider reject every subsequent request.
        let truncated = r##"{"content":"# audit report\n\nvery long content cut"##;
        let mut msgs = vec![
            asst_with_args("call_x", "file_write", truncated),
            ChatMessage::tool("call_x", r#"{"success":false,"error":"path is required"}"#),
        ];
        sanitize_tool_call_args_for_replay(&mut msgs);

        // assistant args are now valid JSON...
        let args = &msgs[0].tool_calls.as_ref().unwrap()[0].function.arguments;
        let v: serde_json::Value = serde_json::from_str(args).unwrap();
        assert!(v.is_object());
        // ...id preserved, so the paired tool response is still linked (not orphaned)
        assert_eq!(msgs[0].tool_calls.as_ref().unwrap()[0].id, "call_x");
        assert_eq!(msgs[1].tool_call_id.as_deref(), Some("call_x"));
    }

    #[test]
    fn sanitize_replay_empty_when_unextractable() {
        // Mid-string truncation with no closing quote -> lenient extracts
        // nothing -> degrade to `{}` (still valid JSON).
        let truncated = r#"{"content":"unfinished"#;
        let mut msgs = vec![asst_with_args("call_y", "file_write", truncated)];
        sanitize_tool_call_args_for_replay(&mut msgs);
        let args = &msgs[0].tool_calls.as_ref().unwrap()[0].function.arguments;
        let v: serde_json::Value = serde_json::from_str(args).unwrap();
        assert!(
            v.as_object().unwrap().is_empty(),
            "expected {{}} when nothing extractable, got {args}"
        );
    }

    #[test]
    fn sanitize_replay_skips_non_assistant_messages() {
        // Only assistant tool_calls are sanitized; tool/user/system untouched.
        let mut msgs = vec![
            ChatMessage::user(r#"{"not":"json but user role"}"#),
            ChatMessage::system("sys"),
            ChatMessage::tool("call_z", r#"{"success":true}"#),
        ];
        sanitize_tool_call_args_for_replay(&mut msgs);
        assert_eq!(
            msgs[0].content.as_deref(),
            Some(r#"{"not":"json but user role"}"#)
        );
        assert_eq!(msgs[2].content.as_deref(), Some(r#"{"success":true}"#));
    }
}
