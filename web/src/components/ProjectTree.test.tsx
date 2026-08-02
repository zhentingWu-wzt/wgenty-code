import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ProjectTree } from "./ProjectTree";
import { useSessionManager } from "../state/sessionManager";
import { DaemonClient } from "../api/client";

const client = new DaemonClient();

const WORKTREES = [
  { path: "/repo/wgenty-code", head: "a", branch: "main", is_main: true },
  { path: "/repo/wgenty-code/.worktrees/feat-x", head: "b", branch: "feat-x", is_main: false },
];

function stubFetch() {
  return vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    const method = init?.method ?? "GET";
    const json = (p: unknown, status = 200) => new Response(JSON.stringify(p), { status });
    if (url === "/api/v1/worktrees" && method === "GET") return json(WORKTREES);
    if (url === "/api/v1/worktrees" && method === "POST")
      return new Response(null, { status: 201 });
    if (url === "/api/v1/sessions" && method === "GET") return json([]);
    if (method === "PUT" || method === "DELETE") return new Response(null, { status: 204 });
    return new Response("not found", { status: 404 });
  });
}

function reset() {
  useSessionManager.setState({
    entries: {},
    order: [],
    activeId: null,
    connection: "unknown",
    modelName: null,
  });
}

describe("ProjectTree", () => {
  beforeEach(reset);
  afterEach(() => vi.unstubAllGlobals());

  it("renders project node, main checkout group, and bound sessions under their task", async () => {
    vi.stubGlobal("fetch", stubFetch());
    const m = useSessionManager.getState();
    m.createLocalSession("chat one");
    m.createLocalSession("task a", {
      id: "d1",
      daemonId: "d1",
      worktree: { path: ".worktrees/feat-x", branch: "feat-x" },
    });
    render(<ProjectTree client={client} />);

    // Project node from the main worktree path.
    expect(await screen.findByText("wgenty-code")).toBeInTheDocument();
    expect(screen.getByText("main checkout")).toBeInTheDocument();
    // Unbound session under main, bound session under the feat-x task node.
    expect(screen.getByText("chat one")).toBeInTheDocument();
    expect(screen.getByText("task a")).toBeInTheDocument();
    expect(screen.getByText("feat-x")).toBeInTheDocument();
  });

  it("+ Task creates a worktree via prompt", async () => {
    const spy = stubFetch();
    vi.stubGlobal("fetch", spy);
    vi.stubGlobal("prompt", vi.fn().mockReturnValue("feat-y"));
    const user = userEvent.setup();
    render(<ProjectTree client={client} />);

    await user.click(await screen.findByRole("button", { name: /task/i }));
    const mk = spy.mock.calls.find(
      ([u, i]) => String(u) === "/api/v1/worktrees" && i?.method === "POST",
    );
    expect(mk).toBeDefined();
    expect(JSON.parse(mk![1]!.body as string)).toEqual({
      path: ".worktrees/feat-y",
      branch: "feat-y",
    });
  });

  it("+ Session opens the new-session dialog", async () => {
    vi.stubGlobal("fetch", stubFetch());
    const user = userEvent.setup();
    render(<ProjectTree client={client} />);

    await user.click(await screen.findByRole("button", { name: /session/i }));
    expect(await screen.findByRole("dialog", { name: "New session" })).toBeInTheDocument();
  });

  it("archive button calls the API and closes the entry", async () => {
    const spy = stubFetch();
    vi.stubGlobal("fetch", spy);
    const m = useSessionManager.getState();
    m.createLocalSession("bound", {
      id: "d1",
      daemonId: "d1",
      worktree: { path: ".worktrees/feat-x", branch: "feat-x" },
    });
    render(<ProjectTree client={client} />);

    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: /archive bound/i }));
    await waitFor(() => {
      expect(
        spy.mock.calls.some(
          ([u, i]) => String(u) === "/api/v1/sessions/d1/archive" && i?.method === "PUT",
        ),
      ).toBe(true);
      expect(useSessionManager.getState().entries["d1"]).toBeUndefined();
    });
  });

  it("removing a task unbinds bound sessions before deleting the worktree", async () => {
    const calls: Array<[string, string]> = [];
    const spy = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      const method = init?.method ?? "GET";
      calls.push([method, url]);
      const json = (p: unknown, status = 200) => new Response(JSON.stringify(p), { status });
      if (url === "/api/v1/worktrees" && method === "GET") return json(WORKTREES);
      if (url === "/api/v1/sessions" && method === "GET") {
        return json([
          {
            id: "s1",
            name: "bound chat",
            created_at: "x",
            updated_at: "x",
            message_count: 1,
            status: "Active",
            worktree: { path: "/repo/wgenty-code/.worktrees/feat-x", branch: "feat-x" },
          },
        ]);
      }
      if (method === "DELETE") return new Response(null, { status: 204 });
      return new Response("not found", { status: 404 });
    });
    vi.stubGlobal("fetch", spy);
    vi.stubGlobal("confirm", vi.fn().mockReturnValue(true));

    const user = userEvent.setup();
    render(<ProjectTree client={client} />);
    await user.click(await screen.findByRole("button", { name: /remove worktree feat-x/i }));

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
    expect(vi.mocked(window.confirm).mock.calls[0][0]).toMatch(/unbound/);
  });
});
