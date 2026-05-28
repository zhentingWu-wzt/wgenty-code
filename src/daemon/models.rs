//! API request/response models for the daemon HTTP API.

use crate::api::ChatMessage;
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
