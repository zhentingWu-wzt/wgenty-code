//! API request/response models for the daemon HTTP API.

use crate::api::ChatMessage;
use crate::config::agent::RootPermissionMode;
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
    /// Trusted identifier of the originating root turn, propagated into
    /// `ToolContext::origin_turn_id` so identity-sensitive tools (e.g. `task`)
    /// can group root-direct children under one turn. Optional; model-supplied
    /// `_turn_id` arguments are never honored.
    #[serde(default)]
    pub turn_id: Option<String>,
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

/// Pending subagent policy-Ask approval (structured).
#[derive(Debug, Serialize, Clone)]
pub struct PendingSubagentPermission {
    pub request_id: String,
    pub from: String,
    pub kind: String,
    pub tool: String,
    pub policy_reason: String,
    pub session_rule: String,
    pub human_summary: String,
}

#[derive(Debug, Serialize)]
pub struct ListPendingPermissionsResponse {
    pub pending: Vec<PendingSubagentPermission>,
}

#[derive(Debug, Deserialize)]
pub struct ResolveSubagentPermissionRequest {
    pub request_id: String,
    pub approved: bool,
    /// When true and approved, also record `session_rule` for future matches.
    #[serde(default)]
    pub always: bool,
    /// Required when `always` is true (or recommended always for AlwaysAllow).
    #[serde(default)]
    pub session_rule: Option<String>,
}

// ── Permission Mode ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SetPermissionModeRequest {
    pub mode: RootPermissionMode,
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

/// `GET /api/v1/tasks/progress` - ready vs blocked counts for agent nudges.
#[derive(Debug, Serialize)]
pub struct TaskProgressResponse {
    pub blocked: usize,
    pub ready: usize,
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
    #[serde(default)]
    pub label: String,
    /// Latest text snapshot from the subagent loop (displayed when messages are empty).
    #[serde(default)]
    pub text_snapshot: Option<String>,
    /// Cumulative tokens consumed by this agent.
    #[serde(default)]
    pub cumulative_tokens: u64,
    /// Model messages captured by the progress callback during the subagent loop.
    #[serde(default)]
    pub messages: Vec<crate::api::ChatMessage>,
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

// ── Unified subagent lifecycle: task-group delivery ───────────────────────

use crate::agent::ChildResult;

/// `POST /api/v1/agents/task-groups/claim` -- atomically claim one ready
/// root-direct task group for the persistent main agent.
#[derive(Debug, Deserialize)]
pub struct ClaimTaskGroupRequest {
    pub session_id: String,
    pub generation: u64,
}

/// One delivered task-group batch. Returned with `200 OK` when a ready group
/// is claimed, or absent (204 No Content) when nothing is ready.
#[derive(Debug, Serialize, Deserialize)]
pub struct TaskGroupDeliveryResponse {
    pub group_id: String,
    pub generation: u64,
    pub results: Vec<ChildResult>,
}

/// `POST /api/v1/agents/generation/reset` -- advance the session generation,
/// cancelling obsolete root-direct subtrees. Returns the new generation.
#[derive(Debug, Deserialize)]
pub struct ResetAgentGenerationRequest {
    pub session_id: String,
}

#[derive(Debug, Serialize)]
pub struct ResetAgentGenerationResponse {
    pub generation: u64,
}
