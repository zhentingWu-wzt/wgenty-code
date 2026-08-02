import { afterEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { WorktreePanel } from "./WorktreePanel";
import { useSessionManager } from "../state/sessionManager";
import { waitFor } from "@testing-library/react";
import { SkillPanel } from "./SkillPanel";
import { DaemonClient } from "../api/client";

function mockFetch(payload: unknown, status = 200) {
  const spy = vi.fn().mockResolvedValue(new Response(JSON.stringify(payload), { status }));
  vi.stubGlobal("fetch", spy);
  return spy;
}

describe("WorktreePanel", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("lists worktrees with branch names", async () => {
    mockFetch([
      { path: "/repo", head: "a", branch: "main", is_main: true },
      { path: "/repo/.worktrees/f", head: "b", branch: "feat", is_main: false },
    ]);
    render(<WorktreePanel client={new DaemonClient()} />);
    // "main" appears twice (branch name + main tag); scope to the branch span.
    expect(await screen.findByText("main", { selector: ".wt-branch" })).toBeInTheDocument();
    expect(screen.getByText("feat")).toBeInTheDocument();
  });

  it("delete button calls the API and refreshes (main worktree has no delete)", async () => {
    const spy = vi.fn().mockImplementation((_url: string, init?: RequestInit) => {
      if (init?.method === "DELETE") return Promise.resolve(new Response(null, { status: 204 }));
      return Promise.resolve(
        new Response(
          JSON.stringify([{ path: "/repo", head: "a", branch: "main", is_main: true }]),
          { status: 200 },
        ),
      );
    });
    vi.stubGlobal("fetch", spy);
    render(<WorktreePanel client={new DaemonClient()} />);
    await screen.findByText("main", { selector: ".wt-branch" });
    expect(screen.queryByRole("button", { name: /remove/i })).not.toBeInTheDocument();
  });
});

describe("SkillPanel", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("is collapsed by default; expanding lists skills with descriptions", async () => {
    mockFetch([{ name: "brainstorming", description: "explore intent", source_path: "/x" }]);
    const user = userEvent.setup();
    render(<SkillPanel client={new DaemonClient()} />);

    // Collapsed by default — the skill row must not be visible yet.
    expect(screen.queryByText("brainstorming")).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /skills/i }));
    expect(await screen.findByText("brainstorming")).toBeInTheDocument();
    expect(screen.getByText("explore intent")).toBeInTheDocument();
  });
});

describe("WorktreePanel remove with bound sessions", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("unbinds bound sessions before deleting the worktree", async () => {
    const calls: Array<[string, string]> = [];
    const spy = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      const method = init?.method ?? "GET";
      calls.push([method, url]);
      const json = (p: unknown, status = 200) => new Response(JSON.stringify(p), { status });
      if (url === "/api/v1/worktrees" && method === "GET") {
        return json([
          { path: "/repo", head: "a", branch: "main", is_main: true },
          { path: "/repo/.worktrees/f", head: "b", branch: "feat", is_main: false },
        ]);
      }
      if (url === "/api/v1/sessions") {
        return json([
          {
            id: "s1",
            name: "bound chat",
            created_at: "x",
            updated_at: "x",
            message_count: 1,
            status: "Active",
            worktree: { path: "/repo/.worktrees/f", branch: "feat" },
          },
        ]);
      }
      if (url.endsWith("/worktree") && method === "DELETE")
        return new Response(null, { status: 204 });
      if (url.startsWith("/api/v1/worktrees?path=") && method === "DELETE") {
        return new Response(null, { status: 204 });
      }
      return new Response("not found", { status: 404 });
    });
    vi.stubGlobal("fetch", spy);
    vi.stubGlobal("confirm", vi.fn().mockReturnValue(true));
    useSessionManager.setState({
      entries: {},
      order: [],
      activeId: null,
      connection: "unknown",
      modelName: null,
    });

    const user = userEvent.setup();
    render(<WorktreePanel client={new DaemonClient()} />);
    await user.click(await screen.findByRole("button", { name: /remove/i }));

    await waitFor(() => {
      const unbindIdx = calls.findIndex(
        ([m, u]) => m === "DELETE" && u === "/api/v1/sessions/s1/worktree",
      );
      const removeIdx = calls.findIndex(
        ([m, u]) => m === "DELETE" && u.startsWith("/api/v1/worktrees?path="),
      );
      expect(unbindIdx).toBeGreaterThan(-1);
      expect(removeIdx).toBeGreaterThan(-1);
      expect(unbindIdx).toBeLessThan(removeIdx);
    });
    // The confirm copy mentions the unbind consequence.
    expect(vi.mocked(window.confirm).mock.calls[0][0]).toMatch(/unbound/);
  });
});
