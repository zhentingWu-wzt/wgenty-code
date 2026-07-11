//! API request/response models for the daemon HTTP API.

use crate::api::ChatMessage;
use crate::context::memory_session::SessionMessage;
use serde::{Deserialize, Serialize};

// ── Health ───────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
}

// ── Config ───────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct ConfigResponse {
    pub model: String,
    pub api_base: String,
    pub max_tokens: usize,
    pub timeout: u64,
    pub streaming: bool,
}

// ── Chat ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ChatStreamRequest {
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub max_tokens: Option<usize>,
    #[serde(default)]
    pub plan_mode: Option<bool>,
}

// ── Tools ────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    pub is_read_only: bool,
}

#[derive(Debug, Serialize)]
pub struct ListToolsResponse {
    pub tools: Vec<ToolInfo>,
}

#[derive(Debug, Deserialize)]
pub struct ExecuteToolRequest {
    pub tool_name: String,
    pub arguments: serde_json::Value,
    #[serde(default)]
    pub session_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ExecuteToolResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_required: Option<PermissionRequiredInfo>,
}

#[derive(Debug, Serialize)]
pub struct PermissionRequiredInfo {
    pub reason: String,
    pub session_rule: String,
}

#[derive(Debug, Deserialize)]
pub struct ApproveToolRequest {
    pub session_rule: String,
}

// ── MCP ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct McpServerInfo {
    pub name: String,
    pub status: String,
    pub tools_count: usize,
    pub resources_count: usize,
}

#[derive(Debug, Serialize)]
pub struct ListMcpServersResponse {
    pub servers: Vec<McpServerInfo>,
}

#[derive(Debug, Deserialize)]
pub struct AddMcpServerRequest {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub auto_start: bool,
}

// ── Tasks ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct TaskInfo {
    pub id: String,
    pub subject: String,
    pub description: String,
    pub status: String,
    pub priority: String,
    pub created_at: String,
    pub updated_at: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ListTasksResponse {
    pub tasks: Vec<TaskInfo>,
}

// ── Todos (s03 TodoWrite) ─────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct TodoItemResponse {
    pub content: String,
    pub status: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub active_form: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent: Option<crate::tasks::SubagentTodoMeta>,
}

#[derive(Debug, Serialize)]
pub struct GetTodosResponse {
    pub items: Vec<TodoItemResponse>,
    pub has_open_items: bool,
    pub display: String,
}

// ── Sessions ──────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct SessionInfoResponse {
    pub id: String,
    pub name: String,
    pub project_path: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub message_count: usize,
    pub status: String,
}

#[derive(Debug, Serialize)]
pub struct SessionResponse {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub updated_at: String,
    pub messages: Vec<SessionMessage>,
}

#[derive(Debug, Deserialize)]
pub struct CreateSessionRequest {
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSessionRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub messages: Option<Vec<SessionMessage>>,
}

#[derive(Debug, Deserialize)]
pub struct SearchSessionsQuery {
    pub q: String,
}

// ── Scoped agent views (strict subagent isolation) ───────────────────────────

use crate::agent::AgentLifecycleStatus;

/// Self projection in a scoped agent view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfAgentResponse {
    pub agent_id: String,
    pub status: AgentLifecycleStatus,
}

/// Direct-child projection, including an opaque navigation capability the
/// trusted UI may use to descend one level.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectChildResponse {
    pub agent_id: String,
    pub status: AgentLifecycleStatus,
    #[serde(default)]
    pub label: String,
    pub summary: Option<String>,
    pub navigation_capability: String,
    /// Latest text snapshot from the subagent loop (displayed when messages are empty).
    #[serde(default)]
    pub text_snapshot: Option<String>,
    /// Cumulative tokens consumed by this subagent.
    #[serde(default)]
    pub cumulative_tokens: u64,
    /// Model messages captured by the progress callback during the subagent
    /// loop. Carried for the TUI focus view; not intended for model consumption.
    #[serde(default)]
    pub messages: Vec<crate::api::ChatMessage>,
}

/// Local view: self plus direct children only. No parent ID, descendant
/// counts, or sibling/other-branch records.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalAgentViewResponse {
    pub self_view: SelfAgentResponse,
    pub children: Vec<DirectChildResponse>,
}

/// Response to `POST /api/v1/ui/viewers`: a bearer token returned once. The
/// daemon stores only the HMAC digest of the token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateViewerResponse {
    pub viewer_token: String,
}
