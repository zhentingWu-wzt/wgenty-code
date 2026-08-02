import { beforeEach, describe, expect, it } from "vitest";
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
