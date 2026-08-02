import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { SessionList } from "./SessionList";
import { sessionMessagesToDisplay } from "../agent/sessionLoad";
import { useSessionManager } from "../state/sessionManager";
import { DaemonClient } from "../api/client";

const client = new DaemonClient();
import type { SessionMessage } from "../api/types";

function reset() {
  useSessionManager.setState({
    entries: {},
    order: [],
    activeId: null,
    connection: "unknown",
    modelName: null,
  });
}

describe("SessionList", () => {
  beforeEach(reset);

  it("renders open sessions with status dot and preview", () => {
    const m = useSessionManager.getState();
    const a = m.createLocalSession("fix bug");
    m.setPreview(a, "reading files…");
    render(<SessionList client={client} />);
    expect(screen.getByText("fix bug")).toBeInTheDocument();
    expect(screen.getByText("reading files…")).toBeInTheDocument();
  });

  it("clicking a session card makes it active", async () => {
    const m = useSessionManager.getState();
    const a = m.createLocalSession("first");
    const b = m.createLocalSession("second");
    m.setActive(a);
    render(<SessionList client={client} />);
    await userEvent.setup().click(screen.getByText("second"));
    expect(useSessionManager.getState().activeId).toBe(b);
  });

  it("new-session button opens the dialog; Create makes a session", async () => {
    const user = userEvent.setup();
    render(<SessionList client={client} />);

    await user.click(screen.getByRole("button", { name: /new session/i }));
    // Dialog opens on the main-checkout default; Create makes a local session.
    const dialog = await screen.findByRole("dialog", { name: "New session" });
    expect(dialog).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Create" }));
    expect(useSessionManager.getState().order).toHaveLength(1);
  });
});

const SAVED_MESSAGES: SessionMessage[] = [
  { role: "user", content: "read the file" },
  {
    role: "assistant",
    content: "let me check",
    tool_calls: [
      { id: "call_1", type: "function", function: { name: "file_read", arguments: "{}" } },
    ],
  },
  {
    role: "tool",
    tool_call_id: "call_1",
    content: JSON.stringify({ success: true, content: "file contents" }),
  },
  { role: "assistant", content: "all done" },
];

describe("sessionMessagesToDisplay", () => {
  it("preserves tool-call structure and emits no empty assistant bubble for tool messages", () => {
    const display = sessionMessagesToDisplay(SAVED_MESSAGES);
    expect(display.map((m) => m.role)).toEqual(["user", "assistant", "assistant"]);
    // (a) no empty-content assistant bubble standing in for the tool message
    expect(display.filter((m) => m.role === "assistant" && m.content === "")).toHaveLength(0);
    // (b) tool call id + result content survive in the reconstructed structure
    const withTools = display[1];
    expect(withTools.toolExecs).toHaveLength(1);
    expect(withTools.toolExecs?.[0].call.id).toBe("call_1");
    expect(withTools.toolExecs?.[0].response.success).toBe(true);
    expect(withTools.toolExecs?.[0].response.content).toBe("file contents");
    expect(display[2].content).toBe("all done");
  });

  it("keeps orphaned tool messages as tool-role display messages", () => {
    const display = sessionMessagesToDisplay([
      { role: "tool", tool_call_id: "call_x", content: "raw result" },
    ]);
    expect(display).toHaveLength(1);
    expect(display[0].role).toBe("tool");
    expect(display[0].toolCallId).toBe("call_x");
    expect(display[0].content).toBe("raw result");
  });

  it("does not throw on malformed or missing tool results", () => {
    const display = sessionMessagesToDisplay([
      {
        role: "assistant",
        tool_calls: [
          { id: "call_1", type: "function", function: { name: "file_read", arguments: "{}" } },
          { id: "call_2", type: "function", function: { name: "grep", arguments: "{}" } },
        ],
      },
      // Non-JSON content, and no result at all for call_2.
      { role: "tool", tool_call_id: "call_1", content: "not json" },
    ]);
    const execs = display[0].toolExecs ?? [];
    expect(execs).toHaveLength(2);
    expect(execs[0].response).toEqual({ success: true, content: "not json" });
    expect(execs[1].response.success).toBe(false);
  });
});

describe("SessionList grouping and actions", () => {
  beforeEach(() => {
    useSessionManager.setState({
      entries: {},
      order: [],
      activeId: null,
      connection: "unknown",
      modelName: null,
    });
  });
  afterEach(() => vi.unstubAllGlobals());

  it("groups sessions under Main checkout and worktree titles", () => {
    const m = useSessionManager.getState();
    m.createLocalSession("chat one");
    m.createLocalSession("chat two");
    m.createLocalSession("task a", {
      id: "d1",
      daemonId: "d1",
      worktree: { path: ".worktrees/feat-x", branch: "feat-x" },
    });
    m.createLocalSession("task b", {
      id: "d2",
      daemonId: "d2",
      worktree: { path: ".worktrees/feat-x", branch: "feat-x" },
    });
    render(<SessionList client={client} />);

    expect(screen.getByText("Main checkout")).toBeInTheDocument();
    expect(screen.getByText("⎇ feat-x")).toBeInTheDocument();
    expect(screen.getByText("chat one")).toBeInTheDocument();
    expect(screen.getByText("task a")).toBeInTheDocument();
  });

  it("archive button calls the API and closes the entry", async () => {
    const spy = vi.fn().mockResolvedValue(new Response(JSON.stringify({}), { status: 200 }));
    vi.stubGlobal("fetch", spy);
    const m = useSessionManager.getState();
    m.createLocalSession("bound", {
      id: "d1",
      daemonId: "d1",
      worktree: { path: ".worktrees/feat-x", branch: "feat-x" },
    });
    render(<SessionList client={client} />);

    await userEvent.setup().click(screen.getByRole("button", { name: /archive bound/i }));

    expect(
      spy.mock.calls.some(
        ([u, i]) => String(u) === "/api/v1/sessions/d1/archive" && i?.method === "PUT",
      ),
    ).toBe(true);
    expect(useSessionManager.getState().entries["d1"]).toBeUndefined();
  });

  it("delete button confirms, calls the API and removes the entry", async () => {
    const spy = vi.fn().mockResolvedValue(new Response(null, { status: 204 }));
    vi.stubGlobal("fetch", spy);
    vi.stubGlobal("confirm", vi.fn().mockReturnValue(true));
    const m = useSessionManager.getState();
    m.createLocalSession("doomed", { id: "d9", daemonId: "d9" });
    render(<SessionList client={client} />);

    await userEvent.setup().click(screen.getByRole("button", { name: /delete doomed/i }));

    expect(
      spy.mock.calls.some(
        ([u, i]) => String(u) === "/api/v1/sessions/d9" && i?.method === "DELETE",
      ),
    ).toBe(true);
    expect(useSessionManager.getState().entries["d9"]).toBeUndefined();
  });
});
