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
  AddMcpServerRequest,
  AgentDirectoryResponse,
  CheckpointInfo,
  ConfigResponse,
  CreateSessionRequest,
  CreateViewerResponse,
  ExecuteToolRequest,
  ExecuteToolResponse,
  GetTodosResponse,
  HealthResponse,
  ListModelsResponse,
  ListPendingPermissionsResponse,
  ListTasksResponse,
  LocalAgentViewResponse,
  McpServerInfo,
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
  TraceEvent,
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

/** Direct daemon connection info served by the vite dev middleware. */
interface DaemonDirectInfo {
  base: string;
  token: string;
}

/**
 * Long-lived SSE streams go DIRECTLY to the daemon origin instead of through
 * the page's origin: browsers cap HTTP/1.1 at ~6 connections per origin, and
 * every permanent stream held through the vite proxy (HMR websocket,
 * heartbeat, trace SSE, per-turn session SSE) consumes one — two app tabs
 * alone exceed the budget and every later fetch (send, stop, health) queues
 * forever, wedging the page. Direct streams get their own per-origin budget
 * on 127.0.0.1:<port>. Resolved fresh per call so a daemon restart (new
 * token/port) is picked up on reconnect. Returns null outside the vite dev
 * server (e.g. desktop shell) → callers fall back to the same-origin proxy.
 */
async function resolveDaemonDirect(): Promise<DaemonDirectInfo | null> {
  try {
    const res = await fetch("/__daemon-info");
    if (!res.ok) return null;
    const info = (await res.json()) as { port?: number; token?: string };
    if (typeof info.port !== "number" || !info.token) return null;
    return { base: `http://127.0.0.1:${info.port}/api/v1`, token: info.token };
  } catch {
    return null;
  }
}

export class DaemonClient {
  /**
   * Construct a client. `base` defaults to `/api/v1` (the Vite proxy prefix),
   * which works in dev. For production deployments, point `base` at the real
   * daemon origin.
   */
  constructor(private readonly base = "/api/v1") {}

  // ── Trusted UI viewer (scoped-agent endpoints) ─────────────────────────────

  /** Cached viewer bearer token (`POST /ui/viewers`). Scoped-agent endpoints
   *  require it in the `x-wgenty-viewer-token` header. Created lazily on first
   *  use; concurrent callers share a single in-flight request. */
  private viewerToken: string | null = null;
  private viewerPromise: Promise<string> | null = null;

  async ensureViewer(): Promise<string> {
    if (this.viewerToken) return this.viewerToken;
    if (!this.viewerPromise) {
      this.viewerPromise = (async () => {
        const res = await fetch(`${this.base}/ui/viewers`, { method: "POST" });
        const r = await jsonOrThrow<CreateViewerResponse>(res);
        this.viewerToken = r.viewer_token;
        return r.viewer_token;
      })();
      // Clear the in-flight promise on settle so a failure can be retried;
      // viewerToken (set only on success) survives for subsequent calls.
      // The `.catch` only handles the promise *derived* from `.finally` —
      // the original's rejection stays with its caller; without it, a failed
      // viewer bootstrap (daemon unreachable) surfaces as an unhandled
      // rejection (e.g. it fails the vitest run).
      this.viewerPromise
        .finally(() => {
          this.viewerPromise = null;
        })
        .catch(() => {});
    }
    return this.viewerPromise;
  }

  /** Headers carrying the viewer token, awaited once per request. */
  private async agentHeaders(): Promise<Record<string, string>> {
    const token = await this.ensureViewer();
    return { "x-wgenty-viewer-token": token };
  }

  // ── Health / config ────────────────────────────────────────────────────────

  async health(): Promise<HealthResponse> {
    return jsonOrThrow(await fetch(`${this.base}/health`));
  }

  async getConfig(): Promise<ConfigResponse> {
    return jsonOrThrow(await fetch(`${this.base}/config`));
  }

  /**
   * PUT /config — partial update of transport settings (max_tokens, timeout,
   * streaming, api_base). Only provided fields are written. Sensitive fields
   * (api_key) are never accepted or returned.
   */
  async updateConfig(
    patch: Partial<Pick<ConfigResponse, "max_tokens" | "timeout" | "streaming" | "api_base">>,
  ): Promise<ConfigResponse> {
    return jsonOrThrow(
      await fetch(`${this.base}/config`, {
        method: "PUT",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(patch),
      }),
    );
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

  /** Resolve a subagent async permission (from the trace SSE push channel).
   *  `sessionRule` lets "always allow" persist the standing rule server-side. */
  async resolveSubagentPermission(
    requestId: string,
    approved: boolean,
    always = false,
    sessionRule?: string,
  ): Promise<void> {
    await jsonOrThrow(
      await fetch(`${this.base}/tools/resolve-permission`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          request_id: requestId,
          approved,
          always,
          ...(sessionRule ? { session_rule: sessionRule } : {}),
        }),
      }),
    );
  }

  /** GET /tools/pending-permissions — policy-Ask waiters still blocked
   *  server-side. Used to re-surface prompts after a page refresh or a
   *  trace-stream reconnect (pending state otherwise lives only in memory). */
  async listPendingPermissions(): Promise<ListPendingPermissionsResponse> {
    return jsonOrThrow(await fetch(`${this.base}/tools/pending-permissions`));
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

  /**
   * Search saved sessions by keyword (matches session name and message content).
   * Daemon-side: GET /sessions/search?q=<query>. Returns sessions across all
   * registered project roots.
   */
  async searchSessions(query: string): Promise<SessionInfo[]> {
    const q = query.trim();
    return jsonOrThrow(await fetch(`${this.base}/sessions/search?q=${encodeURIComponent(q)}`));
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

  /**
   * DELETE /memory/:id — delete a single memory. Requires `origin` to select
   * the pool ("project" or "global").
   */
  async deleteMemory(id: string, origin: "project" | "global"): Promise<void> {
    const res = await fetch(`${this.base}/memory/${encodeURIComponent(id)}?origin=${origin}`, {
      method: "DELETE",
    });
    if (!res.ok) {
      const body = await res.text();
      throw new DaemonError(body || `${res.status} ${res.statusText}`, res.status);
    }
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

  /** Fetch a streaming endpoint: direct to the daemon origin when the dev
   *  middleware offers connection info, else through the same-origin proxy.
   *  A 15s watchdog covers ONLY the connect/headers phase — once headers
   *  arrive it is disarmed and the body streams until it ends or the
   *  caller's signal aborts (a body-phase timeout would kill every SSE
   *  stream 15s in).
   *
   *  A watchdog timeout means the browser QUEUED the request behind a full
   *  per-origin budget (~6 HTTP/1.1 connections: trace + global-events +
   *  per-run session streams + duplicate app tabs), not that the daemon is
   *  down. Queued connects succeed as soon as any holder finishes, so one
   *  retry absorbs transient contention instead of surfacing a hard error. */
  private async fetchStream(path: string, signal?: AbortSignal): Promise<Response> {
    const maxAttempts = 2;
    for (let attempt = 1; ; attempt++) {
      const ctl = new AbortController();
      const onUserAbort = () => ctl.abort(signal!.reason);
      if (signal) {
        if (signal.aborted) ctl.abort(signal.reason);
        // Deliberately not removed after connect: the same controller also
        // governs body streaming, so the Stop button keeps working mid-stream.
        else signal.addEventListener("abort", onUserAbort, { once: true });
      }
      const watchdog = setTimeout(
        () => ctl.abort(new DOMException("stream connect timed out", "TimeoutError")),
        15_000,
      );
      try {
        const direct = await resolveDaemonDirect();
        if (direct) {
          try {
            return await fetch(`${direct.base}${path}`, {
              headers: { authorization: `Bearer ${direct.token}` },
              signal: ctl.signal,
            });
          } catch (err) {
            // Only fall back on a network-level failure (Chrome's TypeError
            // "Failed to fetch" — e.g. CORS / Private Network Access blocking
            // the cross-origin hop to 127.0.0.1). User stop and the connect
            // watchdog surface as aborts and must propagate, not be retried
            // through the proxy.
            if (ctl.signal.aborted || !(err instanceof TypeError)) throw err;
            console.warn(
              "[client] direct daemon stream blocked, falling back to same-origin proxy:",
              err,
            );
          }
        }
        return await fetch(`${this.base}${path}`, { signal: ctl.signal });
      } catch (err) {
        // Watchdog-fired (not user-aborted) connect timeout → retry once;
        // a slot usually frees as soon as a concurrent run/stream ends.
        const connectTimeout =
          err instanceof DOMException &&
          err.name === "TimeoutError" &&
          !signal?.aborted;
        if (!connectTimeout || attempt >= maxAttempts) throw err;
        console.warn(
          `[client] ${path}: SSE connect queued >15s ` +
            "(per-origin connection budget exhausted by concurrent streams/tabs?); retrying…",
        );
        // Drop this attempt's user-abort listener so retries don't stack
        // listeners on the caller's signal.
        signal?.removeEventListener("abort", onUserAbort);
      } finally {
        clearTimeout(watchdog);
      }
    }
  }

  /**
   * Open the trace SSE stream (`GET /subagents/trace/stream`, global live).
   * Returns the raw byte stream; the caller parses newline-delimited JSON
   * TraceEvents.
   *
   * This is the push channel for subagent permission prompts (design D2.1):
   * instead of polling /tools/pending-permissions, the frontend subscribes
   * here and dispatches on `event.kind`. Cold-start recovery of terminal
   * results uses the one-shot `traceReplay` below, not a second stream —
   * every permanent SSE connection counts against the browser's per-origin
   * connection budget.
   */
  async traceStream(
    sessionId?: string,
    since?: number,
    /** Aborts the underlying fetch. ESSENTIAL on hook cleanup: without it the
     *  keepalive-only stream never ends and the connection leaks from the
     *  browser's per-origin budget. */
    signal?: AbortSignal,
  ): Promise<{ body: ReadableStream<Uint8Array> }> {
    const params = new URLSearchParams();
    if (sessionId) params.set("session_id", sessionId);
    if (since !== undefined) params.set("since", String(since));
    const query = params.toString();
    const res = await this.fetchStream(
      `/subagents/trace/stream${query ? `?${query}` : ""}`,
      signal,
    );
    if (!res.ok || !res.body) {
      const text = await res.text().catch(() => "");
      throw new DaemonError(text || `${res.status} ${res.statusText}`, res.status);
    }
    return { body: res.body };
  }

  /** One-shot replay of a session's persisted trace headers (JSON, non-SSE).
   *  Replaces the old session-scoped trace stream for cold-start recovery. */
  async traceReplay(sessionId: string, since?: number): Promise<TraceEvent[]> {
    const params = new URLSearchParams({ session_id: sessionId });
    if (since !== undefined) params.set("since", String(since));
    return jsonOrThrow(await fetch(`${this.base}/subagents/trace/replay?${params}`));
  }

  // ── Scoped agent views (subagent local view + transcript + cancel) ─────────

  /** `GET /agents/self?session_id=<id>` -- root local view (self + direct
   *  children, each with a fresh navigation capability). */
  async getAgentSelf(sessionId: string): Promise<LocalAgentViewResponse> {
    const headers = await this.agentHeaders();
    return jsonOrThrow(
      await fetch(`${this.base}/agents/self?session_id=${encodeURIComponent(sessionId)}`, {
        headers,
      }),
    );
  }

  /** `GET /agents/directory?session_id=<id>` -- full recursive subagent tree
   *  for the session (root agent plus nested children, with depth). */
  async getAgentDirectory(sessionId: string): Promise<AgentDirectoryResponse> {
    const headers = await this.agentHeaders();
    return jsonOrThrow(
      await fetch(`${this.base}/agents/directory?session_id=${encodeURIComponent(sessionId)}`, {
        headers,
      }),
    );
  }

  /** `GET /agents/children/:capability?session_id=<id>` -- navigate one level
   *  into the child bound by `capability`; returns that child's local view. */
  async navigateAgentView(sessionId: string, capability: string): Promise<LocalAgentViewResponse> {
    const headers = await this.agentHeaders();
    return jsonOrThrow(
      await fetch(
        `${this.base}/agents/children/${encodeURIComponent(capability)}?session_id=${encodeURIComponent(sessionId)}`,
        { headers },
      ),
    );
  }

  /** `GET /agents/children/:capability/transcript?session_id=<id>` -- read the
   *  transcript of the direct child bound by `capability`. */
  async getChildTranscript(
    sessionId: string,
    capability: string,
  ): Promise<{ transcript: unknown }> {
    const headers = await this.agentHeaders();
    return jsonOrThrow(
      await fetch(
        `${this.base}/agents/children/${encodeURIComponent(capability)}/transcript?session_id=${encodeURIComponent(sessionId)}`,
        { headers },
      ),
    );
  }

  /** `POST /agents/children/:capability/cancel?session_id=<id>` -- cancel the
   *  direct child bound by `capability`. Returns true on 204. */
  async cancelChild(sessionId: string, capability: string): Promise<void> {
    const headers = await this.agentHeaders();
    const res = await fetch(
      `${this.base}/agents/children/${encodeURIComponent(capability)}/cancel?session_id=${encodeURIComponent(sessionId)}`,
      { method: "POST", headers },
    );
    if (!res.ok) {
      throw new DaemonError(
        (await res.text().catch(() => "")) || `${res.status} ${res.statusText}`,
        res.status,
      );
    }
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

  // ── MCP servers ────────────────────────────────────────────────────────────

  async listMcpServers(): Promise<McpServerInfo[]> {
    const res = await jsonOrThrow<{ servers: McpServerInfo[] }>(
      await fetch(`${this.base}/mcp/servers`),
    );
    return res.servers;
  }

  async addMcpServer(req: AddMcpServerRequest): Promise<void> {
    const res = await fetch(`${this.base}/mcp/servers`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(req),
    });
    if (!res.ok) throw new DaemonError(await res.text(), res.status);
  }

  async removeMcpServer(name: string): Promise<void> {
    const res = await fetch(`${this.base}/mcp/servers/${encodeURIComponent(name)}`, {
      method: "DELETE",
    });
    if (!res.ok) throw new DaemonError(await res.text(), res.status);
  }

  async startMcpServer(name: string): Promise<void> {
    const res = await fetch(`${this.base}/mcp/servers/${encodeURIComponent(name)}/start`, {
      method: "POST",
    });
    if (!res.ok) throw new DaemonError(await res.text(), res.status);
  }

  async stopMcpServer(name: string): Promise<void> {
    const res = await fetch(`${this.base}/mcp/servers/${encodeURIComponent(name)}/stop`, {
      method: "POST",
    });
    if (!res.ok) throw new DaemonError(await res.text(), res.status);
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

  /** POST /sessions/:id/run — daemon spawns the turn; returns immediately.
   *  A 15s watchdog merges with the caller's signal: a request starved by
   *  the per-origin connection budget must surface as an error, not hang. */
  async runSession(sessionId: string, message: string, signal?: AbortSignal): Promise<RunResponse> {
    return jsonOrThrow(
      await fetch(`${this.base}/sessions/${encodeURIComponent(sessionId)}/run`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ message }),
        signal: signal
          ? AbortSignal.any([signal, AbortSignal.timeout(15_000)])
          : AbortSignal.timeout(15_000),
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

  /** GET /sessions/:id/events — SSE stream of SessionEvent from a server-side
   *  run. Without `after` the stream is live-only; with `after` the daemon
   *  replays buffered events with `seq > after` first (or sends sync_lost).
   *  `signal` aborts the underlying fetch (Stop button / connect watchdog).
   *  Long-lived: opened directly against the daemon origin when possible
   *  (see resolveDaemonDirect) so it doesn't consume the page origin's
   *  connection budget. */
  async sessionEvents(
    sessionId: string,
    after?: number,
    signal?: AbortSignal,
  ): Promise<{ body: ReadableStream<Uint8Array> }> {
    const query = after !== undefined ? `?after=${after}` : "";
    const res = await this.fetchStream(
      `/sessions/${encodeURIComponent(sessionId)}/events${query}`,
      signal,
    );
    if (!res.ok || !res.body) {
      const text = await res.text().catch(() => "");
      throw new DaemonError(text || `${res.status} ${res.statusText}`, res.status);
    }
    return { body: res.body };
  }

  /** GET /events — daemon-wide global SSE stream (todos changes, background
   *  results, task-group results, ...). Live-only and low-frequency; clients
   *  realign via the plain GET endpoints after any gap. Long-lived: opened
   *  directly against the daemon origin like sessionEvents. */
  async globalEvents(signal?: AbortSignal): Promise<{ body: ReadableStream<Uint8Array> }> {
    const res = await this.fetchStream(`/events`, signal);
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
