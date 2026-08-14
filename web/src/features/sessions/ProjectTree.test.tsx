import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ProjectTree } from "./ProjectTree";
import { useSessionManager } from "../../state/sessionManager";
import { DaemonClient } from "../../api/client";

const client = new DaemonClient();

const PROJECTS = [
  {
    path: "/repo/wgenty-code",
    name: "wgenty-code",
    is_main: true,
    is_git_repo: true,
    added_at: "2026-08-01",
  },
];

const OTHER_PROJECT = {
  path: "/repo/docs",
  name: "docs",
  is_main: false,
  is_git_repo: false,
  added_at: "2026-08-02",
};

const WORKTREES = [
  { path: "/repo/wgenty-code", head: "a", branch: "main", is_main: true },
  { path: "/repo/wgenty-code/.worktrees/feat-x", head: "b", branch: "feat-x", is_main: false },
];

function stubFetch(projects: unknown[] = PROJECTS) {
  return vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    const method = init?.method ?? "GET";
    const json = (p: unknown, status = 200) => new Response(JSON.stringify(p), { status });
    if (url === "/api/v1/projects" && method === "GET") return json(projects);
    if (url.startsWith("/api/v1/worktrees") && method === "GET") return json(WORKTREES);
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

  it("+ Task creates a worktree via the dialog", async () => {
    const spy = stubFetch();
    vi.stubGlobal("fetch", spy);
    const user = userEvent.setup();
    render(<ProjectTree client={client} />);

    await user.click(await screen.findByRole("button", { name: /new task/i }));
    // The dialog replaced the old native prompt; fill the branch field.
    const dialog = await screen.findByRole("dialog", { name: "New task" });
    expect(dialog).toBeInTheDocument();
    await user.type(screen.getByPlaceholderText(/feature\/login/i), "feat-y");
    await user.click(screen.getByRole("button", { name: "Create" }));

    const mk = await waitFor(() =>
      spy.mock.calls.find(
        ([u, i]) => String(u) === "/api/v1/worktrees" && i?.method === "POST",
      ),
    );
    expect(JSON.parse(mk![1]!.body as string)).toEqual({
      path: ".worktrees/feat-y",
      branch: "feat-y",
      project: "/repo/wgenty-code",
    });
  });

  it("+ session on the main checkout node opens the dialog preset to main", async () => {
    vi.stubGlobal("fetch", stubFetch());
    const user = userEvent.setup();
    render(<ProjectTree client={client} />);

    await user.click(await screen.findByRole("button", { name: "New session" }));
    const dialog = await screen.findByRole("dialog", { name: "New session" });
    expect(dialog).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: /main checkout/i })).toBeChecked();
  });

  it("+ session on a task node opens the dialog preset to that worktree", async () => {
    vi.stubGlobal("fetch", stubFetch());
    const user = userEvent.setup();
    render(<ProjectTree client={client} />);

    await user.click(await screen.findByRole("button", { name: "New session in feat-x" }));
    expect(await screen.findByRole("dialog", { name: "New session" })).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: /existing worktree/i })).toBeChecked();
    await waitFor(() =>
      expect(screen.getByRole("combobox")).toHaveValue("/repo/wgenty-code/.worktrees/feat-x"),
    );
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
      if (url === "/api/v1/projects" && method === "GET") return json(PROJECTS);
      if (url.startsWith("/api/v1/worktrees") && method === "GET") return json(WORKTREES);
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

  it("renders multiple projects and groups sessions by projectPath", async () => {
    const spy = stubFetch([...PROJECTS, OTHER_PROJECT]);
    vi.stubGlobal("fetch", spy);
    const m = useSessionManager.getState();
    m.createLocalSession("main chat"); // projectPath null → main project
    m.createLocalSession("docs chat", {
      id: "d2",
      daemonId: "d2",
      projectPath: "/repo/docs",
    });
    render(<ProjectTree client={client} />);

    // Both project nodes render; sessions land under their own project.
    expect(await screen.findByText("wgenty-code")).toBeInTheDocument();
    expect(screen.getByText("docs")).toBeInTheDocument();
    expect(screen.getByText("main chat")).toBeInTheDocument();
    expect(screen.getByText("docs chat")).toBeInTheDocument();
    // Sessions must not leak across projects: exactly one card each.
    expect(screen.getAllByText("main chat")).toHaveLength(1);
    expect(screen.getAllByText("docs chat")).toHaveLength(1);
  });

  it("non-git project hides task actions and never calls the worktree endpoint", async () => {
    const spy = stubFetch([...PROJECTS, OTHER_PROJECT]);
    vi.stubGlobal("fetch", spy);
    render(<ProjectTree client={client} />);

    await screen.findByText("docs");
    // No "New task" button for the non-git project (only the git one has it).
    expect(screen.getByRole("button", { name: "New task in wgenty-code" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "New task in docs" })).toBeNull();
    // Its main node shows the path, not git semantics.
    expect(screen.getAllByText("main checkout")).toHaveLength(1);
    // Worktrees were only fetched for the git project.
    const wtCalls = spy.mock.calls.filter(
      ([u, i]) => String(u).startsWith("/api/v1/worktrees") && (i?.method ?? "GET") === "GET",
    );
    expect(wtCalls).toHaveLength(1);
    expect(String(wtCalls[0][0])).toContain(encodeURIComponent("/repo/wgenty-code"));
  });

  it("remove project calls the API, drops its local sessions, and refetches", async () => {
    const spy = stubFetch([...PROJECTS, OTHER_PROJECT]);
    vi.stubGlobal("fetch", spy);
    vi.stubGlobal("confirm", vi.fn().mockReturnValue(true));
    const m = useSessionManager.getState();
    m.createLocalSession("docs chat", {
      id: "d2",
      daemonId: "d2",
      projectPath: "/repo/docs",
    });
    const user = userEvent.setup();
    render(<ProjectTree client={client} />);

    await user.click(await screen.findByRole("button", { name: "Remove project docs" }));

    await waitFor(() => {
      expect(
        spy.mock.calls.some(
          ([u, i]) =>
            String(u) === `/api/v1/projects?path=${encodeURIComponent("/repo/docs")}` &&
            i?.method === "DELETE",
        ),
      ).toBe(true);
      expect(useSessionManager.getState().entries["d2"]).toBeUndefined();
    });
  });

  it("main project has no remove button", async () => {
    vi.stubGlobal("fetch", stubFetch());
    render(<ProjectTree client={client} />);

    await screen.findByText("wgenty-code");
    expect(screen.queryByRole("button", { name: /remove project/i })).toBeNull();
  });
});
