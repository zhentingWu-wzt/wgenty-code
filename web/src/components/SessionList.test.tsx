import { beforeEach, afterEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { SessionList } from "./SessionList";
import { sessionMessagesToDisplay } from "../agent/sessionLoad";
import { useSessionManager } from "../state/sessionManager";
import { DaemonClient } from "../api/client";
import type { SessionMessage } from "../api/types";

const client = new DaemonClient();

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

  it("new-session button creates and activates a session", async () => {
    render(<SessionList client={client} />);
    await userEvent.setup().click(screen.getByRole("button", { name: /new session/i }));
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

describe("SessionList saved sessions", () => {
  beforeEach(reset);
  afterEach(() => vi.unstubAllGlobals());

  it("loading a saved session preserves tool calls (no lossy rewrite)", async () => {
    const full = {
      id: "d1",
      name: "saved one",
      created_at: "2026-08-01",
      updated_at: "2026-08-01",
      messages: SAVED_MESSAGES,
      ui_messages: [],
    };
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input);
        const payload =
          url === "/api/v1/sessions"
            ? [
                {
                  id: "d1",
                  name: "saved one",
                  created_at: "2026-08-01",
                  updated_at: "2026-08-01",
                  message_count: SAVED_MESSAGES.length,
                },
              ]
            : full;
        return new Response(JSON.stringify(payload), {
          status: 200,
          headers: { "content-type": "application/json" },
        });
      }),
    );

    render(<SessionList client={client} />);
    await userEvent.setup().click(await screen.findByText("saved one"));

    await waitFor(() => {
      const entry = Object.values(useSessionManager.getState().entries).find(
        (e) => e.daemonId === "d1",
      );
      expect(entry).toBeDefined();
      const msgs = entry!.store.getState().messages;
      // No empty assistant bubble where the tool message was.
      expect(msgs.filter((m) => m.role === "assistant" && m.content === "")).toHaveLength(0);
      const withTools = msgs.find((m) => m.toolExecs && m.toolExecs.length > 0);
      expect(withTools?.toolExecs?.[0].call.id).toBe("call_1");
      expect(withTools?.toolExecs?.[0].response.content).toBe("file contents");
    });
  });
});
