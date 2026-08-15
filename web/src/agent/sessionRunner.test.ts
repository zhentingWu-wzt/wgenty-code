import { beforeEach, describe, expect, it, vi } from "vitest";
import { useSessionManager } from "../state/sessionManager";
import { useDisplayPrefs } from "../state/displayPrefs";
import { runSessionTurn, stopSessionTurn } from "./sessionRunner";
import type { DaemonClient } from "../api/client";
import type { SessionEvent } from "../api/types";

/** Build a fake SSE body (ReadableStream) from a list of SessionEvents. Each
 *  event is sent as a `data: {...}\n` line, matching the daemon's SSE format. */
function fakeEventStream(events: SessionEvent[]): ReadableStream<Uint8Array> {
  const encoder = new TextEncoder();
  const chunks = events.map((ev) => encoder.encode(`data: ${JSON.stringify(ev)}\n\n`));
  return new ReadableStream({
    start(controller) {
      for (const c of chunks) controller.enqueue(c);
      controller.close();
    },
  });
}

/** A stream that delivers events but NEVER closes — mirrors the real daemon,
 *  which keeps the per-session SSE open (keepalives) after the turn ends. */
function openEventStream(events: SessionEvent[]): ReadableStream<Uint8Array> {
  const encoder = new TextEncoder();
  const chunks = events.map((ev) => encoder.encode(`data: ${JSON.stringify(ev)}\n\n`));
  return new ReadableStream({
    start(controller) {
      for (const c of chunks) controller.enqueue(c);
      // Intentionally no controller.close().
    },
  });
}

/** Minimal fake client: only the methods runSessionTurn touches. Each
 *  subscription gets a FRESH stream: the first delivers `events`, later ones
 *  (reconnects) deliver `reconnectEvents`. */
function fakeClient(opts: {
  runId?: string;
  events?: SessionEvent[];
  reconnectEvents?: SessionEvent[];
  runRejects?: Error;
}): Pick<DaemonClient, "runSession" | "sessionEvents" | "cancelRun" | "createSession"> {
  const sessionId = "daemon-s1";
  let subscribes = 0;
  return {
    createSession: vi.fn().mockResolvedValue({ id: sessionId, name: "s1" }) as never,
    runSession: opts.runRejects
      ? (vi.fn().mockRejectedValue(opts.runRejects) as never)
      : (vi.fn().mockResolvedValue({
          run_id: opts.runId ?? "r1",
          session_id: sessionId,
        }) as never),
    sessionEvents: vi.fn().mockImplementation(() => {
      subscribes += 1;
      const evs = subscribes === 1 ? (opts.events ?? []) : (opts.reconnectEvents ?? []);
      return Promise.resolve({ body: fakeEventStream(evs) });
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
    useDisplayPrefs.setState({ mode: "single" });
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
    expect(client.runSession).toHaveBeenCalledWith("daemon-s1", "hi", expect.any(AbortSignal));
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

  it("turn_done with finish_reason tool_calls is a round boundary, not turn end", async () => {
    // Regression: the daemon used to alias every round's StreamDone to
    // turn_done, so the first tool-deciding round stopped the reader and
    // tool calls never rendered. The reader must keep going past it.
    const client = fakeClient({
      events: [
        makeEvent(1, "turn_done", { finish_reason: "tool_calls" }),
        makeEvent(2, "tool_start", { name: "file_read", args: { path: "/a" } }),
        makeEvent(3, "tool_result", { name: "file_read", args: { path: "/a" }, content: "ok" }),
        makeEvent(4, "content_delta", { text: "final answer" }),
        makeEvent(5, "turn_done", { finish_reason: "stop" }),
      ],
    });
    const id = useSessionManager.getState().createLocalSession("s1");
    await runSessionTurn(client as unknown as DaemonClient, id, "read it");

    const store = useSessionManager.getState().entries[id].store.getState();
    const assistant = store.messages.find((m) => m.role === "assistant");
    expect(assistant?.toolExecs).toHaveLength(1);
    expect(assistant?.content).toBe("final answer");
    expect(store.isRunning).toBe(false);
    expect(useSessionManager.getState().entries[id].status).toBe("idle");
  });

  it("rounds mode: each LLM round gets its own bubble (text + that round's cards)", async () => {
    useDisplayPrefs.setState({ mode: "rounds" });
    const client = fakeClient({
      events: [
        makeEvent(1, "content_delta", { text: "checking…" }),
        makeEvent(2, "turn_done", { finish_reason: "tool_calls" }),
        makeEvent(3, "tool_start", { name: "file_read", args: { path: "/a" } }),
        makeEvent(4, "tool_result", { name: "file_read", args: { path: "/a" }, content: "ok" }),
        makeEvent(5, "content_delta", { text: "final answer" }),
        makeEvent(6, "turn_done", { finish_reason: "stop" }),
      ],
    });
    const id = useSessionManager.getState().createLocalSession("s1");
    await runSessionTurn(client as unknown as DaemonClient, id, "read it");

    const store = useSessionManager.getState().entries[id].store.getState();
    const assistants = store.messages.filter((m) => m.role === "assistant");
    expect(assistants).toHaveLength(2);
    expect(assistants[0].content).toBe("checking…");
    expect(assistants[0].round).toBe(1);
    expect(assistants[0].toolExecs).toHaveLength(1);
    expect(assistants[1].content).toBe("final answer");
    expect(assistants[1].round).toBe(2);
    expect(assistants[1].toolExecs).toBeUndefined();
  });

  it("rounds mode: a text-less tool round keeps its cards on the previous bubble", async () => {
    useDisplayPrefs.setState({ mode: "rounds" });
    const client = fakeClient({
      events: [
        makeEvent(1, "content_delta", { text: "first" }),
        makeEvent(2, "turn_done", { finish_reason: "tool_calls" }),
        makeEvent(3, "tool_start", { name: "grep", args: { pattern: "x" } }),
        makeEvent(4, "tool_result", { name: "grep", args: { pattern: "x" }, content: "hit" }),
        makeEvent(5, "turn_done", { finish_reason: "tool_calls" }),
        makeEvent(6, "tool_start", { name: "file_read", args: { path: "/b" } }),
        makeEvent(7, "tool_result", { name: "file_read", args: { path: "/b" }, content: "body" }),
        makeEvent(8, "content_delta", { text: "done" }),
        makeEvent(9, "turn_done", { finish_reason: "stop" }),
      ],
    });
    const id = useSessionManager.getState().createLocalSession("s1");
    await runSessionTurn(client as unknown as DaemonClient, id, "go");

    const store = useSessionManager.getState().entries[id].store.getState();
    const assistants = store.messages.filter((m) => m.role === "assistant");
    expect(assistants).toHaveLength(2);
    expect(assistants[0].content).toBe("first");
    expect(assistants[0].toolExecs).toHaveLength(2);
    expect(assistants[1].content).toBe("done");
    expect(assistants[1].round).toBe(2);
  });

  it("timeline mode: tool entries interleave as separate messages in stream order", async () => {
    useDisplayPrefs.setState({ mode: "timeline" });
    const client = fakeClient({
      events: [
        makeEvent(1, "content_delta", { text: "checking…" }),
        makeEvent(2, "turn_done", { finish_reason: "tool_calls" }),
        makeEvent(3, "tool_start", { name: "file_read", args: { path: "/a" } }),
        makeEvent(4, "tool_result", { name: "file_read", args: { path: "/a" }, content: "ok" }),
        makeEvent(5, "content_delta", { text: "final answer" }),
        makeEvent(6, "turn_done", { finish_reason: "stop" }),
      ],
    });
    const id = useSessionManager.getState().createLocalSession("s1");
    await runSessionTurn(client as unknown as DaemonClient, id, "read it");

    const store = useSessionManager.getState().entries[id].store.getState();
    expect(store.messages.map((m) => m.role)).toEqual(["user", "assistant", "tool", "assistant"]);
    const tool = store.messages.find((m) => m.role === "tool")!;
    expect(tool.streaming).toBe(false);
    expect(tool.toolExec?.call.function.name).toBe("file_read");
    expect(tool.toolExec?.response.content).toBe("ok");
    // Assistant bubbles carry no toolExecs in timeline mode — tools are entries.
    const assistants = store.messages.filter((m) => m.role === "assistant");
    expect(assistants.every((m) => !m.toolExecs)).toBe(true);
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
      // The daemon answers a cancel with a terminal event; the resubscription
      // (after the first stream closed) replays/delivers it.
      reconnectEvents: [makeEvent(2, "turn_done", { finish_reason: "cancelled" })],
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

  it("subscribes to the event stream BEFORE POST /run (no missed fast turns)", async () => {
    // Regression: subscribing after POST /run let a fast turn's whole event
    // batch (turn_done included) fire before the live-only stream attached —
    // the reader then waited forever ("sent a message, nothing happens").
    const order: string[] = [];
    const client = fakeClient({});
    (client.sessionEvents as ReturnType<typeof vi.fn>).mockImplementation(() => {
      order.push("subscribe");
      return Promise.resolve({
        body: fakeEventStream([makeEvent(1, "turn_done", { finish_reason: "stop" })]),
      });
    });
    (client.runSession as ReturnType<typeof vi.fn>).mockImplementation(() => {
      order.push("run");
      return Promise.resolve({ run_id: "r1", session_id: "daemon-s1" });
    });
    const id = useSessionManager.getState().createLocalSession("s1");
    await runSessionTurn(client as unknown as DaemonClient, id, "hi");

    expect(order).toEqual(["subscribe", "run"]);
  });

  it("resubscribes with after=<lastSeq> when the stream drops mid-turn", async () => {
    const client = fakeClient({
      events: [makeEvent(1, "content_delta", { text: "partial" })],
      reconnectEvents: [
        // Stale terminal event from an older run — must NOT end our turn.
        { ...makeEvent(2, "turn_done", { finish_reason: "stop" }), run_id: "old-run" },
        makeEvent(2, "content_delta", { text: " rest" }),
        makeEvent(3, "turn_done", { finish_reason: "stop" }),
      ],
    });
    const id = useSessionManager.getState().createLocalSession("s1");
    await runSessionTurn(client as unknown as DaemonClient, id, "x");

    // Second subscription resumes after the last seq we actually saw.
    expect(client.sessionEvents).toHaveBeenNthCalledWith(
      2,
      "daemon-s1",
      1,
      expect.any(AbortSignal),
    );
    const store = useSessionManager.getState().entries[id].store.getState();
    expect(store.messages.some((m) => m.content === "partial rest")).toBe(true);
    expect(store.isRunning).toBe(false);
    expect(useSessionManager.getState().entries[id].status).toBe("idle");
  });

  it("completes the turn even when the daemon keeps the stream open", async () => {
    // Regression: the real daemon never closes the per-session SSE after a
    // turn (keepalives keep flowing). The reader must exit on turn_done, not
    // on stream close — otherwise every turn hangs in "running" forever and
    // the Stop button appears dead too.
    const client = fakeClient({});
    (client.sessionEvents as ReturnType<typeof vi.fn>).mockImplementation(() =>
      Promise.resolve({
        body: openEventStream([
          makeEvent(1, "content_delta", { text: "answer" }),
          makeEvent(2, "turn_done", { finish_reason: "stop" }),
        ]),
      }),
    );
    const id = useSessionManager.getState().createLocalSession("s1");
    await runSessionTurn(client as unknown as DaemonClient, id, "x");

    const store = useSessionManager.getState().entries[id].store.getState();
    expect(store.messages.some((m) => m.content === "answer")).toBe(true);
    expect(store.isRunning).toBe(false);
    expect(useSessionManager.getState().entries[id].status).toBe("idle");
  });
});
