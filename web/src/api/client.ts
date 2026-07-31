/**
 * DaemonClient — HTTP thin client for the wgenty-code daemon.
 *
 * Mirrors `src/tui/client.rs` (`DaemonClient`) method-for-method for the MVP
 * surface. All requests go through the Vite dev proxy (`/api/*`), which injects
 * the bearer token server-side (see vite.config.ts) — so this code never
 * touches the secret.
 *
 * Why `fetch` + `response.body` for streaming instead of `EventSource`:
 * EventSource cannot send a POST body or custom headers, but the chat endpoint
 * needs both (POST + the proxied auth header). `fetch` with a streamed
 * `response.body` reader is the correct tool.
 */
import type {
  ChatMessage,
  ChatStreamRequest,
  ConfigResponse,
  CreateSessionRequest,
  ExecuteToolRequest,
  ExecuteToolResponse,
  GetTodosResponse,
  HealthResponse,
  ListModelsResponse,
  ListTasksResponse,
  SessionInfo,
  SessionResponse,
  SwitchModelRequest,
  SwitchModelResponse,
  TaskProgressResponse,
  UpdateSessionRequest,
} from "./types";

/** Error thrown when the daemon returns a non-2xx response. */
export class DaemonError extends Error {
  constructor(
    message: string,
    readonly status: number,
  ) {
    super(message);
    this.name = "DaemonError";
  }
}

async function jsonOrThrow<T>(res: Response): Promise<T> {
  if (!res.ok) {
    // Daemon returns plain text for some errors (e.g. 401 "unauthorized: ...").
    const body = await res.text();
    throw new DaemonError(body || `${res.status} ${res.statusText}`, res.status);
  }
  if (res.status === 204) return undefined as T;
  return (await res.json()) as T;
}

export interface ChatStreamOptions {
  /** Optional model override; if omitted the daemon uses its current setting. */
  model?: string;
  /** Plan mode: stream the assistant turn without tool execution hints. */
  planMode?: boolean;
}

export class DaemonClient {
  /**
   * Construct a client. `base` defaults to `/api/v1` (the Vite proxy prefix),
   * which works in dev. For production deployments, point `base` at the real
   * daemon origin.
   */
  constructor(private readonly base = "/api/v1") {}

  // ── Health / config ────────────────────────────────────────────────────────

  async health(): Promise<HealthResponse> {
    return jsonOrThrow(await fetch(`${this.base}/health`));
  }

  async getConfig(): Promise<ConfigResponse> {
    return jsonOrThrow(await fetch(`${this.base}/config`));
  }

  // ── Chat (streaming) ───────────────────────────────────────────────────────

  /**
   * Open a streaming chat turn. Returns the raw `ReadableStream<Uint8Array>`
   * from the SSE response — the caller feeds it into a `StreamProcessor`.
   *
   * Note: the caller MUST also read the `Response` to completion or abort it,
   * otherwise the underlying TCP connection stays open.
   */
  async chatStream(
    messages: ChatMessage[],
    opts: ChatStreamOptions = {},
    signal?: AbortSignal,
  ): Promise<{ body: ReadableStream<Uint8Array> }> {
    const body: ChatStreamRequest = {
      messages,
      ...(opts.model ? { model: opts.model } : {}),
      ...(opts.planMode !== undefined ? { plan_mode: opts.planMode } : {}),
    };
    const res = await fetch(`${this.base}/chat/stream`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
      // Passing the signal lets an in-flight stream be aborted mid-token (the
      // fetch aborts and res.body errors out), not just between rounds.
      signal,
    });
    if (!res.ok || !res.body) {
      const text = await res.text().catch(() => "");
      throw new DaemonError(text || `${res.status} ${res.statusText}`, res.status);
    }
    return { body: res.body };
  }

  // ── Tools ──────────────────────────────────────────────────────────────────

  async executeTool(req: ExecuteToolRequest): Promise<ExecuteToolResponse> {
    return jsonOrThrow(
      await fetch(`${this.base}/tools/execute`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(req),
      }),
    );
  }

  async approveTool(sessionRule: string): Promise<void> {
    await jsonOrThrow(
      await fetch(`${this.base}/tools/approve`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ session_rule: sessionRule }),
      }),
    );
  }

  async unapproveTool(sessionRule: string): Promise<void> {
    await jsonOrThrow(
      await fetch(`${this.base}/tools/unapprove`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ session_rule: sessionRule }),
      }),
    );
  }

  // ── Models ─────────────────────────────────────────────────────────────────

  async listModels(): Promise<ListModelsResponse> {
    return jsonOrThrow(await fetch(`${this.base}/models`));
  }

  async switchModel(req: SwitchModelRequest): Promise<SwitchModelResponse> {
    return jsonOrThrow(
      await fetch(`${this.base}/model/switch`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(req),
      }),
    );
  }

  // ── Sessions ───────────────────────────────────────────────────────────────

  async listSessions(): Promise<SessionInfo[]> {
    return jsonOrThrow(await fetch(`${this.base}/sessions`));
  }

  async createSession(req: CreateSessionRequest = {}): Promise<SessionResponse> {
    return jsonOrThrow(
      await fetch(`${this.base}/sessions`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(req),
      }),
    );
  }

  async loadSession(id: string): Promise<SessionResponse> {
    return jsonOrThrow(await fetch(`${this.base}/sessions/${encodeURIComponent(id)}`));
  }

  async saveSession(id: string, req: UpdateSessionRequest): Promise<SessionResponse> {
    return jsonOrThrow(
      await fetch(`${this.base}/sessions/${encodeURIComponent(id)}`, {
        method: "PUT",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(req),
      }),
    );
  }

  async deleteSession(id: string): Promise<void> {
    await jsonOrThrow(await fetch(`${this.base}/sessions/${encodeURIComponent(id)}`, { method: "DELETE" }));
  }

  // ── Todos / Tasks ──────────────────────────────────────────────────────────

  async getTodos(): Promise<GetTodosResponse> {
    return jsonOrThrow(await fetch(`${this.base}/todos`));
  }

  async listTasks(): Promise<ListTasksResponse> {
    return jsonOrThrow(await fetch(`${this.base}/tasks`));
  }

  async taskProgress(): Promise<TaskProgressResponse> {
    return jsonOrThrow(await fetch(`${this.base}/tasks/progress`));
  }
}
