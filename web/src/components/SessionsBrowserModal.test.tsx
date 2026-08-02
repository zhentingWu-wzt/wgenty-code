import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { SessionsBrowserModal } from "./SessionsBrowserModal";
import { useSessionManager } from "../state/sessionManager";
import { DaemonClient } from "../api/client";
import type { SessionMessage } from "../api/types";

const client = new DaemonClient();

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

const SAVED_INFO = {
  id: "d1",
  name: "saved one",
  created_at: "2026-08-01",
  updated_at: "2026-08-01",
  message_count: SAVED_MESSAGES.length,
};

function stubSessionsFetch() {
  return vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    if (init?.method === "DELETE") return new Response(null, { status: 204 });
    const payload =
      url === "/api/v1/sessions"
        ? [SAVED_INFO]
        : { ...SAVED_INFO, messages: SAVED_MESSAGES, ui_messages: [] };
    return new Response(JSON.stringify(payload), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  });
}

describe("SessionsBrowserModal", () => {
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

  it("lists saved sessions and opens one losslessly (tool calls preserved)", async () => {
    vi.stubGlobal("fetch", stubSessionsFetch());
    render(<SessionsBrowserModal client={client} onClose={() => {}} />);

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

  it("opening closes the modal", async () => {
    vi.stubGlobal("fetch", stubSessionsFetch());
    const onClose = vi.fn();
    render(<SessionsBrowserModal client={client} onClose={onClose} />);
    await userEvent.setup().click(await screen.findByText("saved one"));
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });

  it("delete removes the session after confirm", async () => {
    const spy = stubSessionsFetch();
    vi.stubGlobal("fetch", spy);
    vi.stubGlobal("confirm", vi.fn().mockReturnValue(true));
    render(<SessionsBrowserModal client={client} onClose={() => {}} />);

    await userEvent.setup().click(await screen.findByRole("button", { name: /delete saved one/i }));

    await waitFor(() => {
      const del = spy.mock.calls.find(([, init]) => init?.method === "DELETE");
      expect(del).toBeDefined();
      expect(String(del![0])).toBe("/api/v1/sessions/d1");
    });
  });
});
