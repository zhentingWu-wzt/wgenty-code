//! HTTP client for communicating with the daemon API.
//! Mirrors the TypeScript ApiClient in packages/core/src/client.ts.

use crate::api::ChatMessage;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct DaemonClient {
    /// Client for SSE streaming requests (no timeout — streams can run for minutes).
    http: reqwest::Client,
    /// Separate client for short-lived tool/API requests, avoiding connection-pool
    /// conflicts with the long-lived SSE streaming connection.
    http_tools: reqwest::Client,
    base_url: String,
}

impl DaemonClient {
    pub fn new(base_url: String) -> Self {
        let http = reqwest::Client::builder()
            .build()
            .expect("reqwest client build");
        let http_tools = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .pool_max_idle_per_host(0) // don't keep idle connections — always fresh
            .build()
            .expect("reqwest tools client build");
        Self {
            http,
            http_tools,
            base_url: base_url.trim_end_matches('/').to_string(),
        }
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
    ) -> anyhow::Result<ExecuteToolResponse> {
        let url = format!("{}/api/v1/tools/execute", self.base_url);
        let body = ExecuteToolRequest {
            tool_name: tool_name.to_string(),
            arguments,
            session_id: Some(session_id.to_string()),
        };
        let resp = self
            .http_tools
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

#[derive(Debug, Deserialize)]
pub struct TodoItem {
    pub content: String,
    pub status: String,
    pub active_form: String,
}

#[derive(Debug, Deserialize)]
pub struct TodoResponse {
    pub items: Vec<TodoItem>,
    pub has_open_items: bool,
    pub display: String,
}
