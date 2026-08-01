import { afterEach, describe, expect, it, vi } from "vitest";
import { DaemonClient } from "./client";

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
