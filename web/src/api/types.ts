/**
 * TypeScript types mirroring the wgenty-code daemon API.
 *
 * Source of truth:
 *   - src/daemon/models.rs        (request/response structs)
 *   - src/api/types.rs            (ChatMessage, ToolCall, StreamChunk, Delta)
 *   - src/tui/client.rs           (client-side view structs)
 *
 * Field names match the Rust serde names 1:1 so they can be fed straight to
 * fetch() / JSON.parse without renaming. Optional Rust fields (Option<T> with
 * serde default / skip_if_none) become `undefined`-able TS fields.
 */

// ── Chat / messages ──────────────────────────────────────────────────────────

/** Mirrors `crate::api::ChatMessage` (src/api/types.rs:58). */
export interface ChatMessage {
  role: string;
  content?: string;
  reasoning_content?: string;
  tool_calls?: ToolCall[];
  tool_call_id?: string;
}

/** Mirrors `crate::api::ToolCall` — note `arguments` is a JSON **string**. */
export interface ToolCall {
  id: string;
  /** serde field is `r#type`; serialized JSON key is `type`. */
  type: string;
  function: ToolCallFunction;
}

export interface ToolCallFunction {
  name: string;
  /** JSON-encoded argument string (as OpenAI tool-calling specifies). */
  arguments: string;
}

/** Request body for `POST /api/v1/chat/stream`. Mirrors `ChatStreamRequest`. */
export interface ChatStreamRequest {
  messages: ChatMessage[];
  model?: string;
  max_tokens?: number;
  plan_mode?: boolean;
}

// ── SSE stream shapes (OpenAI-compatible chat.completion.chunk) ──────────────

/** Mirrors `crate::api::StreamChunk`. Each `data:` line in the chat stream. */
export interface StreamChunk {
  id: string;
  object: string;
  created: number;
  model: string;
  choices: StreamChoice[];
  usage?: Usage;
}

export interface Usage {
  prompt_tokens?: number;
  completion_tokens?: number;
  total_tokens?: number;
}

export interface StreamChoice {
  index: number;
  delta: Delta;
  finish_reason?: string;
}

export interface Delta {
  role?: string;
  content?: string;
  reasoning_content?: string;
  tool_calls?: StreamToolCall[];
}

/** A single tool-call delta fragment (may carry only `index` + `arguments`). */
export interface StreamToolCall {
  index: number;
  id?: string;
  function?: { name?: string; arguments?: string };
}

// ── Health / config ──────────────────────────────────────────────────────────

export interface HealthResponse {
  status: string;
  version: string;
}

export interface ConfigResponse {
  model: string;
  api_base: string;
  max_tokens: number;
  timeout: number;
  streaming: boolean;
}

// ── Todos / Tasks ────────────────────────────────────────────────────────────

/** Mirrors `TodoItemResponse` (src/daemon/models.rs:231). */
export interface TodoItem {
  content: string;
  status: string; // "pending" | "in_progress" | "completed"
  active_form?: string;
}

/** Mirrors `GetTodosResponse`. */
export interface GetTodosResponse {
  items: TodoItem[];
  has_open_items: boolean;
  display: string;
}

/** Mirrors `TaskInfo` (src/daemon/models.rs:205). */
export interface TaskInfo {
  id: string;
  subject: string;
  description: string;
  status: string; // "pending" | "in_progress" | "completed" | "deleted"
  priority: string; // "low" | "medium" | "high" | "critical"
  created_at: string;
  updated_at: string;
  tags: string[];
}

export interface ListTasksResponse {
  tasks: TaskInfo[];
}

export interface TaskProgressResponse {
  blocked: number;
  ready: number;
}

// ── Memory (Tier 2 ops-panel-api) ────────────────────────────────────────────

/** A single memory entry. Fields mirror MemoryEntry (src/context/entry.rs);
 *  flattened with `origin` in the list/detail responses. Modeled loosely. */
export interface MemoryEntry {
  id: string;
  memory_type: string;
  content: string;
  importance: number;
  timestamp: string;
  tags: string[];
  recall_count?: number;
  hit_count?: number;
  retrieval_mode?: string;
}

export interface MemoryItem extends MemoryEntry {
  origin: "project" | "global";
}

export interface MemoryListQuery {
  scope?: "project" | "global" | "all";
  min_importance?: number;
  limit?: number;
}

export interface MemoryListResponse {
  items: MemoryItem[];
  total: number;
}

export interface MemoryStatus {
  total_memories: number;
  session_count: number;
  conversation_count: number;
  knowledge_count: number;
  last_consolidation?: string;
  storage_size_bytes: number;
  project_count: number;
  global_count: number;
}

export interface PruneResult {
  before: number;
  after: number;
  removed: number;
  project_before: number;
  project_after: number;
  global_before: number;
  global_after: number;
}

// ── Trace SSE (subagent progress + permission events) ────────────────────────

/** Mirrors `StructuredApproval` (src/teams/permission_bridge.rs). Carried in
 *  TraceEvent.permission for permission_pending / permission_resolved events. */
export interface StructuredApproval {
  request_id: string;
  from: string;
  kind: string;
  tool: string;
  policy_reason: string;
  session_rule: string;
  paths?: string[];
  command?: string;
  risk?: string;
  human_summary: string;
}

/** Mirrors `TraceEvent` (src/teams/trace_sink.rs). `kind` discriminates:
 *  progress (subagent update) vs permission_pending / permission_resolved. */
export interface TraceEvent {
  ts: number;
  session_id: string;
  node_id: string;
  parent_id?: string | null;
  label: string;
  status: string;
  round?: number | null;
  current_tool?: string | null;
  current_params?: unknown;
  elapsed_ms: number;
  progress_delta?: number | null;
  token_budget_k?: number | null;
  cumulative_tokens: number;
  error?: unknown;
  kind?: "progress" | "permission_pending" | "permission_resolved";
  permission?: StructuredApproval;
}

// ── Tools ────────────────────────────────────────────────────────────────────

export interface ToolInfo {
  name: string;
  description: string;
  input_schema: Record<string, unknown>;
  is_read_only: boolean;
}

export interface ListToolsResponse {
  tools: ToolInfo[];
}

/** `POST /api/v1/tools/execute` request. Mirrors `ExecuteToolRequest`. */
export interface ExecuteToolRequest {
  tool_name: string;
  arguments: Record<string, unknown>;
  session_id?: string;
  turn_id?: string;
}

/** `POST /api/v1/tools/execute` response. Mirrors `ExecuteToolResponse`. */
export interface ExecuteToolResponse {
  success: boolean;
  output_type?: string;
  content?: string;
  error?: string;
  metadata?: Record<string, unknown>;
  permission_required?: PermissionRequiredInfo;
}

export interface PermissionRequiredInfo {
  tool_name: string;
  reason: string;
  session_rule: string;
}

// ── Permissions ──────────────────────────────────────────────────────────────

/** Subagent async permission queue (second-phase; not in MVP but typed for later). */
export interface PendingSubagentPermission {
  request_id: string;
  from: string;
  kind: string;
  tool: string;
  policy_reason: string;
  session_rule: string;
  human_summary: string;
}

/** User's decision for a `permission_required` prompt. */
export type PermissionDecision = "allowOnce" | "alwaysAllow" | "deny";

// ── Models ───────────────────────────────────────────────────────────────────

export interface ModelOption {
  key: string;
  label: string;
  model_name: string;
  provider?: string;
  tier?: "light" | "medium" | "heavy";
  active: boolean;
}

export interface ListModelsResponse {
  profiles: ModelOption[];
}

export interface SwitchModelRequest {
  profile: string;
}

export interface SwitchModelResponse {
  success: boolean;
  profile: string;
  label: string;
  model_name: string;
  provider?: string;
}

// ── Sessions ─────────────────────────────────────────────────────────────────

export interface SessionInfo {
  id: string;
  name: string;
  project_path?: string;
  created_at: string;
  updated_at: string;
  message_count: number;
  summary?: string;
}

export interface SessionMessage {
  role: string;
  content?: string;
  tool_calls?: ToolCall[];
  tool_call_id?: string;
}

export interface SessionResponse {
  id: string;
  name: string;
  created_at: string;
  updated_at: string;
  messages: SessionMessage[];
  ui_messages: unknown[];
}

export interface CreateSessionRequest {
  name?: string;
}

export interface UpdateSessionRequest {
  name?: string;
  messages?: SessionMessage[];
  ui_messages?: unknown[];
}

// ── Agent loop result (client-side, not from daemon) ─────────────────────────

/** A full tool call reassembled from SSE fragments by `StreamProcessor`. */
export interface AssembledToolCall {
  id: string;
  type: string;
  function: { name: string; arguments: string };
}
