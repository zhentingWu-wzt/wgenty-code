import type {
  ChatMessage,
  ChatStreamRequest,
  ConfigResponse,
  ExecuteToolRequest,
  ExecuteToolResponse,
  HealthResponse,
  Session,
  SessionInfo,
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
 * Detect network-related errors from fetch() and wrap them with user-friendly
 * messages so the UI can show meaningful status instead of raw TypeErrors.
 */
export function wrapFetchError(err: unknown, context?: string): Error {
  const ctx = context ? ` while ${context}` : "";
  const msg = String(err);

  // Already wrapped / has a message from the daemon
  if (msg.includes("⚠️")) return err as Error;

  // AbortError from timeout signal
  if (msg.includes("abort") || msg.includes("AbortError") || msg.includes("timed out")) {
    return new Error(
      `⚠️  Request timed out${ctx}. The LLM API may be slow or unreachable. ` +
      `Check your network connection or try again.`
    );
  }

  // DNS / hostname resolution failures
  if (msg.includes("ENOTFOUND") || msg.includes("getaddrinfo") || msg.includes("dns")) {
    return new Error(
      `⚠️  Cannot resolve API hostname${ctx}. DNS lookup failed — ` +
      `check your internet connection and DNS settings.`
    );
  }

  // Connection refused
  if (msg.includes("ECONNREFUSED") || msg.includes("refused")) {
    return new Error(
      `⚠️  Connection refused${ctx}. The LLM API server may be down ` +
      `or unreachable from your network (check VPN/firewall/proxy).`
    );
  }

  // Connection reset or pipe broken
  if (msg.includes("ECONNRESET") || msg.includes("reset") || msg.includes("broken pipe")) {
    return new Error(
      `⚠️  Connection was reset${ctx}. The network may be unstable — ` +
      `please try again.`
    );
  }

  // TLS / certificate errors
  if (msg.includes("TLS") || msg.includes("SSL") || msg.includes("certificate") || msg.includes("CERT")) {
    return new Error(
      `⚠️  TLS/SSL error${ctx}. Check your system certificates, proxy, or VPN configuration.`
    );
  }

  // Generic network / fetch failure (network offline, unreachable, etc.)
  if (
    msg.includes("fetch failed") ||
    msg.includes("NetworkError") ||
    msg.includes("network") ||
    msg.includes("Failed to fetch")
  ) {
    return new Error(
      `⚠️  Network is unreachable${ctx}. Please check your internet connection ` +
      `and ensure the API server is accessible.`
    );
  }

  // Already an Error with a meaningful message — pass through
  return err instanceof Error ? err : new Error(String(err));
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
    try {
      const res = await fetch(`${this.baseUrl}/api/v1/health`);
      return res.json();
    } catch (err) {
      throw wrapFetchError(err, "checking daemon health");
    }
  }

  // ── Config ───────────────────────────────────────────────────────────────

  async getConfig(): Promise<ConfigResponse> {
    try {
      const res = await fetch(`${this.baseUrl}/api/v1/config`);
      return res.json();
    } catch (err) {
      throw wrapFetchError(err, "fetching configuration");
    }
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
    } catch (err) {
      throw wrapFetchError(err, "starting chat stream");
    } finally {
      clear();
    }
  }

  // ── Tools ────────────────────────────────────────────────────────────────

  async listTools(): Promise<{ tools: ToolInfo[] }> {
    try {
      const res = await fetch(`${this.baseUrl}/api/v1/tools`);
      if (!res.ok) {
        const body = await res.text().catch(() => "");
        throw new Error(`Failed to list tools (${res.status}): ${body}`);
      }
      return res.json();
    } catch (err) {
      throw wrapFetchError(err, "listing tools");
    }
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
    } catch (err) {
      throw wrapFetchError(err, "executing tool");
    } finally {
      clear();
    }
  }

  async approveTool(sessionRule: string): Promise<void> {
    try {
      const res = await fetch(`${this.baseUrl}/api/v1/tools/approve`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ session_rule: sessionRule }),
      });
      if (!res.ok) {
        const body = await res.text().catch(() => "");
        throw new Error(`Failed to approve tool (${res.status}): ${body}`);
      }
    } catch (err) {
      throw wrapFetchError(err, "approving tool");
    }
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
    try {
      const res = await fetch(`${this.baseUrl}/api/v1/tasks`);
      if (!res.ok) {
        const body = await res.text().catch(() => "");
        throw new Error(`Failed to list tasks (${res.status}): ${body}`);
      }
      return res.json();
    } catch (err) {
      throw wrapFetchError(err, "listing tasks");
    }
  }

  // ── Todos (s03 TodoWrite) ────────────────────────────────────────────────

  async getTodos(): Promise<TodoResponse> {
    try {
      const res = await fetch(`${this.baseUrl}/api/v1/todos`);
      if (!res.ok) {
        const body = await res.text().catch(() => "");
        throw new Error(`Failed to get todos (${res.status}): ${body}`);
      }
      return res.json();
    } catch (err) {
      throw wrapFetchError(err, "getting todos");
    }
  }

  // ── Sessions ──────────────────────────────────────────────────────────────

  async listSessions(): Promise<SessionInfo[]> {
    const res = await fetch(`${this.baseUrl}/api/v1/sessions`);
    if (!res.ok) throw new Error(`Failed to list sessions (${res.status})`);
    return res.json();
  }

  async createSession(name?: string): Promise<Session> {
    const res = await fetch(`${this.baseUrl}/api/v1/sessions`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ name }),
    });
    if (!res.ok) throw new Error(`Failed to create session (${res.status})`);
    return res.json();
  }

  async loadSession(id: string): Promise<Session> {
    const res = await fetch(`${this.baseUrl}/api/v1/sessions/${encodeURIComponent(id)}`);
    if (!res.ok) throw new Error(`Failed to load session (${res.status})`);
    return res.json();
  }

  async saveSession(
    id: string,
    name: string,
    messages: ChatMessage[],
  ): Promise<void> {
    const res = await fetch(
      `${this.baseUrl}/api/v1/sessions/${encodeURIComponent(id)}`,
      {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ name, messages }),
      },
    );
    if (!res.ok) throw new Error(`Failed to save session (${res.status})`);
  }

  async deleteSession(id: string): Promise<void> {
    const res = await fetch(
      `${this.baseUrl}/api/v1/sessions/${encodeURIComponent(id)}`,
      { method: "DELETE" },
    );
    if (!res.ok) throw new Error(`Failed to delete session (${res.status})`);
  }

  async searchSessions(query: string): Promise<SessionInfo[]> {
    const res = await fetch(
      `${this.baseUrl}/api/v1/sessions/search?q=${encodeURIComponent(query)}`,
    );
    if (!res.ok) return [];
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
