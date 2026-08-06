import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { NewSessionModal } from "./NewSessionModal";
import { useSessionManager } from "../../state/sessionManager";
import { DaemonClient } from "../../api/client";

const client = new DaemonClient();

const WORKTREES = [
  { path: "/repo", head: "a", branch: "main", is_main: true },
  { path: "/repo/.worktrees/feat", head: "b", branch: "feat", is_main: false },
];

const CREATED = {
  id: "d1",
  name: "d1",
  created_at: "2026-08-02",
  updated_at: "2026-08-02",
  messages: [],
  ui_messages: [],
};

/** Route-aware fetch mock. `failWorktreeCreate` makes POST /worktrees 400. */
function stubFetch(failWorktreeCreate = false) {
  return vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    const method = init?.method ?? "GET";
    const json = (payload: unknown, status = 200) =>
      new Response(JSON.stringify(payload), { status });
    if (url.startsWith("/api/v1/worktrees") && method === "GET") return json(WORKTREES);
    if (url === "/api/v1/worktrees" && method === "POST") {
      if (failWorktreeCreate) return new Response("already exists", { status: 400 });
      return new Response(null, { status: 201 });
    }
    if (url === "/api/v1/sessions" && method === "POST") return json(CREATED);
    if (url.endsWith("/worktree") && method === "PUT") {
      return json({ session_id: "d1", worktree: { path: "p", branch: "b" } });
    }
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

describe("NewSessionModal", () => {
  beforeEach(reset);
  afterEach(() => vi.unstubAllGlobals());

  it("main checkout (default) creates a plain local session without API calls", async () => {
    const spy = stubFetch();
    vi.stubGlobal("fetch", spy);
    const onClose = vi.fn();
    const user = userEvent.setup();
    render(<NewSessionModal client={client} onClose={onClose} />);

    await user.click(screen.getByRole("button", { name: "Create" }));

    const s = useSessionManager.getState();
    expect(s.order).toHaveLength(1);
    expect(s.entries[s.order[0]].daemonId).toBeNull();
    expect(
      spy.mock.calls.some(([u, i]) => String(u) === "/api/v1/sessions" && i?.method === "POST"),
    ).toBe(false);
    expect(onClose).toHaveBeenCalled();
  });

  it("existing worktree: creates daemon session and binds it", async () => {
    const spy = stubFetch();
    vi.stubGlobal("fetch", spy);
    const user = userEvent.setup();
    render(<NewSessionModal client={client} onClose={() => {}} />);

    await user.click(screen.getByRole("radio", { name: /existing worktree/i }));
    await waitFor(() => screen.getByRole("option", { name: "feat" }));
    await user.selectOptions(screen.getByRole("combobox"), "/repo/.worktrees/feat");
    await user.click(screen.getByRole("button", { name: "Create" }));

    await waitFor(() => {
      const e = useSessionManager.getState().entries["d1"];
      expect(e).toBeDefined();
      expect(e.worktree?.branch).toBe("feat");
      expect(e.daemonId).toBe("d1");
    });
    const bind = spy.mock.calls.find(
      ([u, i]) => String(u).endsWith("/worktree") && i?.method === "PUT",
    );
    expect(bind).toBeDefined();
    expect(JSON.parse(bind![1]!.body as string)).toEqual({
      path: "/repo/.worktrees/feat",
      branch: "feat",
    });
  });

  it("new worktree: creates session + worktree + binding in order", async () => {
    const spy = stubFetch();
    vi.stubGlobal("fetch", spy);
    const user = userEvent.setup();
    render(<NewSessionModal client={client} onClose={() => {}} />);

    await user.click(screen.getByRole("radio", { name: /new worktree/i }));
    await user.type(screen.getByPlaceholderText(/branch name/i), "feat-x");
    await user.click(screen.getByRole("button", { name: "Create" }));

    await waitFor(() => {
      const e = useSessionManager.getState().entries["d1"];
      expect(e?.worktree?.branch).toBe("feat-x");
    });
    const mk = spy.mock.calls.find(
      ([u, i]) => String(u) === "/api/v1/worktrees" && i?.method === "POST",
    );
    expect(mk).toBeDefined();
    expect(JSON.parse(mk![1]!.body as string)).toEqual({
      path: ".worktrees/feat-x",
      branch: "feat-x",
    });
  });

  it("preset existing worktree preselects the bound workspace", async () => {
    vi.stubGlobal("fetch", stubFetch());
    render(
      <NewSessionModal
        client={client}
        onClose={() => {}}
        preset={{ mode: "existing", project: "/repo", path: "/repo/.worktrees/feat", branch: "feat" }}
      />,
    );

    expect(screen.getByRole("radio", { name: /existing worktree/i })).toBeChecked();
    await waitFor(() =>
      expect(screen.getByRole("combobox")).toHaveValue("/repo/.worktrees/feat"),
    );
  });

  it("worktree creation failure shows inline error and creates no session", async () => {
    vi.stubGlobal("fetch", stubFetch(true));
    const onClose = vi.fn();
    const user = userEvent.setup();
    render(<NewSessionModal client={client} onClose={onClose} />);

    await user.click(screen.getByRole("radio", { name: /new worktree/i }));
    await user.type(screen.getByPlaceholderText(/branch name/i), "feat-x");
    await user.click(screen.getByRole("button", { name: "Create" }));

    expect(await screen.findByText(/already exists/)).toBeInTheDocument();
    expect(useSessionManager.getState().order).toHaveLength(0);
    expect(onClose).not.toHaveBeenCalled();
  });

  it("preset main with a project creates a daemon session carrying project_path", async () => {
    const spy = stubFetch();
    vi.stubGlobal("fetch", spy);
    const onClose = vi.fn();
    const user = userEvent.setup();
    render(
      <NewSessionModal
        client={client}
        onClose={onClose}
        preset={{ mode: "main", project: "/repo" }}
      />,
    );

    await user.type(screen.getByPlaceholderText(/session name/i), "proj chat");
    await user.click(screen.getByRole("button", { name: "Create" }));

    await waitFor(() => {
      const create = spy.mock.calls.find(
        ([u, i]) => String(u) === "/api/v1/sessions" && i?.method === "POST",
      );
      expect(create).toBeDefined();
      expect(JSON.parse(create![1]!.body as string)).toEqual({
        name: "proj chat",
        project_path: "/repo",
      });
      const e = useSessionManager.getState().entries["d1"];
      expect(e?.daemonId).toBe("d1");
      expect(e?.projectPath).toBe("/repo");
    });
    expect(onClose).toHaveBeenCalled();
  });

  it("preset project scopes the worktree list and new-worktree creation to it", async () => {
    const spy = stubFetch();
    vi.stubGlobal("fetch", spy);
    const user = userEvent.setup();
    render(
      <NewSessionModal
        client={client}
        onClose={() => {}}
        preset={{ mode: "main", project: "/repo" }}
      />,
    );

    // The dropdown data is fetched with the project query param.
    await waitFor(() =>
      expect(
        spy.mock.calls.some(
          ([u, i]) =>
            String(u) === `/api/v1/worktrees?project=${encodeURIComponent("/repo")}` &&
            (i?.method ?? "GET") === "GET",
        ),
      ).toBe(true),
    );

    await user.click(screen.getByRole("radio", { name: /new worktree/i }));
    await user.type(screen.getByPlaceholderText(/branch name/i), "feat-y");
    await user.click(screen.getByRole("button", { name: "Create" }));

    await waitFor(() => {
      const mk = spy.mock.calls.find(
        ([u, i]) => String(u) === "/api/v1/worktrees" && i?.method === "POST",
      );
      expect(mk).toBeDefined();
      expect(JSON.parse(mk![1]!.body as string)).toEqual({
        path: ".worktrees/feat-y",
        branch: "feat-y",
        project: "/repo",
      });
      const created = spy.mock.calls.find(
        ([u, i]) => String(u) === "/api/v1/sessions" && i?.method === "POST",
      );
      expect(JSON.parse(created![1]!.body as string)).toEqual({ project_path: "/repo" });
    });
  });
});
