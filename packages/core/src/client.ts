import type {
  ChatStreamRequest,
  ConfigResponse,
  ExecuteToolRequest,
  ExecuteToolResponse,
  HealthResponse,
  ToolInfo,
} from "./types.ts";

export interface ClientOptions {
  baseUrl: string;
}

/** Create an AbortSignal that fires after `ms` milliseconds. */
function timeoutSignal(ms: number): { signal: AbortSignal; clear: () => void } {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), ms);
  return { signal: controller.signal, clear: () => clearTimeout(timer) };
}

/**
 * HTTP client for the Rust daemon API.
 */
export class ApiClient {
  private baseUrl: string;

  constructor(options: ClientOptions) {
    this.baseUrl = options.baseUrl.replace(/\/$/, "");
  }

  // ── Health ───────────────────────────────────────────────────────────────

  async health(): Promise<HealthResponse> {
    const res = await fetch(`${this.baseUrl}/api/v1/health`);
    return res.json();
  }

  // ── Config ───────────────────────────────────────────────────────────────

  async getConfig(): Promise<ConfigResponse> {
    const res = await fetch(`${this.baseUrl}/api/v1/config`);
    return res.json();
  }

  // ── Chat / Stream ────────────────────────────────────────────────────────

  /**
   * Send a streaming chat request. Returns the raw Response for SSE reading.
   * Has a 300s timeout to prevent indefinite hanging.
   */
  async chatStream(request: ChatStreamRequest): Promise<Response> {
    const { signal, clear } = timeoutSignal(300_000);

    try {
      const res = await fetch(`${this.baseUrl}/api/v1/chat/stream`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(request),
        signal,
      });

      if (!res.ok) {
        const body = await res.text();
        throw new Error(`API error (${res.status}): ${body}`);
      }

      return res;
    } finally {
      clear();
    }
  }

  // ── Tools ────────────────────────────────────────────────────────────────

  async listTools(): Promise<{ tools: ToolInfo[] }> {
    const res = await fetch(`${this.baseUrl}/api/v1/tools`);
    return res.json();
  }

  async executeTool(request: ExecuteToolRequest): Promise<ExecuteToolResponse> {
    const { signal, clear } = timeoutSignal(120_000);

    try {
      const res = await fetch(`${this.baseUrl}/api/v1/tools/execute`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(request),
        signal,
      });

      if (!res.ok) {
        throw new Error(`Tool execution failed (${res.status})`);
      }

      return res.json();
    } finally {
      clear();
    }
  }

  async approveTool(sessionRule: string): Promise<void> {
    await fetch(`${this.baseUrl}/api/v1/tools/approve`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ session_rule: sessionRule }),
    });
  }

  // ── Background Tasks ─────────────────────────────────────────────────────

  async getBackgroundResults(): Promise<any[]> {
    const resp = await fetch(`${this.baseUrl}/api/v1/background/results`);
    if (!resp.ok) return [];
    const data = await resp.json();
    return data.results || [];
  }

  // ── Tasks ────────────────────────────────────────────────────────────────

  async listTasks(): Promise<TaskListResponse> {
    const res = await fetch(`${this.baseUrl}/api/v1/tasks`);
    return res.json();
  }

  // ── Todos (s03 TodoWrite) ────────────────────────────────────────────────

  async getTodos(): Promise<TodoResponse> {
    const res = await fetch(`${this.baseUrl}/api/v1/todos`);
    return res.json();
  }
}

export interface TodoItemInfo {
  content: string;
  status: string;
  active_form: string;
}

export interface TodoResponse {
  items: TodoItemInfo[];
  has_open_items: boolean;
  display: string;
}

export interface TaskInfo {
  id: string;
  subject: string;
  description: string;
  status: "pending" | "in_progress" | "completed" | "deleted";
  priority: "low" | "medium" | "high" | "critical";
  created_at: string;
  updated_at: string;
  tags: string[];
}

export interface TaskListResponse {
  tasks: TaskInfo[];
}
