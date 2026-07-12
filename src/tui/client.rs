//! HTTP client for communicating with the daemon API.
//! Mirrors the TypeScript ApiClient in packages/core/src/client.ts.

use std::sync::Arc;

use crate::api::ChatMessage;
use anyhow::Context;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use reqwest::{Method, Response, StatusCode};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct DaemonClient {
    /// Client for SSE streaming requests (no timeout — streams can run for minutes).
    http: reqwest::Client,
    /// Separate client for short-lived tool/API requests, avoiding connection-pool
    /// conflicts with the long-lived SSE streaming connection.
    http_tools: reqwest::Client,
    /// Client for long-running tools (`task`/`delegate`) whose subagents can run
    /// for many minutes. No timeout — the tool-level and subagent-level timeouts
    /// are the real ceilings, not the HTTP client (a 300s client timeout was
    /// killing legitimate subagent runs mid-flight).
    http_long: reqwest::Client,
    base_url: String,
    /// Trusted-UI viewer bearer token, obtained once from `POST /api/v1/ui/viewers`
    /// and refreshed only after a daemon restart (401/404). Sent as the
    /// `X-Wgenty-Viewer-Token` header on scoped agent requests. Stored behind
    /// a `RwLock` so `create_viewer` can take `&self`.
    viewer_token: Arc<tokio::sync::RwLock<Option<String>>>,
}

impl DaemonClient {
    pub fn new(base_url: String) -> Self {
        // Read auth token from token file (if present) and set as default
        // header for all requests. Protected endpoints require this header.
        let mut default_headers = HeaderMap::new();
        if let Some(token) = crate::utils::read_daemon_token() {
            if let Ok(val) = HeaderValue::from_str(&format!("Bearer {}", token)) {
                default_headers.insert(AUTHORIZATION, val);
            }
        }

        let http = reqwest::Client::builder()
            .default_headers(default_headers.clone())
            .build()
            .expect("reqwest client build");
        let http_tools = reqwest::Client::builder()
            .default_headers(default_headers.clone())
            .timeout(std::time::Duration::from_secs(300))
            .pool_max_idle_per_host(0) // don't keep idle connections — always fresh
            .build()
            .expect("reqwest tools client build");
        let http_long = reqwest::Client::builder()
            .default_headers(default_headers)
            .pool_max_idle_per_host(0)
            .build()
            .expect("reqwest long client build");
        Self {
            http,
            http_tools,
            http_long,
            base_url: base_url.trim_end_matches('/').to_string(),
            viewer_token: Arc::new(tokio::sync::RwLock::new(None)),
        }
    }

    /// POST /api/v1/ui/viewers — obtain a trusted-UI viewer token.
    pub async fn create_viewer(&self) -> anyhow::Result<()> {
        let url = format!("{}/api/v1/ui/viewers", self.base_url);
        let resp = self
            .http_tools
            .post(&url)
            .send()
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
        self.http_tools
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

    /// GET /api/v1/agents/children/:capability — navigate into a direct child.
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

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Check daemon health. Returns the health response.
    pub async fn health(&self) -> anyhow::Result<HealthResponse> {
        let url = format!("{}/api/v1/health", self.base_url);
        let resp = self.http.get(&url).send().await?;
        Ok(resp.json().await?)
    }

    /// Get daemon config.
    pub async fn get_config(&self) -> anyhow::Result<ConfigResponse> {
        let url = format!("{}/api/v1/config", self.base_url);
        let resp = self.http.get(&url).send().await?;
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
            .http
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("API error ({}): {}", status, text);
        }
        Ok(resp)
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
            &self.http_long
        } else {
            &self.http_tools
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
        self.http_tools
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
        self.http_tools
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({"session_rule": session_rule}))
            .send()
            .await?;
        Ok(())
    }

    /// GET /api/v1/undo — undo most recent checkpoint
    pub async fn undo(&self) -> anyhow::Result<String> {
        let url = format!("{}/api/v1/tools/undo", self.base_url);
        let resp = self.http.get(&url).send().await?;
        Ok(resp.text().await?)
    }

    /// GET /api/v1/background/results
    pub async fn get_background_results(&self) -> anyhow::Result<Vec<serde_json::Value>> {
        let url = format!("{}/api/v1/background/results", self.base_url);
        let resp = self.http_tools.get(&url).send().await?;
        if !resp.status().is_success() {
            return Ok(Vec::new());
        }
        let data: serde_json::Value = resp.json().await?;
        Ok(data["results"].as_array().cloned().unwrap_or_default())
    }

    /// GET /api/v1/sessions
    pub async fn list_sessions(&self) -> anyhow::Result<Vec<SessionInfo>> {
        let url = format!("{}/api/v1/sessions", self.base_url);
        let resp = self.http_tools.get(&url).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("Failed to list sessions ({})", resp.status());
        }
        Ok(resp.json().await?)
    }

    /// POST /api/v1/sessions
    pub async fn create_session(&self, name: Option<&str>) -> anyhow::Result<SessionResponse> {
        let url = format!("{}/api/v1/sessions", self.base_url);
        let resp = self
            .http_tools
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
        let resp = self.http_tools.get(&url).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("Failed to load session ({})", resp.status());
        }
        Ok(resp.json().await?)
    }

    /// PUT /api/v1/sessions/:id
    pub async fn save_session(
        &self,
        id: &str,
        name: &str,
        messages: &[ChatMessage],
    ) -> anyhow::Result<()> {
        let encoded = urlencode(id);
        let url = format!("{}/api/v1/sessions/{}", self.base_url, encoded);
        let resp = self
            .http_tools
            .put(&url)
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({"name": name, "messages": messages}))
            .send()
            .await?;
        if !resp.status().is_success() {
            anyhow::bail!("Failed to save session ({})", resp.status());
        }
        Ok(())
    }

    /// DELETE /api/v1/sessions/:id
    pub async fn delete_session(&self, id: &str) -> anyhow::Result<()> {
        let encoded = urlencode(id);
        let url = format!("{}/api/v1/sessions/{}", self.base_url, encoded);
        let resp = self.http_tools.delete(&url).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("Failed to delete session ({})", resp.status());
        }
        Ok(())
    }

    /// GET /api/v1/sessions/search?q=...
    pub async fn search_sessions(&self, query: &str) -> anyhow::Result<Vec<SessionInfo>> {
        let encoded = urlencode(query);
        let url = format!("{}/api/v1/sessions/search?q={}", self.base_url, encoded);
        let resp = self.http_tools.get(&url).send().await?;
        if !resp.status().is_success() {
            return Ok(Vec::new());
        }
        Ok(resp.json().await?)
    }

    /// GET /api/v1/todos
    pub async fn get_todos(&self) -> anyhow::Result<TodoResponse> {
        let url = format!("{}/api/v1/todos", self.base_url);
        let resp = self.http_tools.get(&url).send().await?;
        Ok(resp.json().await?)
    }
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
    pub reason: String,
    pub session_rule: String,
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
}

/// Metadata for subagent tasks in the TUI layer.
/// Mirrors `tasks::SubagentTodoMeta` — communicates via JSON serialization.
#[derive(Debug, Clone, Deserialize)]
pub struct SubagentTodoMeta {
    pub subagent_type: String,
    pub token_usage: u64,
    pub rounds: u32,
    pub duration_ms: u64,
}

#[derive(Debug, Deserialize)]
pub struct TodoItem {
    pub content: String,
    pub status: String,
    #[serde(default)]
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
}
