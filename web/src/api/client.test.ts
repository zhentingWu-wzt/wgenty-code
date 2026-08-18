import { afterEach, describe, expect, it, vi } from "vitest";
import { DaemonClient, resolveDaemonDirect } from "./client";

function mockFetch(payload: unknown, status = 200) {
  const spy = vi.fn().mockResolvedValue(
    new Response(JSON.stringify(payload), {
      status,
      headers: { "content-type": "application/json" },
    }),
  );
  vi.stubGlobal("fetch", spy);
  return spy;
}

describe("DaemonClient command-center endpoints", () => {
  afterEach(() => vi.unstubAllGlobals());
  const client = new DaemonClient();

  it("listWorktrees GETs /worktrees", async () => {
    const spy = mockFetch([{ path: "/repo", head: "abc", branch: "main", is_main: true }]);
    const wt = await client.listWorktrees();
    expect(spy).toHaveBeenCalledWith("/api/v1/worktrees");
    expect(wt[0].is_main).toBe(true);
  });

  it("createWorktree POSTs path+branch", async () => {
    const spy = mockFetch(null, 201);
    await client.createWorktree({ path: "/repo/.worktrees/f", branch: "f" });
    const [url, init] = spy.mock.calls[0];
    expect(url).toBe("/api/v1/worktrees");
    expect(init.method).toBe("POST");
    expect(JSON.parse(init.body)).toEqual({ path: "/repo/.worktrees/f", branch: "f" });
  });

  it("deleteWorktree DELETEs with ?path= query", async () => {
    const spy = mockFetch(undefined, 204);
    await client.deleteWorktree("/repo/.worktrees/f");
    const [url, init] = spy.mock.calls[0];
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
    expect(JSON.parse(spy.mock.calls[0][1].body)).toEqual({ turn_ids: ["t2", "t3"] });
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
    expect(spy.mock.calls[0][0]).toBe("/api/v1/permission-mode?session_id=s1");
    expect(res.mode).toBe("normal");
  });

  it("setPermissionMode POSTs mode + session_id", async () => {
    const spy = mockFetch({ success: true, mode: "yolo", effective_mode: "yolo" });
    await client.setPermissionMode("s1", "yolo");
    const [url, init] = spy.mock.calls[0];
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
    const [url, init] = spy.mock.calls[0];
    expect(url).toBe("/api/v1/sessions/s1/worktree");
    expect(init.method).toBe("PUT");
    expect(JSON.parse(init.body as string)).toEqual({ path: ".worktrees/a", branch: "a" });
  });

  it("unbindWorktree DELETEs the binding", async () => {
    const spy = mockFetch(undefined, 204);
    await client.unbindWorktree("s1");
    expect(spy.mock.calls[0][0]).toBe("/api/v1/sessions/s1/worktree");
    expect(spy.mock.calls[0][1].method).toBe("DELETE");
  });

  it("setSessionArchived PUTs the flag", async () => {
    const spy = mockFetch({ session_id: "s1", archived: true });
    await client.setSessionArchived("s1", true);
    expect(spy.mock.calls[0][0]).toBe("/api/v1/sessions/s1/archive");
    expect(JSON.parse(spy.mock.calls[0][1].body as string)).toEqual({ archived: true });
  });
});

function jsonResponse(payload: unknown, status = 200) {
  return new Response(JSON.stringify(payload), {
    status,
    headers: { "content-type": "application/json" },
  });
}

/** Route-based fetch stub: each URL maps to a handler; unmapped URLs throw. */
function mockFetchRouter(handlers: Record<string, () => Response>) {
  const spy = vi.fn(async (input: RequestInfo | URL) => {
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
