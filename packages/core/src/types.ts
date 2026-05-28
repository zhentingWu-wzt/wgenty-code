// ── ChatMessage ──────────────────────────────────────────────────────────────

export interface ChatMessage {
  role: "user" | "assistant" | "system" | "tool";
  content?: string | null;
  reasoning_content?: string | null;
  tool_calls?: ToolCall[] | null;
  tool_call_id?: string | null;
}

// ── Tool types ───────────────────────────────────────────────────────────────

export interface ToolCall {
  id: string;
  type: "function";
  function: {
    name: string;
    arguments: string;
  };
}

export interface ToolDefinition {
  type: "function";
  function: {
    name: string;
    description: string;
    parameters: Record<string, unknown>;
  };
}

export interface ToolInfo {
  name: string;
  description: string;
  input_schema: Record<string, unknown>;
  is_read_only: boolean;
}

// ── Stream chunk (OpenAI-compatible SSE) ────────────────────────────────────

export interface StreamChunk {
  id: string;
  object: string;
  created: number;
  model: string;
  choices: StreamChoice[];
}

export interface StreamChoice {
  index: number;
  delta: Delta;
  finish_reason?: string | null;
}

export interface Delta {
  role?: string | null;
  content?: string | null;
  reasoning_content?: string | null;
  tool_calls?: StreamToolCall[] | null;
}

export interface StreamToolCall {
  index: number;
  id?: string | null;
  type?: string | null;
  function?: {
    name?: string | null;
    arguments?: string | null;
  } | null;
}

// ── API request/response types ───────────────────────────────────────────────

export interface ChatStreamRequest {
  messages: ChatMessage[];
  model?: string;
  max_tokens?: number;
}

export interface ExecuteToolRequest {
  tool_name: string;
  arguments: Record<string, unknown>;
  session_id?: string;
}

export interface ExecuteToolResponse {
  success: boolean;
  output_type?: string | null;
  content?: string | null;
  error?: string | null;
  metadata?: Record<string, unknown> | null;
  permission_required?: {
    reason: string;
    session_rule: string;
  } | null;
}

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

// ── Session types ──────────────────────────────────────────────────────────────

export interface SessionInfo {
  id: string;
  name: string;
  created_at: string;
  updated_at: string;
  message_count: number;
  summary?: string | null;
}

export interface Session {
  id: string;
  name: string;
  created_at: string;
  updated_at: string;
  messages: ChatMessage[];
}
