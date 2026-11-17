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
use std::sync::OnceLock;
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

/// Anthropic requires an explicit `max_tokens`; when `transport.max_tokens = 0`
/// ("omit the field" on OpenAI-compatible APIs), send this fallback instead.
const ANTHROPIC_MAX_TOKENS_FALLBACK: usize = 8192;

/// How long to wait before re-sending a rate-limited (429) request when the
/// server did not send a parseable `Retry-After` header.
const RATE_LIMIT_BACKOFF_BASE_SECS: u64 = 2;
/// Hard ceiling per 429 wait, so a server-supplied `Retry-After: 3600` does
/// not hang a subagent report phase for an hour.
const RATE_LIMIT_BACKOFF_MAX_SECS: u64 = 30;

/// Decide whether an HTTP response should be re-sent after waiting
/// `Duration::ZERO` (immediately, no backoff), some backoff duration, or not
/// at all (`None`). Extracted as a pure function so the retry policy is
/// unit-testable without a live endpoint.
fn rate_limit_retry_wait(
    status: reqwest::StatusCode,
    response: &reqwest::Response,
) -> Option<Duration> {
    if status != reqwest::StatusCode::TOO_MANY_REQUESTS {
        return None;
    }
    match response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            let value = value.trim();
            // Delay-seconds form.
            if let Ok(secs) = value.parse::<u64>() {
                return Some(Duration::from_secs(secs));
            }
            // HTTP-date form (e.g. "Wed, 21 Oct 2026 07:28:00 GMT").
            httpdate::parse_http_date(value).ok().map(|when| {
                match when.duration_since(std::time::SystemTime::now()) {
                    Ok(delta) => delta,
                    // Date already in the past: retry permitted immediately.
                    Err(_) => Duration::ZERO,
                }
            })
        }) {
        Some(wait) => Some(wait.min(Duration::from_secs(RATE_LIMIT_BACKOFF_MAX_SECS))),
        None => Some(Duration::from_secs(RATE_LIMIT_BACKOFF_BASE_SECS)),
    }
}

/// Configured process-wide cap on concurrent in-flight LLM requests, read
/// from `models.transport.max_concurrent_llm_requests` exactly once (the
/// first LLM request) and cached: the semaphore cannot be resized after
/// creation, so later settings hot-reloads intentionally do not change the
/// cap within the current process. A settings load failure falls back to the
/// serde default.
fn configured_max_concurrent_llm_requests() -> usize {
    static CACHE: OnceLock<usize> = OnceLock::new();
    *CACHE.get_or_init(|| {
        Settings::load()
            .map(|s| s.models.transport.max_concurrent_llm_requests)
            .unwrap_or_else(|_| crate::config::models::default_max_concurrent_llm_requests())
    })
}

/// Process-wide permit pool bounding concurrent in-flight LLM requests
/// (chat + streaming): main loop, subagents, RLM, planner, daemon requests.
///
/// Deliberately process-level instead of per-`ApiClient` state: the daemon
/// builds one `ApiClient` per request, so a per-instance semaphore would never
/// actually throttle anything. `max_concurrent_llm_requests = 0` disables the
/// cap entirely (see [`ApiClient::acquire_llm_inflight_permit`]).
static LLM_INFLIGHT_SEMAPHORE: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();

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

/// Strip `reasoning_content` from replayed history at the request boundary.
///
/// Most OpenAI-compatible providers (GLM/z.ai, OpenAI, self-hosted gateways)
/// don't need historical reasoning echoed back — resending it re-bills those
/// tokens on every turn. DeepSeek is the documented exception that requires
/// echo-back. `override_setting` (`models.transport.strip_reasoning_content`)
/// wins over the provider rule: `Some(true)` forces stripping, `Some(false)`
/// forces keeping, `None` follows the provider default.
///
/// Request boundary only: on-disk sessions, local replay and the compactor
/// keep their reasoning content untouched.
fn strip_reasoning_content_for_replay(
    messages: &mut [ChatMessage],
    provider: &dyn Provider,
    override_setting: Option<bool>,
) {
    if !provider.is_openai_compat() {
        return;
    }
    let strip = override_setting.unwrap_or_else(|| !provider.echoes_reasoning_content());
    if !strip {
        return;
    }
    for msg in messages.iter_mut() {
        msg.reasoning_content = None;
    }
}

#[derive(Clone)]
pub struct ApiClient {
    settings: Settings,
    http_client: Arc<Client>,
    /// Dedicated client for streaming requests.
    ///
    /// Unlike `http_client`, this has **no total `.timeout()`**. A total
    /// deadline covers the entire response body, so a long SSE stream
    /// (reasoning models, long outputs) gets killed mid-stream and surfaces as
    /// "error decoding response body" in the daemon's stream-chunk error path.
    /// Instead this client caps only the connect phase and the per-chunk idle
    /// time, so an active stream that keeps producing tokens stays alive
    /// indefinitely while a hung connection still aborts. `transport.timeout`
    /// is reused as that idle budget - i.e. "max seconds without progress".
    http_client_stream: Arc<Client>,
    provider: Arc<dyn Provider>,
}

impl ApiClient {
    pub fn new(settings: Settings) -> Self {
        let (http_client, http_client_stream) = Self::build_clients(&settings);
        Self::with_clients(settings, http_client, http_client_stream)
    }

    /// Build the pair of pooled `reqwest::Client`s (non-streaming + streaming)
    /// from `settings`. Shared by [`new`](Self::new) and by callers that reuse
    /// a single pooled client across many `ApiClient` instances (e.g. the
    /// daemon, which creates one `ApiClient` per request) so the HTTP
    /// keep-alive pool and TLS session cache survive across requests instead of
    /// being rebuilt - and re-handshaked - on every call.
    pub fn build_clients(settings: &Settings) -> (Arc<Client>, Arc<Client>) {
        let timeout = Duration::from_secs(settings.models.transport.timeout);

        // Non-streaming client: a total deadline is correct here - the whole
        // JSON response should arrive within `timeout`.
        let http_client = Client::builder()
            .timeout(timeout)
            .build()
            .unwrap_or_else(|e| {
                tracing::error!(error = %e, "failed to build HTTP client, using default");
                Client::default()
            });

        // Streaming client: no total deadline (see `http_client_stream` doc).
        // `connect_timeout` bounds connection setup; `read_timeout` bounds the
        // gap between SSE chunks (reset on each chunk received). Raise
        // `transport.timeout` if a model's first-token latency exceeds it.
        let http_client_stream = Client::builder()
            .connect_timeout(timeout)
            .read_timeout(timeout)
            .build()
            .unwrap_or_else(|e| {
                tracing::error!(
                    error = %e,
                    "failed to build streaming HTTP client, using default"
                );
                Client::default()
            });

        (Arc::new(http_client), Arc::new(http_client_stream))
    }

    /// Construct an `ApiClient` reusing externally-provided pooled clients.
    ///
    /// `settings` is still consumed per-instance so API key / base URL / model /
    /// provider are read fresh each call; only the `reqwest::Client` (connection
    /// pool + TLS session cache) is shared. Use this when many short-lived
    /// `ApiClient`s are created from the same config (e.g. one per daemon
    /// request) to avoid re-handshaking TLS on every call.
    pub fn with_clients(
        settings: Settings,
        http_client: Arc<Client>,
        http_client_stream: Arc<Client>,
    ) -> Self {
        let provider: Arc<dyn Provider> = provider::resolve_provider(
            &settings.models.main.endpoint_base_url(),
            settings.models.main.provider.as_deref(),
        );

        Self {
            settings,
            http_client,
            http_client_stream,
            provider,
        }
    }

    /// Wait for a slot in the process-wide LLM request cap. Called on every
    /// send so bursts from a wave of subagent spawns queue here instead of
    /// tripping provider rate limits (429). Holds the permit for the duration
    /// of the request, including 429 backoff waits, so a single client cannot
    /// re-enter and defeat the cap.
    async fn acquire_llm_inflight_permit() -> tokio::sync::OwnedSemaphorePermit {
        let configured = configured_max_concurrent_llm_requests();
        if configured == 0 {
            // Cap disabled: acquire from a permit pool that never runs out.
            static UNBOUNDED: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
            let semaphore =
                UNBOUNDED.get_or_init(|| Arc::new(tokio::sync::Semaphore::new(usize::MAX)));
            return tokio::sync::Semaphore::acquire_owned(Arc::clone(semaphore))
                .await
                .expect("unbounded semaphore has infinite permits");
        }
        let semaphore = LLM_INFLIGHT_SEMAPHORE
            .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(configured)));
        tokio::sync::Semaphore::acquire_owned(Arc::clone(semaphore))
            .await
            .expect("LLM inflight semaphore is never closed")
    }

    /// The provider used by this client
    pub fn provider(&self) -> &dyn Provider {
        self.provider.as_ref()
    }

    pub fn get_api_key(&self) -> Option<String> {
        if self.settings.models.active_model.is_some() {
            return self
                .settings
                .models
                .main
                .api_key
                .clone()
                .or_else(|| self.settings.models.main.endpoint_api_key());
        }
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
        self.send_with_retry_impl(&self.http_client, endpoint, url, headers, body)
            .await
    }

    /// Like `send_with_retry` but on the streaming client (no total timeout -
    /// see `http_client_stream`), so a long SSE stream isn't killed mid-body.
    async fn send_stream_with_retry(
        &self,
        endpoint: &str,
        url: &str,
        headers: &[(&str, String)],
        body: &impl serde::Serialize,
    ) -> anyhow::Result<reqwest::Response> {
        self.send_with_retry_impl(&self.http_client_stream, endpoint, url, headers, body)
            .await
    }

    async fn send_with_retry_impl(
        &self,
        client: &Client,
        endpoint: &str,
        url: &str,
        headers: &[(&str, String)],
        body: &impl serde::Serialize,
    ) -> anyhow::Result<reqwest::Response> {
        const API_MAX_RETRIES: u32 = 4; // 5 total attempts
        let mut delay = 2u64;
        let _permit = Self::acquire_llm_inflight_permit().await;
        for attempt in 0..=API_MAX_RETRIES {
            let mut req = client.post(url);
            for (k, v) in headers {
                req = req.header(*k, v.as_str());
            }
            req = req.header("Content-Type", "application/json").json(body);
            match req.send().await {
                Ok(resp) => {
                    let status = resp.status();
                    if let Some(wait) = rate_limit_retry_wait(status, &resp) {
                        if attempt < API_MAX_RETRIES {
                            tracing::warn!(
                                attempt = attempt + 1,
                                max_attempts = API_MAX_RETRIES + 1,
                                endpoint = %endpoint,
                                retry_after_secs = wait.as_secs(),
                                "rate limited (429) by {} API, backing off and re-sending",
                                self.provider.name()
                            );
                            if !wait.is_zero() {
                                tokio::time::sleep(wait).await;
                            }
                            continue;
                        }
                    }
                    return Ok(resp);
                }
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

    /// `transport.max_tokens = 0` means "omit the field" so the provider's
    /// default output limit applies (same omit pattern as `temperature`).
    /// Any positive value is sent verbatim as an explicit output cap.
    fn request_max_tokens(&self) -> Option<usize> {
        let limit = self.settings.models.transport.max_tokens;
        (limit > 0).then_some(limit)
    }

    pub async fn chat(
        &self,
        mut messages: Vec<ChatMessage>,
        tools: Option<Vec<ToolDefinition>>,
    ) -> anyhow::Result<ChatResponse> {
        sanitize_tool_call_args_for_replay(&mut messages);
        strip_reasoning_content_for_replay(
            &mut messages,
            self.provider.as_ref(),
            self.settings.models.transport.strip_reasoning_content,
        );
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
            max_tokens: self.request_max_tokens(),
            stream: false,
            temperature: self.settings.models.main.temperature,
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

        let status = response.status();
        if !status.is_success() {
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

        // Read body as text first so we can include it in decode errors.
        let body_text = response
            .text()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to read response body: {}", e))?;
        let chat_response: ChatResponse = serde_json::from_str(&body_text).map_err(|e| {
            let preview: String = body_text.chars().take(800).collect();
            tracing::error!(
                ?e,
                body_preview = %preview,
                "Failed to deserialize response body as ChatResponse"
            );
            anyhow::anyhow!(
                "error decoding response body: {}\n--- response body (first 800 chars) ---\n{}",
                e,
                preview
            )
        })?;
        Ok(chat_response)
    }

    async fn chat_anthropic(
        &self,
        api_key: &str,
        messages: Vec<ChatMessage>,
        tools: Option<Vec<ToolDefinition>>,
    ) -> anyhow::Result<ChatResponse> {
        let (mut anthropic_msgs, system_prompt) = convert_messages_to_anthropic(&messages);
        let anthropic_tools = tools.as_ref().map(|t| convert_tools_to_anthropic(t));

        // Cache the stable conversation prefix (system + tools + prior turns).
        anthropic::apply_conversation_cache_breakpoint(&mut anthropic_msgs);

        let request = anthropic::AnthropicRequest {
            model: self
                .provider
                .resolve_model_id(&self.settings.models.main.name),
            messages: anthropic_msgs,
            max_tokens: self
                .settings
                .models
                .transport
                .max_tokens
                .max(ANTHROPIC_MAX_TOKENS_FALLBACK),
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

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("{}", format_api_error(status, &body)));
        }

        let body_text = response
            .text()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to read Anthropic response body: {}", e))?;
        let anthropic_resp: anthropic::AnthropicResponse =
            serde_json::from_str(&body_text).map_err(|e| {
                let preview: String = body_text.chars().take(800).collect();
                tracing::error!(
                    ?e,
                    body_preview = %preview,
                    "Failed to deserialize Anthropic response body"
                );
                anyhow::anyhow!(
                    "error decoding Anthropic response body: {}\n--- response body (first 800 chars) ---\n{}",
                    e,
                    preview
                )
            })?;
        Ok(convert_anthropic_response(&anthropic_resp))
    }

    pub async fn chat_stream(
        &self,
        mut messages: Vec<ChatMessage>,
        tools: Option<Vec<ToolDefinition>>,
    ) -> anyhow::Result<reqwest::Response> {
        sanitize_tool_call_args_for_replay(&mut messages);
        strip_reasoning_content_for_replay(
            &mut messages,
            self.provider.as_ref(),
            self.settings.models.transport.strip_reasoning_content,
        );
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
            max_tokens: self.request_max_tokens(),
            stream: true,
            temperature: self.settings.models.main.temperature,
            tools,
            stream_options: Some(StreamOptions {
                include_usage: true,
            }),
        };

        let url = self.build_endpoint("chat/completions");

        let response = self
            .send_stream_with_retry(
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
            apply_conversation_cache_breakpoint, convert_messages_to_anthropic,
            convert_tools_to_anthropic, AnthropicRequest,
        };

        let (mut anthropic_msgs, system_prompt) = convert_messages_to_anthropic(&messages);
        let anthropic_tools = tools.as_ref().map(|t| convert_tools_to_anthropic(t));

        // Mark the end of the stable conversation prefix so Anthropic can cache
        // the entire history (system + tools + prior turns) and only process the
        // new user message fresh.
        apply_conversation_cache_breakpoint(&mut anthropic_msgs);

        let request = AnthropicRequest {
            model: self
                .provider
                .resolve_model_id(&self.settings.models.main.name),
            messages: anthropic_msgs,
            max_tokens: self
                .settings
                .models
                .transport
                .max_tokens
                .max(ANTHROPIC_MAX_TOKENS_FALLBACK),
            system: system_prompt,
            tools: anthropic_tools,
            stream: true,
        };

        let url = self.build_endpoint("messages");

        let response = self
            .send_stream_with_retry(
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

    fn response_with_retry_after(value: Option<&str>) -> reqwest::Response {
        let mut builder = http::Response::builder().status(429);
        if let Some(value) = value {
            builder = builder.header("Retry-After", value);
        }
        reqwest::Response::from(builder.body("rate limited").expect("static body"))
    }

    #[test]
    fn rate_limit_wait_none_for_success_or_other_errors() {
        let ok = reqwest::Response::from(
            http::Response::builder()
                .status(200)
                .body("")
                .expect("static body"),
        );
        assert_eq!(rate_limit_retry_wait(ok.status(), &ok), None);

        let server_error = reqwest::Response::from(
            http::Response::builder()
                .status(503)
                .body("")
                .expect("static body"),
        );
        assert_eq!(
            rate_limit_retry_wait(server_error.status(), &server_error),
            None
        );
    }

    #[test]
    fn rate_limit_wait_uses_default_backoff_without_header() {
        let response = response_with_retry_after(None);
        assert_eq!(
            rate_limit_retry_wait(response.status(), &response),
            Some(Duration::from_secs(RATE_LIMIT_BACKOFF_BASE_SECS))
        );
    }

    #[test]
    fn rate_limit_wait_honors_delay_seconds_header() {
        let response = response_with_retry_after(Some("7"));
        assert_eq!(
            rate_limit_retry_wait(response.status(), &response),
            Some(Duration::from_secs(7))
        );
    }

    #[test]
    fn rate_limit_wait_caps_server_supplied_delay() {
        let response = response_with_retry_after(Some("3600"));
        let wait =
            rate_limit_retry_wait(response.status(), &response).expect("429 must be retryable");
        assert!(wait <= Duration::from_secs(RATE_LIMIT_BACKOFF_MAX_SECS));
    }

    #[test]
    fn rate_limit_wait_retries_immediately_on_past_http_date() {
        // A date in the past means "the window already reset" -> wait zero.
        let response = response_with_retry_after(Some("Wed, 01 Jan 2020 00:00:00 GMT"));
        assert_eq!(
            rate_limit_retry_wait(response.status(), &response),
            Some(Duration::ZERO)
        );
    }

    #[test]
    fn rate_limit_wait_falls_back_when_header_unparseable() {
        let response = response_with_retry_after(Some("not-a-date-or-seconds"));
        assert_eq!(
            rate_limit_retry_wait(response.status(), &response),
            Some(Duration::from_secs(RATE_LIMIT_BACKOFF_BASE_SECS))
        );
    }

    fn asst_with_reasoning(reasoning: &str) -> ChatMessage {
        ChatMessage {
            role: "assistant".to_string(),
            content: Some("answer".to_string()),
            reasoning_content: Some(reasoning.to_string()),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    #[test]
    fn strip_reasoning_openai_compat_removes_field_from_request_json() {
        // GLM/z.ai and other OpenAI-compatible endpoints don't need the
        // reasoning echo; the serialized request must not carry the field.
        let mut msgs = vec![asst_with_reasoning("thinking...")];
        strip_reasoning_content_for_replay(&mut msgs, &provider::OpenAIProvider, None);
        assert_eq!(msgs[0].reasoning_content, None);
        let json = serde_json::to_string(&msgs[0]).unwrap();
        assert!(!json.contains(r#""reasoning_content""#), "got: {json}");
        // Content survives; only reasoning is stripped.
        assert_eq!(msgs[0].content.as_deref(), Some("answer"));
    }

    #[test]
    fn strip_reasoning_deepseek_keeps_echo_back() {
        // DeepSeek officially requires reasoning_content echo-back.
        let mut msgs = vec![asst_with_reasoning("thinking...")];
        strip_reasoning_content_for_replay(&mut msgs, &provider::DeepSeekProvider, None);
        assert_eq!(msgs[0].reasoning_content.as_deref(), Some("thinking..."));
    }

    #[test]
    fn strip_reasoning_settings_override_wins_over_provider_rule() {
        // Some(false) forces keeping even on a strip-by-default provider.
        let mut msgs = vec![asst_with_reasoning("thinking...")];
        strip_reasoning_content_for_replay(&mut msgs, &provider::OpenAIProvider, Some(false));
        assert_eq!(msgs[0].reasoning_content.as_deref(), Some("thinking..."));

        // Some(true) forces stripping even on DeepSeek.
        let mut msgs = vec![asst_with_reasoning("thinking...")];
        strip_reasoning_content_for_replay(&mut msgs, &provider::DeepSeekProvider, Some(true));
        assert_eq!(msgs[0].reasoning_content, None);
    }

    #[test]
    fn strip_reasoning_skips_anthropic_path() {
        // Anthropic requests go through convert_messages_to_anthropic, not
        // ChatMessage serde — the strip must not touch this path.
        let mut msgs = vec![asst_with_reasoning("thinking...")];
        strip_reasoning_content_for_replay(&mut msgs, &provider::AnthropicProvider, None);
        assert_eq!(msgs[0].reasoning_content.as_deref(), Some("thinking..."));
    }
}
