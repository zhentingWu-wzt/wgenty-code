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

// ── MCP servers ──────────────────────────────────────────────────────────────

export interface McpServerInfo {
  name: string;
  status: string;
  tools_count: number;
  resources_count: number;
}

export interface AddMcpServerRequest {
  name: string;
  command: string;
  args?: string[];
  env?: Record<string, string>;
  auto_start?: boolean;
}

// ── Permission mode ──────────────────────────────────────────────────────────

/** Mirrors `RootPermissionMode` (src/config/agent.rs:110). serde rename_all = snake_case. */
export type PermissionMode = "normal" | "accept_edits" | "yolo";

/** Response from GET/POST /api/v1/permission-mode. */
export interface PermissionModeResponse {
  mode: PermissionMode;
  effective_mode: string;
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
  /** Terminal-only final result text of the subagent (present on the last
   *  progress event once status is completed/failed/cancelled). */
  result?: string | null;
  kind?:
    | "progress"
    | "permission_pending"
    | "permission_resolved"
    | "question_pending"
    | "question_resolved";
  permission?: StructuredApproval;
  question?: QuestionPayload;
}

// ── Scoped agent views (subagent local view + transcript) ───────────────────

/** Mirrors `AgentLifecycleStatus` (serde variant names). Kept as a loose string
 *  union so unknown future variants still render without a type error. */
export type AgentLifecycleStatus =
  | "idle"
  | "running"
  | "thinking"
  | "awaiting_approval"
  | "awaiting_input"
  | "completed"
  | "done"
  | "failed"
  | "cancelled"
  | (string & {});

/** Self projection in a scoped agent view. Mirrors `SelfAgentResponse`. */
export interface SelfAgentResponse {
  agent_id: string;
  status: AgentLifecycleStatus;
  label: string;
  text_snapshot?: string | null;
  cumulative_tokens: number;
  started_at: number;
  elapsed_ms: number;
  round?: number | null;
  max_rounds?: number | null;
  /** Model messages captured by the progress callback. */
  messages: ChatMessage[];
}

/** Direct-child projection with an opaque navigation capability. Mirrors
 *  `DirectChildResponse`. */
export interface DirectChildResponse {
  agent_id: string;
  status: AgentLifecycleStatus;
  label: string;
  summary?: string | null;
  navigation_capability: string;
  text_snapshot?: string | null;
  cumulative_tokens: number;
  started_at: number;
  elapsed_ms: number;
  round?: number | null;
  max_rounds?: number | null;
  messages: ChatMessage[];
}

/** Local view: self plus direct children only. Mirrors
 *  `LocalAgentViewResponse`. */
export interface LocalAgentViewResponse {
  self_view: SelfAgentResponse;
  children: DirectChildResponse[];
}

/** One node of the recursive subagent tree. Mirrors `AgentDirectoryEntry`
 *  (src/daemon/models.rs). */
export interface AgentDirectoryEntry {
  agent_id: string;
  status: AgentLifecycleStatus;
  label: string;
  summary: string | null;
  cumulative_tokens: number;
  started_at: number;
  elapsed_ms: number;
  round: number | null;
  max_rounds: number | null;
  depth: number;
  children: AgentDirectoryEntry[];
}

/** Full subagent tree for a session. Mirrors `AgentDirectoryResponse`
 *  (src/daemon/models.rs). */
export interface AgentDirectoryResponse {
  session_id: string;
  root: AgentDirectoryEntry;
}

/** `POST /api/v1/ui/viewers` response. */
export interface CreateViewerResponse {
  viewer_token: string;
}

// ── ask_user_question (server-side interaction) ──────────────────────────────

export interface QuestionOption {
  label: string;
  description: string;
  preview?: string;
}

/** Mirrors QuestionPayload (src/daemon/interaction_bridge.rs). Pushed via the
 *  trace SSE when the server-side loop blocks on ask_user_question. */
export interface QuestionPayload {
  request_id: string;
  session_id: string;
  question: string;
  options: QuestionOption[];
  multi_select: boolean;
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

/** A session's worktree binding (project → worktree → session, N:1). */
export interface WorktreeBinding {
  path: string;
  branch: string;
}

export interface SessionInfo {
  id: string;
  name: string;
  /** Owning project's canonical path; null = main project (historical sessions). */
  project_path?: string | null;
  created_at: string;
  updated_at: string;
  message_count: number;
  summary?: string;
  /** "Active" | "Paused" | "Archived" | "Error" — archived sessions are
   *  hidden from default list views (client-side filtering). */
  status: string;
  /** Bound worktree; absent/null = main checkout. */
  worktree?: WorktreeBinding | null;
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
  /** Registered project to create the session in; omitted = main project. */
  project_path?: string;
}

// ── Agent loop result (client-side, not from daemon) ─────────────────────────

/** A full tool call reassembled from SSE fragments by `StreamProcessor`. */
export interface AssembledToolCall {
  id: string;
  type: string;
  function: { name: string; arguments: string };
}

// ── Command center (projects / worktrees / skills / checkpoints) ─────────────

/** Mirrors the daemon's project registry entry (GET /api/v1/projects). The
 *  main project (daemon working dir) is always first. */
export interface ProjectInfo {
  /** Canonical absolute path — the registry key. */
  path: string;
  name: string;
  is_main: boolean;
  /** Non-git projects reject the worktree endpoints (400), so the UI must
   *  skip worktree calls and git-only actions for them. */
  is_git_repo: boolean;
  added_at: string;
}

export interface WorktreeInfo {
  path: string;
  head: string;
  branch: string | null;
  is_main: boolean;
}

export interface SkillInfoDto {
  name: string;
  description: string;
  source_path: string;
}

export interface CheckpointInfo {
  turn_id: string;
  created_at: number;
  file_count: number;
}

export interface UndoTurnResult {
  restored: number;
  skipped: number;
  failed: number;
  rewound_turns: string[];
}

// ── Server-side run (web as observer) ────────────────────────────────────────

export type SessionEventKind =
  | "content_delta"
  | "reasoning_delta"
  | "tool_start"
  | "tool_result"
  | "turn_done"
  | "turn_error"
  | "save"
  | "turn_context";

/** Mirrors SessionEvent (src/daemon/run_loop.rs:26). Server-side run broadcasts
 *  these on GET /sessions/:id/events (SSE). data shape varies by kind. */
export interface SessionEvent {
  seq: number;
  session_id: string;
  run_id: string;
  kind: SessionEventKind;
  data: Record<string, unknown>;
}

/** Response to POST /sessions/:id/run. */
export interface RunResponse {
  run_id: string;
  session_id: string;
}

// ── Filesystem browsing (web directory picker) ──────────────────────────────

/** Mirrors DirEntry (src/daemon/fs.rs). */
export interface DirEntry {
  name: string;
  path: string;
  /** Dot-directory (.git, .config …) — frontend de-emphasizes via opacity. */
  is_hidden: boolean;
}

/** Mirrors DirListing (src/daemon/fs.rs). Response to GET /api/v1/fs/dirs. */
export interface DirListing {
  /** Canonicalized absolute path of the listed directory. */
  current: string;
  /** Parent directory, or null at a filesystem root. */
  parent: string | null;
  /** Sorted child directories (hidden ones interleaved). */
  entries: DirEntry[];
}
