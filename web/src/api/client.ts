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
  CheckpointInfo,
  ConfigResponse,
  CreateSessionRequest,
  ExecuteToolRequest,
  ExecuteToolResponse,
  GetTodosResponse,
  HealthResponse,
  ListModelsResponse,
  ListTasksResponse,
  MemoryListQuery,
  MemoryListResponse,
  MemoryItem,
  MemoryStatus,
  PruneResult,
  ProjectInfo,
  RunResponse,
  SessionInfo,
  SessionResponse,
  SkillInfoDto,
  SwitchModelRequest,
  SwitchModelResponse,
  TaskProgressResponse,
  UndoTurnResult,
  WorktreeBinding,
  WorktreeInfo,
  DirListing,
  PermissionMode,
  PermissionModeResponse,
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
  // 204 No Content carries no JSON body. 201 may (POST /projects returns the
  // created ProjectInfo) or may not (POST /worktrees) — parse what is there.
  if (res.status === 204) return undefined as T;
  if (res.status === 201) {
    const text = await res.text();
    return (text ? JSON.parse(text) : undefined) as T;
  }
  return (await res.json()) as T;
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

  /**
   * `session_id` is required daemon-side (server-side paths reject a missing
   * one with 400). Callers pass the active session's daemon id, falling back
   * to the local id — the daemon resolves unknown ids to the main working
   * root, matching the pre-change `"default"` fallback.
   */
  async getPermissionMode(sessionId: string): Promise<PermissionModeResponse> {
    return jsonOrThrow(
      await fetch(`${this.base}/permission-mode?session_id=${encodeURIComponent(sessionId)}`),
    );
  }

  async setPermissionMode(
    sessionId: string,
    mode: PermissionMode,
  ): Promise<PermissionModeResponse> {
    return jsonOrThrow(
      await fetch(`${this.base}/permission-mode`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ mode, session_id: sessionId }),
      }),
    );
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

  /** Resolve a subagent async permission (from the trace SSE push channel). */
  async resolveSubagentPermission(
    requestId: string,
    approved: boolean,
    always = false,
  ): Promise<void> {
    await jsonOrThrow(
      await fetch(`${this.base}/tools/resolve-permission`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ request_id: requestId, approved, always }),
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

  async deleteSession(id: string): Promise<void> {
    await jsonOrThrow(
      await fetch(`${this.base}/sessions/${encodeURIComponent(id)}`, { method: "DELETE" }),
    );
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

  // ── Memory (Tier 2 ops-panel-api) ──────────────────────────────────────────

  async memoryStatus(): Promise<MemoryStatus> {
    return jsonOrThrow(await fetch(`${this.base}/memory/status`));
  }

  async listMemory(query: MemoryListQuery = {}): Promise<MemoryListResponse> {
    const params = new URLSearchParams();
    if (query.scope) params.set("scope", query.scope);
    if (query.min_importance !== undefined)
      params.set("min_importance", String(query.min_importance));
    if (query.limit !== undefined) params.set("limit", String(query.limit));
    const qs = params.toString();
    return jsonOrThrow(await fetch(`${this.base}/memory${qs ? `?${qs}` : ""}`));
  }

  async getMemory(id: string): Promise<MemoryItem> {
    return jsonOrThrow(await fetch(`${this.base}/memory/${encodeURIComponent(id)}`));
  }

  async pruneMemory(dryRun = false): Promise<PruneResult> {
    return jsonOrThrow(
      await fetch(`${this.base}/memory/prune`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ dry_run: dryRun }),
      }),
    );
  }

  // ── Trace SSE (subagent progress + permission events) ──────────────────────

  /**
   * Open the global trace SSE stream (`GET /subagents/trace/stream`). Returns
   * the raw byte stream; the caller parses newline-delimited JSON TraceEvents.
   *
   * This is the push channel for subagent permission prompts (design D2.1):
   * instead of polling /tools/pending-permissions, the frontend subscribes here
   * and dispatches on `event.kind`.
   */
  async traceStream(): Promise<{ body: ReadableStream<Uint8Array> }> {
    const res = await fetch(`${this.base}/subagents/trace/stream`);
    if (!res.ok || !res.body) {
      const text = await res.text().catch(() => "");
      throw new DaemonError(text || `${res.status} ${res.statusText}`, res.status);
    }
    return { body: res.body };
  }

  // ── Command center: projects / worktrees / skills / checkpoints ────────────

  async listProjects(): Promise<ProjectInfo[]> {
    return jsonOrThrow(await fetch(`${this.base}/projects`));
  }

  async addProject(path: string): Promise<ProjectInfo> {
    return jsonOrThrow(
      await fetch(`${this.base}/projects`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ path }),
      }),
    );
  }

  /** Unregister a project (registry only — files on disk are untouched). */
  async removeProject(path: string): Promise<void> {
    await jsonOrThrow(
      await fetch(`${this.base}/projects?path=${encodeURIComponent(path)}`, {
        method: "DELETE",
      }),
    );
  }

  /** List sub-directories of a path for the web directory picker.
   *  Omit `path` to list the user home directory. Read-only, daemon-side. */
  async listDirs(path?: string): Promise<DirListing> {
    const qs = path ? `?path=${encodeURIComponent(path)}` : "";
    return jsonOrThrow(await fetch(`${this.base}/fs/dirs${qs}`));
  }
  async listWorktrees(project?: string): Promise<WorktreeInfo[]> {
    const qs = project ? `?project=${encodeURIComponent(project)}` : "";
    return jsonOrThrow(await fetch(`${this.base}/worktrees${qs}`));
  }

  async createWorktree(req: { path: string; branch: string; project?: string }): Promise<void> {
    await jsonOrThrow(
      await fetch(`${this.base}/worktrees`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(req),
      }),
    );
  }

  async deleteWorktree(path: string, project?: string): Promise<void> {
    const params = new URLSearchParams({ path });
    if (project) params.set("project", project);
    await jsonOrThrow(
      await fetch(`${this.base}/worktrees?${params.toString()}`, { method: "DELETE" }),
    );
  }

  async listSkills(): Promise<SkillInfoDto[]> {
    return jsonOrThrow(await fetch(`${this.base}/skills`));
  }

  async listCheckpoints(): Promise<CheckpointInfo[]> {
    return jsonOrThrow(await fetch(`${this.base}/checkpoints`));
  }

  async undoTurns(turnIds: string[]): Promise<UndoTurnResult> {
    return jsonOrThrow(
      await fetch(`${this.base}/tools/undo-turn`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ turn_ids: turnIds }),
      }),
    );
  }

  // ── Worktree binding / archive (project v1) ────────────────────────────────

  async bindWorktree(sessionId: string, req: WorktreeBinding): Promise<void> {
    await jsonOrThrow(
      await fetch(`${this.base}/sessions/${encodeURIComponent(sessionId)}/worktree`, {
        method: "PUT",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(req),
      }),
    );
  }

  async unbindWorktree(sessionId: string): Promise<void> {
    await jsonOrThrow(
      await fetch(`${this.base}/sessions/${encodeURIComponent(sessionId)}/worktree`, {
        method: "DELETE",
      }),
    );
  }

  async setSessionArchived(sessionId: string, archived: boolean): Promise<void> {
    await jsonOrThrow(
      await fetch(`${this.base}/sessions/${encodeURIComponent(sessionId)}/archive`, {
        method: "PUT",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ archived }),
      }),
    );
  }

  // ── Server-side run (web as observer) ──────────────────────────────────────

  /** POST /sessions/:id/run — daemon spawns the turn; returns immediately. */
  async runSession(sessionId: string, message: string): Promise<RunResponse> {
    return jsonOrThrow(
      await fetch(`${this.base}/sessions/${encodeURIComponent(sessionId)}/run`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ message }),
      }),
    );
  }

  /** POST /sessions/:id/cancel — cancel an active run. */
  async cancelRun(sessionId: string): Promise<void> {
    await jsonOrThrow(
      await fetch(`${this.base}/sessions/${encodeURIComponent(sessionId)}/cancel`, {
        method: "POST",
      }),
    );
  }

  /** GET /sessions/:id/events — SSE stream of SessionEvent from a server-side run. */
  async sessionEvents(sessionId: string): Promise<{ body: ReadableStream<Uint8Array> }> {
    const res = await fetch(`${this.base}/sessions/${encodeURIComponent(sessionId)}/events`);
    if (!res.ok || !res.body) {
      const text = await res.text().catch(() => "");
      throw new DaemonError(text || `${res.status} ${res.statusText}`, res.status);
    }
    return { body: res.body };
  }

  /** POST /interactions/:id/resolve — answer a pending ask_user_question. */
  async resolveInteraction(requestId: string, answer: string): Promise<void> {
    await jsonOrThrow(
      await fetch(`${this.base}/interactions/${encodeURIComponent(requestId)}/resolve`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ answer }),
      }),
    );
  }
}
