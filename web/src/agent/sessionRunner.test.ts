import { beforeEach, describe, expect, it, vi } from "vitest";
import { useSessionManager } from "../state/sessionManager";
import { runSessionTurn, stopSessionTurn } from "./sessionRunner";
import type { DaemonClient } from "../api/client";
import type { SessionEvent } from "../api/types";

/** Build a fake SSE body (ReadableStream) from a list of SessionEvents. Each
 *  event is sent as a `data: {...}\n` line, matching the daemon's SSE format. */
function fakeEventStream(events: SessionEvent[]): ReadableStream<Uint8Array> {
  const encoder = new TextEncoder();
  const chunks = events.map(
    (ev) => encoder.encode(`data: ${JSON.stringify(ev)}\n\n`),
  );
  return new ReadableStream({
    start(controller) {
      for (const c of chunks) controller.enqueue(c);
      controller.close();
    },
  });
}

/** Minimal fake client: only the methods runSessionTurn touches. */
function fakeClient(opts: {
  runId?: string;
  events?: SessionEvent[];
  runRejects?: Error;
}): Pick<DaemonClient, "runSession" | "sessionEvents" | "cancelRun" | "createSession"> {
  const sessionId = "daemon-s1";
  return {
    createSession: vi.fn().mockResolvedValue({ id: sessionId, name: "s1" }) as never,
    runSession: opts.runRejects
      ? vi.fn().mockRejectedValue(opts.runRejects) as never
      : vi.fn().mockResolvedValue({
          run_id: opts.runId ?? "r1",
          session_id: sessionId,
        }) as never,
    sessionEvents: vi.fn().mockResolvedValue({
      body: fakeEventStream(opts.events ?? []),
    }) as never,
    cancelRun: vi.fn().mockResolvedValue(undefined) as never,
  };
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

function makeEvent(
  seq: number,
  kind: SessionEvent["kind"],
  data: Record<string, unknown>,
): SessionEvent {
  return { seq, session_id: "daemon-s1", run_id: "r1", kind, data };
}

describe("runSessionTurn (server-side observer)", () => {
  beforeEach(() => {
    reset();
    vi.clearAllMocks();
  });

  it("POSTs /run, streams content_delta into the store, then goes idle", async () => {
    const client = fakeClient({
      events: [
        makeEvent(1, "content_delta", { text: "Hello" }),
        makeEvent(2, "content_delta", { text: ", world" }),
        makeEvent(3, "turn_done", { finish_reason: "stop" }),
      ],
    });
    const id = useSessionManager.getState().createLocalSession("s1");
    await runSessionTurn(client as unknown as DaemonClient, id, "hi");

    const store = useSessionManager.getState().entries[id].store.getState();
    expect(store.messages.some((m) => m.content === "Hello, world")).toBe(true);
    expect(store.isRunning).toBe(false);
    expect(useSessionManager.getState().entries[id].status).toBe("idle");
    expect(client.runSession).toHaveBeenCalledWith("daemon-s1", "hi");
  });

  it("tool_start + tool_result renders as a tool exec card", async () => {
    const client = fakeClient({
      events: [
        makeEvent(1, "tool_start", { name: "file_read", args: { path: "/a" } }),
        makeEvent(2, "tool_result", { name: "file_read", args: { path: "/a" }, content: "ok" }),
        makeEvent(3, "turn_done", { finish_reason: "stop" }),
      ],
    });
    const id = useSessionManager.getState().createLocalSession("s1");
    await runSessionTurn(client as unknown as DaemonClient, id, "read it");

    const store = useSessionManager.getState().entries[id].store.getState();
    const assistant = store.messages.find((m) => m.role === "assistant");
    expect(assistant?.toolExecs).toHaveLength(1);
    expect(assistant?.toolExecs?.[0].call.function.name).toBe("file_read");
  });

  it("turn_error marks the session error with the message", async () => {
    const client = fakeClient({
      events: [makeEvent(1, "turn_error", { message: "upstream rejected" })],
    });
    const id = useSessionManager.getState().createLocalSession("s1");
    await runSessionTurn(client as unknown as DaemonClient, id, "x");

    const entry = useSessionManager.getState().entries[id];
    expect(entry.status).toBe("error");
    expect(entry.store.getState().lastError?.kind).toBe("upstream");
    expect(entry.store.getState().lastError?.message).toContain("upstream rejected");
  });

  it("reasoning_delta appends to the reasoning block", async () => {
    const client = fakeClient({
      events: [
        makeEvent(1, "reasoning_delta", { text: "thinking..." }),
        makeEvent(2, "content_delta", { text: "answer" }),
        makeEvent(3, "turn_done", { finish_reason: "stop" }),
      ],
    });
    const id = useSessionManager.getState().createLocalSession("s1");
    await runSessionTurn(client as unknown as DaemonClient, id, "x");

    const assistant = useSessionManager
      .getState()
      .entries[id].store.getState()
      .messages.find((m) => m.role === "assistant");
    expect(assistant?.reasoning).toContain("thinking");
  });

  it("stopSessionTurn POSTs /cancel and aborts the reader", async () => {
    const client = fakeClient({
      events: [
        makeEvent(1, "content_delta", { text: "partial" }),
        // No turn_done — the turn is still "running" when we cancel.
      ],
    });
    const id = useSessionManager.getState().createLocalSession("s1");
    // Start the turn but don't await (it's still streaming).
    const p = runSessionTurn(client as unknown as DaemonClient, id, "x");
    // Give it a tick to subscribe.
    await new Promise((r) => setTimeout(r, 50));
    await stopSessionTurn(client as unknown as DaemonClient, id);
    await p;

    expect(client.cancelRun).toHaveBeenCalledWith("daemon-s1");
    expect(useSessionManager.getState().entries[id].status).toBe("idle");
  });
});
