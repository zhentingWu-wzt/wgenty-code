import { afterEach, describe, expect, it, vi } from "vitest";
import { DaemonClient, resolveDaemonDirect } from "./client";

function mockFetch(payload: unknown, status = 200) {
  const spy = vi.fn(async (input: RequestInfo | URL) => {
    // authedFetch 会先探 /__daemon-info：404 让 resolveDaemonDirect 走 DEV→null，
    // 保证后续断言里 calls[0] 仍是 API 调用本身。
    if (String(input) === "/__daemon-info") return new Response("not found", { status: 404 });
    // 每次调用返回新 Response——共享实例会在 daemon-info 探测消费 body 后不可再读。
    return new Response(JSON.stringify(payload), {
      status,
      headers: { "content-type": "application/json" },
    });
  });
  vi.stubGlobal("fetch", spy);
  return spy;
}

describe("DaemonClient command-center endpoints", () => {
  afterEach(() => vi.unstubAllGlobals());
  const client = new DaemonClient();

  it("listWorktrees GETs /worktrees", async () => {
    const spy = mockFetch([{ path: "/repo", head: "abc", branch: "main", is_main: true }]);
    const wt = await client.listWorktrees();
    expect(apiCall(spy)[0]).toBe("/api/v1/worktrees");
    expect(wt[0].is_main).toBe(true);
  });

  it("createWorktree POSTs path+branch", async () => {
    const spy = mockFetch(null, 201);
    await client.createWorktree({ path: "/repo/.worktrees/f", branch: "f" });
    const [url, init] = apiCall(spy);
    expect(url).toBe("/api/v1/worktrees");
    expect(init.method).toBe("POST");
    expect(JSON.parse(init.body as string)).toEqual({ path: "/repo/.worktrees/f", branch: "f" });
  });

  it("deleteWorktree DELETEs with ?path= query", async () => {
    const spy = mockFetch(undefined, 204);
    await client.deleteWorktree("/repo/.worktrees/f");
    const [url, init] = apiCall(spy);
    expect(url).toBe(`/api/v1/worktrees?path=${encodeURIComponent("/repo/.worktrees/f")}`);
    expect(init.method).toBe("DELETE");
  });

  it("listSkills GETs /skills", async () => {
    mockFetch([{ name: "alpha", description: "d", source_path: "/x/SKILL.md" }]);
    const skills = await client.listSkills();
    expect(skills[0].name).toBe("alpha");
  });

  it("listCheckpoints GETs /checkpoints", async () => {
    mockFetch([{ turn_id: "t1", created_at: 123, file_count: 2 }]);
    const cps = await client.listCheckpoints();
    expect(cps[0].turn_id).toBe("t1");
  });

  it("undoTurns POSTs turn_ids", async () => {
    const spy = mockFetch({ restored: 1, skipped: 0, failed: 0, rewound_turns: 1 });
    const res = await client.undoTurns(["t2", "t3"]);
    expect(JSON.parse(apiCall(spy)[1].body as string)).toEqual({ turn_ids: ["t2", "t3"] });
    expect(res.restored).toBe(1);
  });
});

describe("DaemonClient permission-mode endpoints", () => {
  afterEach(() => vi.unstubAllGlobals());
  const client = new DaemonClient();

  // The daemon rejects a missing session_id with 400 — both calls must carry it.
  it("getPermissionMode GETs with ?session_id= query", async () => {
    const spy = mockFetch({ mode: "normal", effective_mode: "normal" });
    const res = await client.getPermissionMode("s1");
    expect(apiCall(spy)[0]).toBe("/api/v1/permission-mode?session_id=s1");
    expect(res.mode).toBe("normal");
  });

  it("setPermissionMode POSTs mode + session_id", async () => {
    const spy = mockFetch({ success: true, mode: "yolo", effective_mode: "yolo" });
    await client.setPermissionMode("s1", "yolo");
    const [url, init] = apiCall(spy);
    expect(url).toBe("/api/v1/permission-mode");
    expect(init.method).toBe("POST");
    expect(JSON.parse(init.body as string)).toEqual({ mode: "yolo", session_id: "s1" });
  });
});

describe("DaemonClient worktree binding + archive", () => {
  afterEach(() => vi.unstubAllGlobals());
  const client = new DaemonClient();

  it("bindWorktree PUTs the binding", async () => {
    const spy = mockFetch({ session_id: "s1", worktree: { path: "/r/.worktrees/a", branch: "a" } });
    await client.bindWorktree("s1", { path: ".worktrees/a", branch: "a" });
    const [url, init] = apiCall(spy);
    expect(url).toBe("/api/v1/sessions/s1/worktree");
    expect(init.method).toBe("PUT");
    expect(JSON.parse(init.body as string)).toEqual({ path: ".worktrees/a", branch: "a" });
  });

  it("unbindWorktree DELETEs the binding", async () => {
    const spy = mockFetch(undefined, 204);
    await client.unbindWorktree("s1");
    expect(apiCall(spy)[0]).toBe("/api/v1/sessions/s1/worktree");
    expect(apiCall(spy)[1].method).toBe("DELETE");
  });

  it("setSessionArchived PUTs the flag", async () => {
    const spy = mockFetch({ session_id: "s1", archived: true });
    await client.setSessionArchived("s1", true);
    expect(apiCall(spy)[0]).toBe("/api/v1/sessions/s1/archive");
    expect(JSON.parse(apiCall(spy)[1].body as string)).toEqual({ archived: true });
  });
});

/** 第一个非 /__daemon-info 探测的调用——authedFetch 每次 API 调用前先探测一次，
 *  因此原有基于 calls[0] 的断言一律经由本辅助取 API 调用本身。 */
function apiCall(spy: { mock: { calls: unknown[][] } }) {
  const call = spy.mock.calls.find((c) => String(c[0]) !== "/__daemon-info");
  expect(call, "expected an API fetch call").toBeTruthy();
  return call as [string, { method?: string; body?: string }];
}

function jsonResponse(payload: unknown, status = 200) {
  return new Response(JSON.stringify(payload), {
    status,
    headers: { "content-type": "application/json" },
  });
}

/** Route-based fetch stub: each URL maps to a handler; unmapped URLs throw. */
function mockFetchRouter(handlers: Record<string, () => Response>) {
  const spy = vi.fn(async (input: RequestInfo | URL, _init?: RequestInit) => {
    const handler = handlers[String(input)];
    if (!handler) throw new Error(`unexpected fetch: ${String(input)}`);
    return handler();
  });
  vi.stubGlobal("fetch", spy);
  return spy;
}

describe("resolveDaemonDirect fallback chain", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.unstubAllEnvs();
  });

  it("falls back to same-origin /auth/bootstrap when /__daemon-info is unavailable", async () => {
    vi.stubEnv("DEV", false);
    mockFetchRouter({
      "/__daemon-info": () => jsonResponse({ error: "not found" }, 404),
      "/auth/bootstrap": () => jsonResponse({ token: "tok-bootstrap" }),
    });
    const info = await resolveDaemonDirect();
    expect(info).toEqual({ base: `${location.origin}/api/v1`, token: "tok-bootstrap" });
  });

  it("falls back to /auth/bootstrap when the /__daemon-info fetch throws", async () => {
    vi.stubEnv("DEV", false);
    mockFetchRouter({
      "/__daemon-info": () => {
        throw new Error("connection refused");
      },
      "/auth/bootstrap": () => jsonResponse({ token: "tok-bootstrap" }),
    });
    const info = await resolveDaemonDirect();
    expect(info).toEqual({ base: `${location.origin}/api/v1`, token: "tok-bootstrap" });
  });

  it("returns null when bootstrap responds 404 (old daemon)", async () => {
    vi.stubEnv("DEV", false);
    mockFetchRouter({
      "/__daemon-info": () => jsonResponse({}, 404),
      "/auth/bootstrap": () => jsonResponse({}, 404),
    });
    expect(await resolveDaemonDirect()).toBeNull();
  });

  it("returns null when the bootstrap fetch throws", async () => {
    vi.stubEnv("DEV", false);
    mockFetchRouter({
      "/__daemon-info": () => jsonResponse({}, 404),
      "/auth/bootstrap": () => {
        throw new Error("network down");
      },
    });
    expect(await resolveDaemonDirect()).toBeNull();
  });

  it("returns null when the bootstrap payload has no token", async () => {
    vi.stubEnv("DEV", false);
    mockFetchRouter({
      "/__daemon-info": () => jsonResponse({}, 404),
      "/auth/bootstrap": () => jsonResponse({}),
    });
    expect(await resolveDaemonDirect()).toBeNull();
  });

  it("does not attempt bootstrap in vite dev mode", async () => {
    vi.stubEnv("DEV", true);
    const spy = mockFetchRouter({
      "/__daemon-info": () => jsonResponse({}, 404),
      "/auth/bootstrap": () => jsonResponse({ token: "tok-bootstrap" }),
    });
    expect(await resolveDaemonDirect()).toBeNull();
    expect(spy.mock.calls.map((c) => c[0])).not.toContain("/auth/bootstrap");
  });

  it("still prefers /__daemon-info when it succeeds", async () => {
    vi.stubEnv("DEV", false);
    const spy = mockFetchRouter({
      "/__daemon-info": () => jsonResponse({ port: 8371, token: "tok-info" }),
      "/auth/bootstrap": () => jsonResponse({ token: "tok-bootstrap" }),
    });
    expect(await resolveDaemonDirect()).toEqual({
      base: "http://127.0.0.1:8371/api/v1",
      token: "tok-info",
    });
    expect(spy.mock.calls.map((c) => c[0])).not.toContain("/auth/bootstrap");
  });
});

describe("DaemonClient authedFetch Authorization injection", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.unstubAllEnvs();
  });

  /** Hosted 模式（无 vite 中间件）：__daemon-info 404 → bootstrap 供给 token。 */
  function hostedRouter(token: string, extra: Record<string, () => Response>) {
    return mockFetchRouter({
      "/__daemon-info": () => jsonResponse({ error: "not found" }, 404),
      "/auth/bootstrap": () => jsonResponse({ token }),
      ...extra,
    });
  }

  /** 在捕获的 fetch 调用中找指定 URL，返回其 Authorization 头（无则 null）。 */
  function authHeaderFor(spy: ReturnType<typeof mockFetchRouter>, url: string): string | null {
    const call = spy.mock.calls.find((c) => c[0] === url);
    expect(call, `expected a fetch to ${url}`).toBeTruthy();
    return new Headers(call?.[1]?.headers).get("authorization");
  }

  it("hosted 模式：health() 注入 Authorization: Bearer <token>", async () => {
    vi.stubEnv("DEV", false);
    const spy = hostedRouter("tok-hosted", {
      "/api/v1/health": () => jsonResponse({ status: "ok" }),
    });
    await new DaemonClient().health();
    expect(authHeaderFor(spy, "/api/v1/health")).toBe("Bearer tok-hosted");
  });

  it("hosted 模式：ensureViewer() 的 POST /ui/viewers 同样注入 Authorization 头", async () => {
    vi.stubEnv("DEV", false);
    const spy = hostedRouter("tok-hosted", {
      "/api/v1/ui/viewers": () => jsonResponse({ viewer_token: "vt-1" }),
    });
    await new DaemonClient().ensureViewer();
    expect(authHeaderFor(spy, "/api/v1/ui/viewers")).toBe("Bearer tok-hosted");
  });

  it("hosted 模式：既有头（viewer token）与注入的 Authorization 合并共存，不丢头", async () => {
    vi.stubEnv("DEV", false);
    const url = "/api/v1/agents/self?session_id=s1";
    const spy = hostedRouter("tok-hosted", {
      "/api/v1/ui/viewers": () => jsonResponse({ viewer_token: "vt-1" }),
      [url]: () => jsonResponse({}),
    });
    await new DaemonClient().getAgentSelf("s1");
    const call = spy.mock.calls.find((c) => c[0] === url);
    expect(call, `expected a fetch to ${url}`).toBeTruthy();
    const headers = new Headers(call?.[1]?.headers);
    expect(headers.get("authorization")).toBe("Bearer tok-hosted");
    expect(headers.get("x-wgenty-viewer-token")).toBe("vt-1");
  });

  it("resolveDaemonDirect 为 null 时不注入 Authorization，也不抛错", async () => {
    // dev 模式：__daemon-info 404 → resolveDaemonDirect 直接返回 null。
    vi.stubEnv("DEV", true);
    const spy = mockFetchRouter({
      "/__daemon-info": () => jsonResponse({ error: "not found" }, 404),
      "/api/v1/health": () => jsonResponse({ status: "ok" }),
    });
    await new DaemonClient().health();
    expect(authHeaderFor(spy, "/api/v1/health")).toBeNull();
  });
});

describe("DaemonClient authedFetch 零行为变化", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.unstubAllEnvs();
  });

  it("dev 形态（__daemon-info 200）：API 调用不再请求 bootstrap，头取自 daemon-info", async () => {
    vi.stubEnv("DEV", true);
    const spy = mockFetchRouter({
      "/__daemon-info": () => jsonResponse({ port: 8371, token: "tok-info" }),
      "/api/v1/health": () => jsonResponse({ status: "ok" }),
    });
    await new DaemonClient().health();
    const call = spy.mock.calls.find((c) => c[0] === "/api/v1/health");
    expect(call, "expected a fetch to /api/v1/health").toBeTruthy();
    expect(new Headers(call?.[1]?.headers).get("authorization")).toBe("Bearer tok-info");
    expect(spy.mock.calls.map((c) => c[0])).not.toContain("/auth/bootstrap");
  });

  it("旧 daemon（两端点都 404）：API 调用不注入 Authorization，也不抛错", async () => {
    vi.stubEnv("DEV", false);
    const spy = mockFetchRouter({
      "/__daemon-info": () => jsonResponse({ error: "not found" }, 404),
      "/auth/bootstrap": () => jsonResponse({ error: "not found" }, 404),
      "/api/v1/health": () => jsonResponse({ status: "ok" }),
    });
    const res = await new DaemonClient().health();
    expect(res.status).toBe("ok");
    const call = spy.mock.calls.find((c) => c[0] === "/api/v1/health");
    expect(call, "expected a fetch to /api/v1/health").toBeTruthy();
    expect(new Headers(call?.[1]?.headers).get("authorization")).toBeNull();
  });
});
