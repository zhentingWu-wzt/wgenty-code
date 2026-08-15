//! HTTP client for communicating with the daemon API.
//! Mirrors the TypeScript ApiClient in packages/core/src/client.ts.

use std::sync::Arc;

use crate::api::ChatMessage;
use anyhow::Context;
use futures::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use reqwest::{Method, Response, StatusCode};
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
#[error("run_session failed ({status:?}): {message}")]
pub(crate) struct RunSessionError {
    pub(crate) status: Option<StatusCode>,
    pub(crate) message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CancelRunOutcome {
    Cancelled,
    NotRunning,
}

/// Result of `POST /api/v1/tools/undo-turn` (code rollback of a turn range).
///
/// Mirrors `CheckpointStore::RewindRangeReport`. The TUI uses `restored` for
/// the `UndoReport` and `rewound_turns.is_empty()` to detect "nothing to roll
/// back" (`code_skipped`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UndoRangeResult {
    /// Files restored (written back or deleted) across all rewound turns.
    pub restored: usize,
    /// Files skipped (e.g. binary) across all rewound turns.
    pub skipped: usize,
    /// Paths that could not be restored.
    pub failed: Vec<String>,
    /// Turn ids actually rewound (newest-first), excluding skipped/missing.
    pub rewound_turns: Vec<String>,
}

/// One entry of `GET /api/v1/checkpoints` - a per-turn checkpoint snapshot's
/// metadata. Mirrors `CheckpointStore::TurnInfo`. The TUI uses `file_count` to
/// populate `TurnRecord::file_count` for the `/undo` turn-picker display.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CheckpointInfo {
    pub turn_id: String,
    pub created_at: String,
    pub file_count: usize,
}

/// One selectable model profile returned by `GET /api/v1/models`. Drives the
/// `/model` picker rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelOption {
    /// Profile key — what `POST /api/v1/model/switch` expects.
    pub key: String,
    /// Human-readable label (`display_name` or model name).
    pub label: String,
    /// Underlying model name (e.g. `claude-sonnet-4-5`).
    pub model_name: String,
    /// Forced provider, if any.
    pub provider: Option<String>,
    /// Declared tier (`"light"`/`"medium"`/`"heavy"`), if the profile set one.
    pub tier: Option<String>,
    /// Whether this is the currently active profile.
    pub active: bool,
}

/// Result of `POST /api/v1/model/switch`, used for the TUI confirmation toast.
#[derive(Debug, Clone, Default)]
pub struct ModelSwitchResult {
    pub label: String,
    pub model_name: String,
    pub provider: Option<String>,
}

#[derive(Clone)]
pub struct DaemonClient {
    /// Client for SSE streaming requests (no timeout — streams can run for minutes).
    http: Arc<std::sync::RwLock<Option<reqwest::Client>>>,
    /// Separate client for short-lived tool/API requests, avoiding connection-pool
    /// conflicts with the long-lived SSE streaming connection.
    http_tools: Arc<std::sync::RwLock<Option<reqwest::Client>>>,
    /// Client for long-running tools (`task`/`delegate`) whose subagents can run
    /// for many minutes. No timeout — the tool-level and subagent-level timeouts
    /// are the real ceilings, not the HTTP client (a 300s client timeout was
    /// killing legitimate subagent runs mid-flight).
    http_long: Arc<std::sync::RwLock<Option<reqwest::Client>>>,
    base_url: String,
    /// Bearer token for daemon auth, captured from the token file at
    /// construction and re-read from disk on 401 (the daemon generates a fresh
    /// token on every start, so a daemon restart invalidates the captured one).
    auth_token: Arc<std::sync::RwLock<Option<String>>>,
    /// Source of the current on-disk token. Injectable so tests can simulate
    /// a daemon restart (fresh token) without touching the real token file.
    token_reader: Arc<dyn Fn() -> Option<String> + Send + Sync>,
    /// Trusted-UI viewer bearer token, obtained once from `POST /api/v1/ui/viewers`
    /// and refreshed only after a daemon restart (401/404). Sent as the
    /// `X-Wgenty-Viewer-Token` header on scoped agent requests. Stored behind
    /// a `RwLock` so `create_viewer` can take `&self`.
    viewer_token: Arc<tokio::sync::RwLock<Option<String>>>,
}

impl std::fmt::Debug for DaemonClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print tokens; the client pools and viewer token carry no
        // useful debug state beyond the endpoint.
        f.debug_struct("DaemonClient")
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

impl DaemonClient {
    pub fn new(base_url: String) -> Self {
        Self::with_token_reader(base_url, Arc::new(crate::utils::read_daemon_token))
    }

    fn with_token_reader(
        base_url: String,
        token_reader: Arc<dyn Fn() -> Option<String> + Send + Sync>,
    ) -> Self {
        // Clients are created lazily on first use to avoid blocking the first
        // rendered frame. reqwest::Client::builder().build() initialises the
        // TLS backend and connection pool, which can take 200-300ms.
        let auth_token = token_reader();
        Self {
            http: Arc::new(std::sync::RwLock::new(None)),
            http_tools: Arc::new(std::sync::RwLock::new(None)),
            http_long: Arc::new(std::sync::RwLock::new(None)),
            base_url: base_url.trim_end_matches('/').to_string(),
            auth_token: Arc::new(std::sync::RwLock::new(auth_token)),
            token_reader,
            viewer_token: Arc::new(tokio::sync::RwLock::new(None)),
        }
    }

    /// Default headers (auth token) snapshot for one lazily-created client,
    /// built from the *current* token so a client rebuilt after
    /// [`refresh_auth_from_disk`](Self::refresh_auth_from_disk) picks up the
    /// new credentials.
    fn default_headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Some(token) = self
            .auth_token
            .read()
            .expect("auth token lock poisoned")
            .as_ref()
        {
            if let Ok(val) = HeaderValue::from_str(&format!("Bearer {}", token)) {
                headers.insert(AUTHORIZATION, val);
            }
        }
        headers
    }

    /// Return the cached client in `slot`, building it (with the current auth
    /// token) on first use. `reqwest::Client` clones share the underlying
    /// connection pool, so cloning per call is cheap.
    fn cached_or_build(
        slot: &std::sync::RwLock<Option<reqwest::Client>>,
        build: impl FnOnce() -> reqwest::Client,
    ) -> reqwest::Client {
        if let Some(client) = slot.read().expect("daemon client lock poisoned").as_ref() {
            return client.clone();
        }
        slot.write()
            .expect("daemon client lock poisoned")
            .get_or_insert_with(build)
            .clone()
    }

    /// Lazily create the SSE streaming client (no timeout).
    pub fn http(&self) -> reqwest::Client {
        Self::cached_or_build(&self.http, || {
            reqwest::Client::builder()
                .default_headers(self.default_headers())
                .build()
                .expect("reqwest client build")
        })
    }

    /// Lazily create the tool/API client (300s timeout, no idle pool).
    pub fn http_tools(&self) -> reqwest::Client {
        Self::cached_or_build(&self.http_tools, || {
            reqwest::Client::builder()
                .default_headers(self.default_headers())
                .timeout(std::time::Duration::from_secs(300))
                .pool_max_idle_per_host(0) // don't keep idle connections - always fresh
                .build()
                .expect("reqwest tools client build")
        })
    }

    /// Lazily create the long-running tool client (no timeout, no idle pool).
    pub fn http_long(&self) -> reqwest::Client {
        Self::cached_or_build(&self.http_long, || {
            reqwest::Client::builder()
                .default_headers(self.default_headers())
                .pool_max_idle_per_host(0)
                .build()
                .expect("reqwest long client build")
        })
    }

    /// Re-read the bearer token from the on-disk token file. When it changed
    /// (a restarted daemon generated a fresh token), drop the cached HTTP
    /// clients so they rebuild lazily with the new credentials. Returns true
    /// when the token actually changed.
    fn refresh_auth_from_disk(&self) -> bool {
        let new_token = (self.token_reader)();
        let changed = {
            let mut current = self.auth_token.write().expect("auth token lock poisoned");
            if *current == new_token {
                false
            } else {
                *current = new_token;
                true
            }
        };
        if changed {
            *self.http.write().expect("daemon client lock poisoned") = None;
            *self
                .http_tools
                .write()
                .expect("daemon client lock poisoned") = None;
            *self.http_long.write().expect("daemon client lock poisoned") = None;
            tracing::info!("daemon auth token changed on disk; rebuilt HTTP clients");
        }
        changed
    }

    /// Send a request, retrying once with a disk-refreshed bearer token when
    /// the daemon answers 401. A 401 from the localhost daemon almost always
    /// means the daemon was restarted and now expects the fresh token while
    /// this client still holds the one captured at construction.
    async fn send_with_auth_retry(
        &self,
        build: impl Fn(&Self) -> reqwest::RequestBuilder,
    ) -> anyhow::Result<Response> {
        let resp = build(self).send().await?;
        if resp.status() == StatusCode::UNAUTHORIZED && self.refresh_auth_from_disk() {
            return build(self)
                .send()
                .await
                .context("retry request with refreshed daemon token");
        }
        Ok(resp)
    }

    /// POST /api/v1/ui/viewers — obtain a trusted-UI viewer token.
    pub async fn create_viewer(&self) -> anyhow::Result<()> {
        let url = format!("{}/api/v1/ui/viewers", self.base_url);
        let resp = self
            .send_with_auth_retry(|c| c.http_tools().post(&url))
            .await
            .context("create trusted UI viewer")?;
        if !resp.status().is_success() {
            anyhow::bail!("Failed to create viewer ({})", resp.status());
        }
        let data: serde_json::Value = resp
            .json()
            .await
            .context("decode trusted UI viewer response")?;
        let token = data["viewer_token"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing viewer_token in response"))?
            .to_string();
        *self.viewer_token.write().await = Some(token);
        Ok(())
    }

    async fn ensure_viewer(&self) -> anyhow::Result<()> {
        if self.viewer_token.read().await.is_some() {
            return Ok(());
        }
        self.create_viewer()
            .await
            .context("create trusted UI viewer before scoped agent request")
    }

    async fn scoped_request(&self, method: Method, url: &str) -> anyhow::Result<Response> {
        let token = self
            .viewer_token
            .read()
            .await
            .clone()
            .ok_or_else(|| anyhow::anyhow!("trusted UI viewer token is unavailable"))?;
        self.http_tools()
            .request(method, url)
            .header("X-Wgenty-Viewer-Token", token)
            .send()
            .await
            .context("send capability-scoped agent request")
    }

    async fn send_scoped_request(&self, method: Method, url: &str) -> anyhow::Result<Response> {
        self.ensure_viewer().await?;
        let response = self.scoped_request(method.clone(), url).await?;
        if matches!(
            response.status(),
            StatusCode::UNAUTHORIZED | StatusCode::NOT_FOUND
        ) {
            self.create_viewer()
                .await
                .context("refresh trusted UI viewer after scoped request rejection")?;
            return self.scoped_request(method, url).await;
        }
        Ok(response)
    }

    /// GET /api/v1/agents/self — root local view for `session_id`.
    pub async fn get_root_agent_view(
        &self,
        session_id: &str,
    ) -> anyhow::Result<crate::daemon::models::LocalAgentViewResponse> {
        let url = format!(
            "{}/api/v1/agents/self?session_id={}",
            self.base_url, session_id
        );
        let resp = self.send_scoped_request(Method::GET, &url).await?;
        if !resp.status().is_success() {
            anyhow::bail!("get_root_agent_view ({})", resp.status());
        }
        resp.json()
            .await
            .context("decode root agent local view response")
    }

    /// GET /api/v1/agents/children/:capability — navigate to the bound target.
    pub async fn navigate_agent_view(
        &self,
        session_id: &str,
        capability: &str,
    ) -> anyhow::Result<crate::daemon::models::LocalAgentViewResponse> {
        let url = format!(
            "{}/api/v1/agents/children/{}?session_id={}",
            self.base_url, capability, session_id
        );
        let resp = self.send_scoped_request(Method::GET, &url).await?;
        if !resp.status().is_success() {
            anyhow::bail!("navigate_agent_view ({})", resp.status());
        }
        resp.json()
            .await
            .context("decode child agent local view response")
    }

    /// GET /api/v1/agents/children/:capability/transcript
    pub async fn get_child_transcript(
        &self,
        session_id: &str,
        capability: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let url = format!(
            "{}/api/v1/agents/children/{}/transcript?session_id={}",
            self.base_url, capability, session_id
        );
        let resp = self.send_scoped_request(Method::GET, &url).await?;
        if !resp.status().is_success() {
            anyhow::bail!("get_child_transcript ({})", resp.status());
        }
        resp.json()
            .await
            .context("decode child agent transcript response")
    }

    /// POST /api/v1/agents/children/:capability/cancel
    pub async fn cancel_child(&self, session_id: &str, capability: &str) -> anyhow::Result<()> {
        let url = format!(
            "{}/api/v1/agents/children/{}/cancel?session_id={}",
            self.base_url, capability, session_id
        );
        let resp = self.send_scoped_request(Method::POST, &url).await?;
        if !resp.status().is_success() {
            anyhow::bail!("cancel_child ({})", resp.status());
        }
        Ok(())
    }

    /// `POST /api/v1/agents/task-groups/claim` -- atomically claim one ready
    /// root-direct task group. Returns `Ok(Some(delivery))` when a ready group
    /// was claimed, or `Ok(None)` when nothing is ready (HTTP 204).
    pub async fn claim_task_group(
        &self,
        session_id: &str,
        generation: u64,
    ) -> anyhow::Result<Option<TaskGroupDeliveryResponse>> {
        let url = format!("{}/api/v1/agents/task-groups/claim", self.base_url);
        let body = serde_json::json!({
            "session_id": session_id,
            "generation": generation,
        });
        let resp = self
            .http_tools()
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .context("claim ready task group")?;
        if resp.status() == StatusCode::NO_CONTENT {
            return Ok(None);
        }
        if !resp.status().is_success() {
            anyhow::bail!("claim_task_group ({})", resp.status());
        }
        Ok(Some(
            resp.json().await.context("decode task group delivery")?,
        ))
    }

    /// `POST /api/v1/agents/generation/reset` -- advance the session generation
    /// and cancel obsolete root-direct subtrees. Returns the new generation.
    pub async fn reset_agent_generation(&self, session_id: &str) -> anyhow::Result<u64> {
        let url = format!("{}/api/v1/agents/generation/reset", self.base_url);
        let body = serde_json::json!({ "session_id": session_id });
        let resp = self
            .http_tools()
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .context("reset agent generation")?;
        if !resp.status().is_success() {
            anyhow::bail!("reset_agent_generation ({})", resp.status());
        }
        #[derive(serde::Deserialize)]
        struct ResetResponse {
            generation: u64,
        }
        let parsed: ResetResponse = resp.json().await.context("decode reset generation")?;
        Ok(parsed.generation)
    }

    /// `POST /api/v1/agents/session/cancel` -- cancel the entire agent session
    /// on shutdown. Cancels live root-direct subtrees bottom-up and releases
    /// every permit.
    pub async fn cancel_agent_session(&self, session_id: &str) -> anyhow::Result<()> {
        let url = format!("{}/api/v1/agents/session/cancel", self.base_url);
        let body = serde_json::json!({ "session_id": session_id });
        let resp = self
            .http_tools()
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .context("cancel agent session")?;
        if !resp.status().is_success() {
            anyhow::bail!("cancel_agent_session ({})", resp.status());
        }
        Ok(())
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Check daemon health. Returns the health response.
    pub async fn health(&self) -> anyhow::Result<HealthResponse> {
        let url = format!("{}/api/v1/health", self.base_url);
        let resp = self.http().get(&url).send().await?;
        Ok(resp.json().await?)
    }

    /// Get daemon config.
    pub async fn get_config(&self) -> anyhow::Result<ConfigResponse> {
        let url = format!("{}/api/v1/config", self.base_url);
        let resp = self.http().get(&url).send().await?;
        Ok(resp.json().await?)
    }

    /// POST /api/v1/chat/stream — returns the raw SSE response stream.
    pub async fn chat_stream(
        &self,
        messages: Vec<ChatMessage>,
        max_tokens: Option<usize>,
    ) -> anyhow::Result<reqwest::Response> {
        self.chat_stream_with_plan(messages, max_tokens, None).await
    }

    /// Chat stream with optional plan_mode flag.
    pub async fn chat_stream_with_plan(
        &self,
        messages: Vec<ChatMessage>,
        max_tokens: Option<usize>,
        plan_mode: Option<bool>,
    ) -> anyhow::Result<reqwest::Response> {
        let url = format!("{}/api/v1/chat/stream", self.base_url);
        let body = ChatStreamRequest {
            messages,
            model: None,
            max_tokens,
            plan_mode,
        };
        let resp = self
            .send_with_auth_retry(|c| {
                c.http()
                    .post(&url)
                    .header("Content-Type", "application/json")
                    .json(&body)
            })
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("API error ({}): {}", status, text);
        }
        Ok(resp)
    }

    /// POST /api/v1/sessions/:id/run - start a server-side agent turn.
    /// The daemon owns the loop (LLM calls + tool execution + persistence);
    /// subscribe to events via [`session_events`](Self::session_events) to
    /// observe progress. Returns the run_id.
    pub async fn run_session(
        &self,
        session_id: &str,
        message: &str,
        plan_mode: bool,
    ) -> anyhow::Result<String> {
        self.try_run_session(session_id, message, plan_mode)
            .await
            .map_err(anyhow::Error::from)
    }

    pub(crate) async fn try_run_session(
        &self,
        session_id: &str,
        message: &str,
        plan_mode: bool,
    ) -> Result<String, RunSessionError> {
        let encoded = urlencode(session_id);
        let url = format!("{}/api/v1/sessions/{}/run", self.base_url, encoded);
        let resp = self
            .send_with_auth_retry(|c| {
                c.http_tools()
                    .post(&url)
                    .header("Content-Type", "application/json")
                    .json(&serde_json::json!({ "message": message, "plan_mode": plan_mode }))
            })
            .await
            .map_err(|error| RunSessionError {
                status: None,
                message: error.to_string(),
            })?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(RunSessionError {
                status: Some(status),
                message: text,
            });
        }
        let status = resp.status();
        let body: serde_json::Value = resp.json().await.map_err(|error| RunSessionError {
            status: Some(status),
            message: error.to_string(),
        })?;
        Ok(body["run_id"].as_str().unwrap_or("").to_string())
    }

    /// GET /api/v1/sessions/:id/events - SSE stream of `SessionEvent`.
    /// Long-lived connection; the daemon pushes events as the server-side
    /// run progresses. Returns the raw response for the caller to read.
    pub async fn session_events(&self, session_id: &str) -> anyhow::Result<reqwest::Response> {
        self.session_events_after(session_id, None).await
    }

    pub(crate) async fn session_events_after(
        &self,
        session_id: &str,
        after: Option<u64>,
    ) -> anyhow::Result<reqwest::Response> {
        let encoded = urlencode(session_id);
        let url = format!("{}/api/v1/sessions/{}/events", self.base_url, encoded);
        let resp = self
            .send_with_auth_retry(|c| {
                let mut request = c.http().get(&url);
                if let Some(after) = after {
                    request = request.query(&[("after", after)]);
                }
                request
            })
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("session_events failed ({}): {}", status, text);
        }
        Ok(resp)
    }

    /// GET /api/v1/subagents/trace/stream?session_id=... - SSE stream of
    /// TraceEvent (subagent progress + permission/question lifecycle).
    pub async fn trace_stream(&self, session_id: &str) -> anyhow::Result<reqwest::Response> {
        let encoded = urlencode(session_id);
        let url = format!(
            "{}/api/v1/subagents/trace/stream?session_id={}",
            self.base_url, encoded
        );
        let resp = self.send_with_auth_retry(|c| c.http().get(&url)).await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("trace_stream failed ({}): {}", status, text);
        }
        Ok(resp)
    }

    /// POST /api/v1/sessions/:id/cancel - cancel the active server-side run.
    pub async fn cancel_run(&self, session_id: &str) -> anyhow::Result<()> {
        self.try_cancel_run(session_id).await.map(|_| ())
    }

    pub(crate) async fn try_cancel_run(
        &self,
        session_id: &str,
    ) -> anyhow::Result<CancelRunOutcome> {
        let encoded = urlencode(session_id);
        let url = format!("{}/api/v1/sessions/{}/cancel", self.base_url, encoded);
        let resp = self
            .send_with_auth_retry(|c| c.http_tools().post(&url))
            .await?;
        if resp.status().is_success() {
            return Ok(CancelRunOutcome::Cancelled);
        }
        if resp.status() == StatusCode::NOT_FOUND {
            return Ok(CancelRunOutcome::NotRunning);
        }
        if !resp.status().is_success() {
            anyhow::bail!("cancel_run failed ({})", resp.status());
        }
        unreachable!("success and not-found statuses returned above")
    }

    /// Signal cancellation and wait until the daemon releases the session run
    /// claim. The cancel endpoint returns 204 when it only signalled the token;
    /// 404 is the authoritative idle state after the run's final save.
    pub(crate) async fn cancel_run_and_wait_for_release(
        &self,
        session_id: &str,
        timeout: std::time::Duration,
    ) -> anyhow::Result<()> {
        tokio::time::timeout(timeout, async {
            loop {
                match self
                    .try_cancel_run(session_id)
                    .await
                    .with_context(|| format!("cancel daemon session {session_id}"))?
                {
                    CancelRunOutcome::NotRunning => return Ok(()),
                    CancelRunOutcome::Cancelled => {
                        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                    }
                }
            }
        })
        .await
        .context("timed out waiting for daemon session run to release")?
    }

    /// POST /api/v1/interactions/:id/resolve - answer a pending ask_user_question.
    /// `answer` is a JSON string `{"selected":[...],"text":"..."}`.
    pub async fn resolve_interaction(&self, request_id: &str, answer: &str) -> anyhow::Result<()> {
        let encoded = urlencode(request_id);
        let url = format!("{}/api/v1/interactions/{}/resolve", self.base_url, encoded);
        let resp = self
            .http_tools()
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({ "answer": answer }))
            .send()
            .await?;
        if !resp.status().is_success() {
            anyhow::bail!("resolve_interaction failed ({})", resp.status());
        }
        Ok(())
    }

    /// POST /api/v1/tools/execute
    pub async fn execute_tool(
        &self,
        tool_name: &str,
        arguments: serde_json::Value,
        session_id: &str,
        turn_id: Option<&str>,
    ) -> anyhow::Result<ExecuteToolResponse> {
        let url = format!("{}/api/v1/tools/execute", self.base_url);
        let body = ExecuteToolRequest {
            tool_name: tool_name.to_string(),
            arguments,
            session_id: Some(session_id.to_string()),
            turn_id: turn_id.map(|t| t.to_string()),
        };
        // task/delegate run subagents that can take many minutes. Use the
        // no-timeout client so the HTTP request isn't killed at 300s while
        // the subagent is still running on the daemon.
        let client = if tool_name == "task" || tool_name == "delegate" {
            self.http_long()
        } else {
            self.http_tools()
        };
        let resp = client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            anyhow::bail!("Tool execution failed ({})", resp.status());
        }
        Ok(resp.json().await?)
    }

    /// POST /api/v1/tools/approve
    pub async fn approve_tool(&self, session_rule: &str) -> anyhow::Result<()> {
        let url = format!("{}/api/v1/tools/approve", self.base_url);
        self.http_tools()
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({"session_rule": session_rule}))
            .send()
            .await?;
        Ok(())
    }

    /// POST /api/v1/tools/unapprove
    pub async fn unapprove_tool(&self, session_rule: &str) -> anyhow::Result<()> {
        let url = format!("{}/api/v1/tools/unapprove", self.base_url);
        self.http_tools()
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({"session_rule": session_rule}))
            .send()
            .await?;
        Ok(())
    }

    /// GET /api/v1/tools/pending-permissions
    pub async fn list_pending_permissions(&self) -> anyhow::Result<Vec<PendingSubagentPermission>> {
        let url = format!("{}/api/v1/tools/pending-permissions", self.base_url);
        let resp = self.http_tools().get(&url).send().await?;
        if !resp.status().is_success() {
            return Ok(Vec::new());
        }
        let body: ListPendingPermissionsResponse = resp.json().await?;
        Ok(body.pending)
    }

    /// POST /api/v1/tools/resolve-permission
    pub async fn resolve_subagent_permission(
        &self,
        request_id: &str,
        approved: bool,
        always: bool,
        session_rule: Option<&str>,
    ) -> anyhow::Result<bool> {
        let url = format!("{}/api/v1/tools/resolve-permission", self.base_url);
        let resp = self
            .http_tools()
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "request_id": request_id,
                "approved": approved,
                "always": always,
                "session_rule": session_rule,
            }))
            .send()
            .await?;
        if !resp.status().is_success() {
            anyhow::bail!("resolve-permission failed ({})", resp.status());
        }
        let v: serde_json::Value = resp.json().await?;
        Ok(v.get("resolved").and_then(|x| x.as_bool()).unwrap_or(false))
    }

    /// POST /api/v1/permission-mode - update root agent runtime permission mode
    /// and sandbox effective mode (Plan included).
    pub async fn set_permission_mode(
        &self,
        session_id: &str,
        mode: crate::config::agent::RootPermissionMode,
        effective_mode: crate::sandbox::EffectiveMode,
    ) -> anyhow::Result<()> {
        let url = format!("{}/api/v1/permission-mode", self.base_url);
        let resp = self
            .http_tools()
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "mode": mode,
                "effective_mode": effective_mode,
                "session_id": session_id,
            }))
            .send()
            .await?;
        if !resp.status().is_success() {
            anyhow::bail!("set-permission-mode failed ({})", resp.status());
        }
        Ok(())
    }

    /// GET /api/v1/permission-mode - fetch current root agent permission mode.
    pub async fn get_permission_mode(
        &self,
        session_id: &str,
    ) -> anyhow::Result<crate::config::agent::RootPermissionMode> {
        let url = format!("{}/api/v1/permission-mode", self.base_url);
        let resp = self
            .http_tools()
            .get(&url)
            .query(&[("session_id", session_id)])
            .send()
            .await?;
        if !resp.status().is_success() {
            anyhow::bail!("get-permission-mode failed ({})", resp.status());
        }
        let v: serde_json::Value = resp.json().await?;
        Ok(serde_json::from_value(
            v.get("mode").cloned().unwrap_or(serde_json::Value::Null),
        )?)
    }

    /// GET /api/v1/models - list switchable model profiles for the `/model`
    /// picker. Returns `(key, label, model_name, provider, active)` tuples in
    /// alphabetical order by key.
    pub async fn list_models(&self) -> anyhow::Result<Vec<ModelOption>> {
        let url = format!("{}/api/v1/models", self.base_url);
        let resp = self.http_tools().get(&url).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("list-models failed ({})", resp.status());
        }
        let v: serde_json::Value = resp.json().await?;
        let arr = v
            .get("profiles")
            .and_then(|x| x.as_array())
            .ok_or_else(|| anyhow::anyhow!("list-models: malformed response"))?;
        Ok(arr
            .iter()
            .map(|p| ModelOption {
                key: p
                    .get("key")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                label: p
                    .get("label")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                model_name: p
                    .get("model_name")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                provider: p
                    .get("provider")
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_string()),
                tier: p
                    .get("tier")
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_string()),
                active: p.get("active").and_then(|x| x.as_bool()).unwrap_or(false),
            })
            .collect())
    }

    /// POST /api/v1/model/switch - activate a named profile. On success the
    /// next chat turn uses the new model. Returns the daemon's response
    /// (label/model_name/provider) for UI confirmation.
    pub async fn switch_model(&self, profile: &str) -> anyhow::Result<ModelSwitchResult> {
        let url = format!("{}/api/v1/model/switch", self.base_url);
        let resp = self
            .http_tools()
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({ "profile": profile }))
            .send()
            .await?;
        if !resp.status().is_success() {
            // Surface the daemon's actionable error (lists available profiles).
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("{body}");
        }
        let v: serde_json::Value = resp.json().await?;
        Ok(ModelSwitchResult {
            label: v
                .get("label")
                .and_then(|x| x.as_str())
                .unwrap_or(profile)
                .to_string(),
            model_name: v
                .get("model_name")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            provider: v
                .get("provider")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string()),
        })
    }

    /// GET /api/v1/undo — undo most recent checkpoint
    pub async fn undo(&self) -> anyhow::Result<String> {
        let url = format!("{}/api/v1/tools/undo", self.base_url);
        let resp = self.http().get(&url).send().await?;
        Ok(resp.text().await?)
    }

    /// POST /api/v1/tools/undo-turn - rewind a range of turns (oldest-first)
    /// in reverse, restoring files to the state before the oldest turn. Backs
    /// the TUI `/undo` code-rollback flow. Returns the aggregated result;
    /// `rewound_turns.is_empty()` signals "no checkpointed turn to roll back".
    pub async fn undo_turn_range(&self, turn_ids: &[String]) -> anyhow::Result<UndoRangeResult> {
        let url = format!("{}/api/v1/tools/undo-turn", self.base_url);
        let resp = self
            .http_tools()
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({ "turn_ids": turn_ids }))
            .send()
            .await?;
        if !resp.status().is_success() {
            anyhow::bail!("undo-turn failed ({})", resp.status());
        }
        let v: serde_json::Value = resp.json().await?;
        Ok(UndoRangeResult {
            restored: v["restored"].as_u64().unwrap_or(0) as usize,
            skipped: v["skipped"].as_u64().unwrap_or(0) as usize,
            failed: v["failed"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
            rewound_turns: v["rewound_turns"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
        })
    }

    /// GET /api/v1/checkpoints - list per-turn checkpoint snapshots
    /// (newest-first). Each entry carries the turn id, creation timestamp, and
    /// file count, used by the TUI `/undo` turn-picker to show how many files
    /// each turn touched. Failures return an empty vec so the picker keeps
    /// rendering with `file_count = 0`.
    pub async fn list_checkpoints(&self) -> anyhow::Result<Vec<CheckpointInfo>> {
        let url = format!("{}/api/v1/checkpoints", self.base_url);
        let resp = self.http_tools().get(&url).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("list checkpoints failed ({})", resp.status());
        }
        let v: serde_json::Value = resp.json().await?;
        let arr = v.as_array().cloned().unwrap_or_default();
        Ok(arr
            .into_iter()
            .map(|e| CheckpointInfo {
                turn_id: e["turn_id"].as_str().unwrap_or("").to_string(),
                created_at: e["created_at"].as_str().unwrap_or("").to_string(),
                file_count: e["file_count"].as_u64().unwrap_or(0) as usize,
            })
            .collect())
    }

    /// GET /api/v1/background/results
    pub async fn get_background_results(&self) -> anyhow::Result<Vec<serde_json::Value>> {
        let url = format!("{}/api/v1/background/results", self.base_url);
        let resp = self.http_tools().get(&url).send().await?;
        if !resp.status().is_success() {
            return Ok(Vec::new());
        }
        let data: serde_json::Value = resp.json().await?;
        Ok(data["results"].as_array().cloned().unwrap_or_default())
    }

    /// GET /api/v1/sessions
    pub async fn list_sessions(&self) -> anyhow::Result<Vec<SessionInfo>> {
        let url = format!("{}/api/v1/sessions", self.base_url);
        let resp = self.http_tools().get(&url).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("Failed to list sessions ({})", resp.status());
        }
        Ok(resp.json().await?)
    }

    /// POST /api/v1/sessions
    pub async fn create_session(&self, name: Option<&str>) -> anyhow::Result<SessionResponse> {
        let url = format!("{}/api/v1/sessions", self.base_url);
        let resp = self
            .http_tools()
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({"name": name}))
            .send()
            .await?;
        if !resp.status().is_success() {
            anyhow::bail!("Failed to create session ({})", resp.status());
        }
        Ok(resp.json().await?)
    }

    /// GET /api/v1/sessions/:id
    pub async fn load_session(&self, id: &str) -> anyhow::Result<SessionResponse> {
        let encoded = urlencode(id);
        let url = format!("{}/api/v1/sessions/{}", self.base_url, encoded);
        let resp = self
            .http_tools()
            .get(&url)
            .send()
            .await
            .with_context(|| format!("check whether daemon session {id} exists"))?;
        if !resp.status().is_success() {
            anyhow::bail!("Failed to load session ({})", resp.status());
        }
        Ok(resp.json().await?)
    }

    /// Return whether the daemon already persists this session id.
    pub(crate) async fn session_exists(&self, id: &str) -> anyhow::Result<bool> {
        let encoded = urlencode(id);
        let url = format!("{}/api/v1/sessions/{}", self.base_url, encoded);
        let resp = self.http_tools().get(&url).send().await?;
        if resp.status().is_success() {
            return Ok(true);
        }
        if resp.status() == StatusCode::NOT_FOUND {
            return Ok(false);
        }
        anyhow::bail!("session existence check failed ({})", resp.status())
    }

    /// PUT /api/v1/sessions/:id
    /// Retries up to 2 additional times on network errors or 5xx responses
    /// with exponential backoff (1s, 2s, 4s).
    pub async fn save_session(
        &self,
        id: &str,
        name: &str,
        messages: &[ChatMessage],
        ui_messages: &[crate::context::SessionUiMessage],
    ) -> anyhow::Result<()> {
        const MAX_RETRIES: u32 = 3;
        let encoded = urlencode(id);
        let url = format!("{}/api/v1/sessions/{}", self.base_url, encoded);
        let body = serde_json::json!({
            "name": name,
            "messages": messages,
            "ui_messages": ui_messages,
        });
        let mut last_err: Option<anyhow::Error> = None;
        for attempt in 0..MAX_RETRIES {
            if attempt > 0 {
                // Exponential backoff: 1s, 2s, 4s
                tokio::time::sleep(std::time::Duration::from_secs(
                    1u64 << attempt.saturating_sub(1),
                ))
                .await;
            }
            match self
                .http_tools()
                .put(&url)
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => {
                    tracing::debug!("save_session succeeded on attempt {}", attempt + 1);
                    return Ok(());
                }
                Ok(resp) => {
                    let status = resp.status();
                    last_err = Some(anyhow::anyhow!("Failed to save session ({})", status));
                    // 401 = daemon restarted with a fresh token; re-read it
                    // from disk and retry instead of dropping the session save.
                    if status == StatusCode::UNAUTHORIZED && self.refresh_auth_from_disk() {
                        tracing::warn!("save_session got 401; refreshed daemon token, retrying");
                        continue;
                    }
                    if status.is_server_error() {
                        tracing::warn!(
                            "save_session attempt {}/{} failed with {}: will retry",
                            attempt + 1,
                            MAX_RETRIES,
                            status
                        );
                        continue;
                    }
                    // Client errors (4xx) are not retriable.
                    tracing::error!(
                        "save_session failed with client error {}: not retrying",
                        status
                    );
                    break;
                }
                Err(e) => {
                    tracing::warn!(
                        "save_session attempt {}/{} network error: {e}: will retry",
                        attempt + 1,
                        MAX_RETRIES
                    );
                    last_err = Some(anyhow::anyhow!("Save session network error: {e}"));
                    continue;
                }
            }
        }
        Err(last_err
            .unwrap_or_else(|| anyhow::anyhow!("save_session failed after {MAX_RETRIES} attempts")))
    }

    /// DELETE /api/v1/sessions/:id
    pub async fn delete_session(&self, id: &str) -> anyhow::Result<()> {
        let encoded = urlencode(id);
        let url = format!("{}/api/v1/sessions/{}", self.base_url, encoded);
        let resp = self.http_tools().delete(&url).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("Failed to delete session ({})", resp.status());
        }
        Ok(())
    }

    /// GET /api/v1/sessions/search?q=...
    pub async fn search_sessions(&self, query: &str) -> anyhow::Result<Vec<SessionInfo>> {
        let encoded = urlencode(query);
        let url = format!("{}/api/v1/sessions/search?q={}", self.base_url, encoded);
        let resp = self.http_tools().get(&url).send().await?;
        if !resp.status().is_success() {
            return Ok(Vec::new());
        }
        Ok(resp.json().await?)
    }

    /// GET /api/v1/todos
    pub async fn get_todos(&self) -> anyhow::Result<TodoResponse> {
        let url = format!("{}/api/v1/todos", self.base_url);
        let resp = self.http_tools().get(&url).send().await?;
        Ok(resp.json().await?)
    }

    /// GET /api/v1/events — daemon global event stream (SSE, live-only).
    ///
    /// Single attempt: connect/parse errors surface as `Err` and the caller
    /// owns reconnect/fallback. Each item is one parsed [`GlobalEventWire`];
    /// a stream item error means the connection dropped mid-stream.
    pub async fn subscribe_events(
        &self,
    ) -> anyhow::Result<impl futures::Stream<Item = anyhow::Result<GlobalEventWire>>> {
        let url = format!("{}/api/v1/events", self.base_url);
        let resp = self
            .send_with_auth_retry(|c| c.http().get(&url))
            .await
            .context("connect global event stream")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("subscribe_events failed ({}): {}", status, text);
        }
        Ok(sse_data_lines(resp).map(|line| {
            line.and_then(|payload| {
                serde_json::from_str::<GlobalEventWire>(&payload).context("parse global event")
            })
        }))
    }

    /// GET /api/v1/tasks/progress - ready/blocked counts for agent nudges.
    pub async fn task_progress(&self) -> anyhow::Result<TaskProgressResponse> {
        let url = format!("{}/api/v1/tasks/progress", self.base_url);
        let resp = self.http_tools().get(&url).send().await?;
        Ok(resp.json().await?)
    }

    /// GET /api/v1/client/heartbeat — public SSE keepalive endpoint. The
    /// daemon counts the TUI as a connected thin client for as long as the
    /// response stream stays open.
    async fn connect_heartbeat(&self) -> Result<Response, reqwest::Error> {
        let url = format!("{}/api/v1/client/heartbeat", self.base_url);
        self.http().get(&url).send().await
    }

    /// Spawn a background task holding the heartbeat connection for the
    /// process lifetime. A running TUI counts as a thin client, so the
    /// daemon's idle shutdown stays suspended even when the TUI sits idle at
    /// the prompt (no API requests); when the TUI exits, the connection
    /// drops and the daemon's idle timer starts. Reconnects with exponential
    /// backoff (1s → 30s) so a daemon restart doesn't permanently detach the
    /// TUI's presence. Detached: the task dies with the process.
    pub fn spawn_heartbeat_keeper(&self) {
        let client = self.clone();
        tokio::spawn(async move {
            let mut backoff = std::time::Duration::from_secs(1);
            loop {
                if let Ok(resp) = client.connect_heartbeat().await {
                    backoff = std::time::Duration::from_secs(1);
                    // Hold the connection open, discarding keepalives, until
                    // the server closes it or the connection drops.
                    let mut stream = resp.bytes_stream();
                    while let Some(Ok(_)) = futures::StreamExt::next(&mut stream).await {}
                }
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(std::time::Duration::from_secs(30));
            }
        });
    }
}

/// `GET /api/v1/tasks/progress` response (mirrors daemon model).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct TaskProgressResponse {
    pub blocked: usize,
    pub ready: usize,
}

/// Wire shape of one daemon global event (`GET /api/v1/events`). TUI-local
/// mirror of the daemon's `GlobalEvent` envelope — `kind` stays a plain
/// string so unknown kinds forward-compatibly deserialize.
#[derive(Debug, Clone, Deserialize)]
pub struct GlobalEventWire {
    pub seq: u64,
    pub kind: String,
    pub data: serde_json::Value,
}

/// Split an SSE response body into `data:` payload lines.
///
/// This is THE single SSE line parser for the TUI: the session-event reader,
/// the subagent trace reader, and the global-events subscription all consume
/// it (previously the buffering/`strip_prefix("data: ")` loop was inlined in
/// each reader). Non-`data:` lines (keep-alive comments, event:/id: fields)
/// are skipped. A chunk read error terminates the stream after yielding one
/// `Err`; a clean close yields `None`.
pub(crate) fn sse_data_lines(
    resp: Response,
) -> impl futures::Stream<Item = anyhow::Result<String>> {
    let stream = resp.bytes_stream();
    futures::stream::try_unfold(
        (stream, String::new()),
        |(mut stream, mut buf)| async move {
            loop {
                if let Some(pos) = buf.find('\n') {
                    let line = buf[..pos].trim().to_string();
                    buf.drain(..pos + 1);
                    if let Some(payload) = line.strip_prefix("data: ") {
                        return Ok(Some((payload.to_string(), (stream, buf))));
                    }
                    // Comment/keep-alive or other SSE field — keep scanning.
                    continue;
                }
                match stream.next().await {
                    Some(Ok(chunk)) => buf.push_str(&String::from_utf8_lossy(&chunk)),
                    Some(Err(e)) => return Err(anyhow::Error::new(e).context("SSE stream read")),
                    None => return Ok(None),
                }
            }
        },
    )
}

/// Simple percent-encode for URL path segments (only encode truly unsafe chars).
fn urlencode(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            }
            _ => {
                result.push_str(&format!("%{:02X}", byte));
            }
        }
    }
    result
}

// ── Request types ─────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct ChatStreamRequest {
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    plan_mode: Option<bool>,
}

#[derive(Debug, Serialize)]
struct ExecuteToolRequest {
    tool_name: String,
    arguments: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    turn_id: Option<String>,
}

// ── Response types ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
}

#[derive(Debug, Deserialize)]
pub struct ConfigResponse {
    pub model: String,
    pub api_base: String,
    pub max_tokens: usize,
    pub timeout: u64,
    pub streaming: bool,
}

/// One delivered task-group batch (mirrors the daemon response). Used by the
/// continuation scheduler to inject completed subagent results into the main
/// agent turn.
#[derive(Debug, Clone, Deserialize)]
pub struct TaskGroupDeliveryResponse {
    pub group_id: String,
    pub generation: u64,
    pub results: Vec<crate::agent::ChildResult>,
}

#[derive(Debug, Deserialize)]
pub struct ExecuteToolResponse {
    pub success: bool,
    pub output_type: Option<String>,
    pub content: Option<String>,
    pub error: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub permission_required: Option<PermissionRequiredInfo>,
}

#[derive(Debug, Deserialize)]
pub struct PermissionRequiredInfo {
    /// Canonical tool name for AcceptEdits matching (not the session_rule).
    #[serde(default)]
    pub tool_name: String,
    pub reason: String,
    pub session_rule: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PendingSubagentPermission {
    pub request_id: String,
    pub from: String,
    pub kind: String,
    pub tool: String,
    pub policy_reason: String,
    pub session_rule: String,
    pub human_summary: String,
}

#[derive(Debug, Deserialize)]
struct ListPendingPermissionsResponse {
    pending: Vec<PendingSubagentPermission>,
}

#[derive(Debug, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub updated_at: String,
    pub message_count: usize,
    pub summary: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SessionResponse {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub updated_at: String,
    pub messages: Vec<ChatMessage>,
    /// Human-facing TUI transcript; empty/absent for legacy sessions.
    #[serde(default)]
    pub ui_messages: Vec<crate::context::SessionUiMessage>,
}

/// Metadata for subagent tasks in the TUI layer.
/// Mirrors `tasks::SubagentTodoMeta` — communicates via JSON serialization.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SubagentTodoMeta {
    pub subagent_type: String,
    pub token_usage: u64,
    pub rounds: u32,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct TodoItem {
    pub content: String,
    pub status: String,
    // Wire compat: `GET /todos` (TodoItemResponse) serializes `active_form`,
    // while `todos_changed` SSE events embed `tasks::TodoItem` which renames
    // the field to `activeForm`. Accept both; `default` keeps the field
    // optional on either shape.
    #[serde(default, alias = "activeForm")]
    pub active_form: String,
    #[serde(default)]
    pub subagent: Option<SubagentTodoMeta>,
}

#[derive(Debug, Deserialize)]
pub struct TodoResponse {
    pub items: Vec<TodoItem>,
    pub has_open_items: bool,
    pub display: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State;
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::RwLock;
    use tokio::task::JoinHandle;

    const VIEWER_HEADER: &str = "X-Wgenty-Viewer-Token";

    /// Wire-format regression: the daemon's `tasks::TodoItem` serializes
    /// `active_form` as `activeForm` (serde rename), which previously was
    /// silently swallowed by `#[serde(default)]` here. Both the SSE
    /// (`activeForm`) and the GET /todos (`active_form`) shapes must parse.
    #[test]
    fn todo_item_deserializes_active_form_from_both_wire_shapes() {
        let sse_shape: TodoItem = serde_json::from_str(
            r#"{"content":"fix bug","status":"in_progress","activeForm":"Fixing the bug"}"#,
        )
        .expect("activeForm (SSE todos_changed) shape parses");
        assert_eq!(sse_shape.active_form, "Fixing the bug");

        let get_shape: TodoItem = serde_json::from_str(
            r#"{"content":"fix bug","status":"in_progress","active_form":"Fixing the bug"}"#,
        )
        .expect("active_form (GET /todos) shape parses");
        assert_eq!(get_shape.active_form, "Fixing the bug");

        let absent: TodoItem =
            serde_json::from_str(r#"{"content":"fix bug","status":"pending"}"#).expect("optional");
        assert_eq!(absent.active_form, "");
    }

    #[derive(Clone)]
    struct ScopedServerState {
        viewer_creations: Arc<AtomicUsize>,
        scoped_requests: Arc<AtomicUsize>,
        valid_token: Arc<RwLock<Option<String>>>,
        scoped_status: StatusCode,
    }

    impl ScopedServerState {
        fn new(scoped_status: StatusCode) -> Self {
            Self {
                viewer_creations: Arc::new(AtomicUsize::new(0)),
                scoped_requests: Arc::new(AtomicUsize::new(0)),
                valid_token: Arc::new(RwLock::new(None)),
                scoped_status,
            }
        }
    }

    async fn create_viewer(State(state): State<ScopedServerState>) -> Json<serde_json::Value> {
        let creation = state.viewer_creations.fetch_add(1, Ordering::SeqCst) + 1;
        let token = format!("viewer-{creation}");
        *state.valid_token.write().await = Some(token.clone());
        Json(serde_json::json!({"viewer_token": token}))
    }

    async fn root_view(
        State(state): State<ScopedServerState>,
        headers: HeaderMap,
    ) -> (StatusCode, Json<serde_json::Value>) {
        state.scoped_requests.fetch_add(1, Ordering::SeqCst);
        if state.scoped_status != StatusCode::OK {
            return (state.scoped_status, Json(serde_json::json!({})));
        }
        let supplied = headers
            .get(VIEWER_HEADER)
            .and_then(|value| value.to_str().ok());
        let valid = state.valid_token.read().await;
        if valid.is_none() || supplied != valid.as_deref() {
            return (StatusCode::NOT_FOUND, Json(serde_json::json!({})));
        }
        (
            StatusCode::OK,
            Json(serde_json::json!({
                "self_view": {"agent_id": "root", "status": "Running"},
                "children": []
            })),
        )
    }

    async fn spawn_scoped_server(
        scoped_status: StatusCode,
    ) -> (String, ScopedServerState, JoinHandle<()>) {
        let state = ScopedServerState::new(scoped_status);
        let app = Router::new()
            .route("/api/v1/ui/viewers", post(create_viewer))
            .route("/api/v1/agents/self", get(root_view))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind scoped test server");
        let address = listener.local_addr().expect("read scoped server address");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve scoped test server");
        });
        (format!("http://{address}"), state, server)
    }

    #[tokio::test]
    async fn root_view_creates_viewer_before_first_scoped_request() {
        let (base_url, state, server) = spawn_scoped_server(StatusCode::OK).await;
        let client = DaemonClient::new(base_url);

        let view = client
            .get_root_agent_view("session-a")
            .await
            .expect("fetch root view");

        assert_eq!(view.self_view.agent_id, "root");
        assert_eq!(state.viewer_creations.load(Ordering::SeqCst), 1);
        assert_eq!(state.scoped_requests.load(Ordering::SeqCst), 1);
        server.abort();
    }

    #[tokio::test]
    async fn root_view_refreshes_stale_viewer_after_not_found() {
        let (base_url, state, server) = spawn_scoped_server(StatusCode::OK).await;
        let client = DaemonClient::new(base_url);
        *client.viewer_token.write().await = Some("stale-viewer".to_string());

        let view = client
            .get_root_agent_view("session-a")
            .await
            .expect("refresh stale viewer and fetch root view");

        assert_eq!(view.self_view.agent_id, "root");
        assert_eq!(state.viewer_creations.load(Ordering::SeqCst), 1);
        assert_eq!(state.scoped_requests.load(Ordering::SeqCst), 2);
        server.abort();
    }

    #[tokio::test]
    async fn root_view_retries_auth_failure_only_once() {
        let (base_url, state, server) = spawn_scoped_server(StatusCode::NOT_FOUND).await;
        let client = DaemonClient::new(base_url);

        let error = client
            .get_root_agent_view("session-a")
            .await
            .expect_err("return final not-found response after one refresh");

        assert!(error.to_string().contains("404 Not Found"));
        assert_eq!(state.viewer_creations.load(Ordering::SeqCst), 2);
        assert_eq!(state.scoped_requests.load(Ordering::SeqCst), 2);
        server.abort();
    }

    #[tokio::test]
    async fn root_view_does_not_refresh_viewer_after_server_error() {
        let (base_url, state, server) =
            spawn_scoped_server(StatusCode::INTERNAL_SERVER_ERROR).await;
        let client = DaemonClient::new(base_url);

        let error = client
            .get_root_agent_view("session-a")
            .await
            .expect_err("return server error without refreshing viewer");

        assert!(error.to_string().contains("500 Internal Server Error"));
        assert_eq!(state.viewer_creations.load(Ordering::SeqCst), 1);
        assert_eq!(state.scoped_requests.load(Ordering::SeqCst), 1);
        server.abort();
    }

    /// Simulates a daemon restart: the server only accepts the *new* bearer
    /// token while the client was constructed with the old one. The first
    /// attempt 401s, the client re-reads the (mocked) on-disk token and the
    /// retried request succeeds.
    #[tokio::test]
    async fn run_session_retries_once_with_refreshed_token_after_401() {
        let disk_token = Arc::new(std::sync::RwLock::new("old-token".to_string()));
        let requests = Arc::new(AtomicUsize::new(0));

        let handler_disk_token = disk_token.clone();
        let handler_requests = requests.clone();
        let app = Router::new().route(
            "/api/v1/sessions/:id/run",
            post(move |headers: HeaderMap| {
                let disk_token = handler_disk_token.clone();
                let requests = handler_requests.clone();
                async move {
                    requests.fetch_add(1, Ordering::SeqCst);
                    let supplied = headers
                        .get(axum::http::header::AUTHORIZATION)
                        .and_then(|value| value.to_str().ok())
                        .and_then(|value| value.strip_prefix("Bearer "));
                    if supplied == Some("new-token") {
                        (StatusCode::OK, Json(serde_json::json!({"run_id": "r1"})))
                    } else {
                        // The "restart" lands the new token on disk before
                        // rejecting the old one.
                        *disk_token.write().expect("disk token lock") = "new-token".to_string();
                        (StatusCode::UNAUTHORIZED, Json(serde_json::json!({})))
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind auth-retry test server");
        let address = listener.local_addr().expect("read test server address");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve auth-retry test server");
        });

        let reader_disk_token = disk_token.clone();
        let client = DaemonClient::with_token_reader(
            format!("http://{address}"),
            Arc::new(move || Some(reader_disk_token.read().expect("disk token lock").clone())),
        );

        let run_id = client
            .run_session("session-a", "hello", false)
            .await
            .expect("run_session recovers from 401 via refreshed token");

        assert_eq!(run_id, "r1");
        assert_eq!(requests.load(Ordering::SeqCst), 2);
        server.abort();
    }

    /// When the on-disk token did not change, the 401 is surfaced as-is
    /// (single attempt) instead of retrying in a loop.
    #[tokio::test]
    async fn run_session_does_not_retry_401_when_token_unchanged() {
        let requests = Arc::new(AtomicUsize::new(0));
        let handler_requests = requests.clone();
        let app = Router::new().route(
            "/api/v1/sessions/:id/run",
            post(move || {
                let requests = handler_requests.clone();
                async move {
                    requests.fetch_add(1, Ordering::SeqCst);
                    (StatusCode::UNAUTHORIZED, Json(serde_json::json!({})))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind no-retry test server");
        let address = listener.local_addr().expect("read test server address");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve no-retry test server");
        });

        let client = DaemonClient::with_token_reader(
            format!("http://{address}"),
            Arc::new(|| Some("same-token".to_string())),
        );

        let error = client
            .run_session("session-a", "hello", false)
            .await
            .expect_err("surface the 401 without retrying");

        assert!(
            error.to_string().contains("401"),
            "unexpected error: {error}"
        );
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        server.abort();
    }
}
