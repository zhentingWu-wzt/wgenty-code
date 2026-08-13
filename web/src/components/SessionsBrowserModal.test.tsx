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
  status: "Active",
  worktree: null,
};

const ARCHIVED_INFO = { ...SAVED_INFO, id: "d2", name: "old chat", status: "Archived" };

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
      // Tool calls are preserved losslessly in any display mode: folded into an
      // assistant bubble's `toolExecs` (single/rounds) or as a standalone
      // `role:"tool"` entry carrying `toolExec` (timeline).
      const exec = msgs
        .flatMap((m) => (m.toolExecs && m.toolExecs.length ? m.toolExecs : m.toolExec ? [m.toolExec] : []))
        .find((e) => e.call.id === "call_1");
      expect(exec?.call.id).toBe("call_1");
      expect(exec?.response.content).toBe("file contents");
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

describe("SessionsBrowserModal archive view", () => {
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

  it("hides archived sessions in the collapsed Archived section; unarchive restores", async () => {
    const spy = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      if (url.endsWith("/archive") && init?.method === "PUT") {
        return new Response(JSON.stringify({ session_id: "d2", archived: false }), { status: 200 });
      }
      if (url === "/api/v1/sessions") {
        return new Response(JSON.stringify([SAVED_INFO, ARCHIVED_INFO]), { status: 200 });
      }
      return new Response("not found", { status: 404 });
    });
    vi.stubGlobal("fetch", spy);
    const user = userEvent.setup();
    render(<SessionsBrowserModal client={client} onClose={() => {}} />);

    // Default view: only the active session; archived one hidden.
    expect(await screen.findByText("saved one")).toBeInTheDocument();
    expect(screen.queryByText("old chat")).not.toBeInTheDocument();

    // Expand the Archived section and unarchive it.
    await user.click(screen.getByRole("button", { name: /Archived \(1\)/ }));
    await user.click(await screen.findByRole("button", { name: /unarchive old chat/i }));
    await waitFor(() => {
      const call = spy.mock.calls.find(
        ([u, i]) => String(u).endsWith("/archive") && i?.method === "PUT",
      );
      expect(call).toBeDefined();
      expect(JSON.parse(call![1]!.body as string)).toEqual({ archived: false });
    });
  });

  it("archive button on a default row calls the API", async () => {
    const spy = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      if (url.endsWith("/archive") && init?.method === "PUT") {
        return new Response(JSON.stringify({ session_id: "d1", archived: true }), { status: 200 });
      }
      if (url === "/api/v1/sessions") {
        return new Response(JSON.stringify([SAVED_INFO]), { status: 200 });
      }
      return new Response("not found", { status: 404 });
    });
    vi.stubGlobal("fetch", spy);
    render(<SessionsBrowserModal client={client} onClose={() => {}} />);

    await userEvent
      .setup()
      .click(await screen.findByRole("button", { name: /archive saved one/i }));
    await waitFor(() => {
      const call = spy.mock.calls.find(
        ([u, i]) => String(u).endsWith("/archive") && i?.method === "PUT",
      );
      expect(call).toBeDefined();
      expect(JSON.parse(call![1]!.body as string)).toEqual({ archived: true });
    });
  });
});
