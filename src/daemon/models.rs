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
    /// Canonical tool name used for AcceptEdits / mode auto-approve matching.
    /// Distinct from `session_rule`, which may be a path/command-scoped key.
    pub tool_name: String,
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
    /// Sandbox effective mode including Plan. When omitted, derived from `mode`.
    #[serde(default)]
    pub effective_mode: Option<crate::sandbox::EffectiveMode>,
}

// ── Model Switch ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SwitchModelRequest {
    /// Profile key in `models.profiles` to activate.
    pub profile: String,
}

#[derive(Debug, Serialize)]
pub struct SwitchModelResponse {
    pub success: bool,
    /// The activated profile key.
    pub profile: String,
    /// Human-readable label (`display_name` or model name).
    pub label: String,
    /// The underlying model name now in `models.main`.
    pub model_name: String,
    /// Resolved provider, if any.
    pub provider: Option<String>,
}

/// One selectable entry for the `/model` picker, returned by `list_models`.
#[derive(Debug, Serialize)]
pub struct ModelProfileInfo {
    pub key: String,
    pub label: String,
    pub model_name: String,
    pub provider: Option<String>,
    /// Declared tier (`"light"`/`"medium"`/`"heavy"`), if the profile set one.
    pub tier: Option<String>,
    pub active: bool,
}

#[derive(Debug, Serialize)]
pub struct ListModelsResponse {
    pub profiles: Vec<ModelProfileInfo>,
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

/// Worktree reference exposed in session responses (project → worktree → session).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeRef {
    pub path: String,
    pub branch: String,
}

#[derive(Debug, Serialize)]
pub struct SessionInfoResponse {
    pub id: String,
    pub name: String,
    pub project_path: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub message_count: usize,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<WorktreeRef>,
}

#[derive(Debug, Serialize)]
pub struct SessionResponse {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub updated_at: String,
    pub messages: Vec<SessionMessage>,
    /// Human-facing TUI transcript; empty for legacy sessions.
    #[serde(default)]
    pub ui_messages: Vec<crate::context::SessionUiMessage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<WorktreeRef>,
}

#[derive(Debug, Deserialize)]
pub struct CreateSessionRequest {
    #[serde(default)]
    pub name: Option<String>,
    /// Project root the session belongs to (must be the main project or a
    /// registered one). `None` = main project (legacy behavior).
    #[serde(default)]
    pub project_path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSessionRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub messages: Option<Vec<SessionMessage>>,
    /// When `Some`, replace the UI transcript track. `None` leaves existing data.
    #[serde(default)]
    pub ui_messages: Option<Vec<crate::context::SessionUiMessage>>,
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
    /// Unix epoch ms when this agent started (0 if unknown).
    #[serde(default)]
    pub started_at: i64,
    /// Elapsed wall-clock ms; live for running agents (recomputed by the daemon).
    #[serde(default)]
    pub elapsed_ms: u64,
    /// Current round index, if reported by the subagent loop.
    #[serde(default)]
    pub round: Option<usize>,
    /// Maximum rounds configured for this agent.
    #[serde(default)]
    pub max_rounds: Option<usize>,
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
    /// Unix epoch ms when this subagent started (0 if unknown).
    #[serde(default)]
    pub started_at: i64,
    /// Elapsed wall-clock ms; live for running subagents (recomputed by the daemon).
    #[serde(default)]
    pub elapsed_ms: u64,
    /// Current round index, if reported by the subagent loop.
    #[serde(default)]
    pub round: Option<usize>,
    /// Maximum rounds configured for this subagent.
    #[serde(default)]
    pub max_rounds: Option<usize>,
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

// ── Memory ops API (Tier 2 web-ops-console: ops-panel-api) ────────────────────
// Wraps the existing MemoryManager so the web frontend can inspect/prune the
// memory pools. MemoryOrigin is intentionally not Serialize on the model
// (src/context/entry.rs), so the ops DTOs carry a lowercase `origin` string.

/// `GET /api/v1/memory` query filters.
#[derive(Debug, Deserialize, Default)]
pub struct MemoryListQuery {
    pub scope: Option<String>, // "project" | "global" | "all" (default all)
    pub min_importance: Option<f32>,
    pub limit: Option<usize>,
    /// Project whose memory pool to query (`None` = main project).
    #[serde(default)]
    pub project: Option<String>,
}

/// Query for memory endpoints that only need the project selector.
#[derive(Debug, Deserialize)]
pub struct MemoryProjectQuery {
    /// Project whose memory pool to use (`None` = main project).
    #[serde(default)]
    pub project: Option<String>,
}

/// One memory item with its origin annotated (the model's MemoryOrigin is not
/// serialized, so we project to a string here). MemoryEntry fields are flattened
/// in so the client sees id/content/importance/etc. at the top level.
#[derive(Debug, Serialize)]
pub struct MemoryItemResponse {
    pub origin: String, // "project" | "global"
    #[serde(flatten)]
    pub entry: crate::context::MemoryEntry,
}

#[derive(Debug, Serialize)]
pub struct MemoryListResponse {
    pub items: Vec<MemoryItemResponse>,
    pub total: usize,
}

/// `GET /api/v1/memory/:id` — single item with origin. MemoryEntry already
/// serializes its own id/content/etc.; we just add `origin` alongside by
/// projecting to a Value rather than flatten (avoids field-name coupling).
#[derive(Debug, Serialize)]
pub struct MemoryDetailResponse {
    pub origin: String,
    #[serde(flatten)]
    pub entry: crate::context::MemoryEntry,
}

/// `POST /api/v1/memory/prune` request. `dry_run` defaults false.
#[derive(Debug, Deserialize, Default)]
pub struct PruneRequest {
    #[serde(default)]
    pub dry_run: bool,
}

/// `POST /api/v1/interactions/:id/resolve` — answer a pending ask_user_question.
#[derive(Debug, Deserialize)]
pub struct ResolveInteractionRequest {
    /// The user's answer: a JSON string (selected option values or free text).
    pub answer: String,
}
