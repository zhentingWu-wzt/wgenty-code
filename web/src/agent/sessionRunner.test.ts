import { beforeEach, describe, expect, it, vi } from "vitest";
import { useSessionManager } from "../state/sessionManager";
import { runSessionTurn, stopSessionTurn } from "./sessionRunner";
import type { DaemonClient } from "../api/client";
import type { SessionEvent } from "../api/types";

// ── Fake ws push channel ─────────────────────────────────────────────────────
// sessionRunner consumes the wsChannel singleton; tests swap it for a local
// dispatcher whose handlers register synchronously (like the real channel's
// subscribeSession), so delivering events from inside the POST /run mock is
// deterministic — the subscribe always happened first (the fast-turn race).

const wsTest = vi.hoisted(() => {
  const state = {
    sessionHandlers: new Map<string, Set<(ev: never) => void>>(),
    status: "open" as "idle" | "connecting" | "open" | "backoff",
    sent: [] as Array<{ op: string; session_id: string; after?: number }>,
  };
  return state;
});

vi.mock("../api/wsChannel", () => ({
  wsChannel: {
    connect: () => {},
    status: () => wsTest.status,
    subscribeTrace: () => () => {},
    subscribeGlobal: () => () => {},
    subscribeSession: (
      sessionId: string,
      handler: (ev: never) => void,
      opts?: { after?: number },
    ) => {
      let set = wsTest.sessionHandlers.get(sessionId);
      if (!set) {
        set = new Set();
        wsTest.sessionHandlers.set(sessionId, set);
      }
      set.add(handler);
      wsTest.sent.push({ op: "subscribe", session_id: sessionId, after: opts?.after });
      return {
        unsubscribe: () => {
          set?.delete(handler);
          wsTest.sent.push({ op: "unsubscribe", session_id: sessionId });
        },
      };
    },
    onReconnected: () => () => {},
  },
}));

/** Deliver a session event to every registered subscriber (server → client). */
function serverSend(sessionId: string, ev: SessionEvent): void {
  for (const h of wsTest.sessionHandlers.get(sessionId) ?? []) h(ev as never);
}

/** Minimal fake client: only the methods runSessionTurn touches. Events are
 *  pushed onto the ws channel when POST /run resolves (subscribe-first order
 *  guaranteed — see awaitTurnOverWs). */
function fakeClient(opts: {
  runId?: string;
  events?: SessionEvent[];
  runRejects?: Error;
}): Pick<DaemonClient, "runSession" | "cancelRun" | "createSession"> {
  const sessionId = "daemon-s1";
  return {
    createSession: vi.fn().mockResolvedValue({ id: sessionId, name: "s1" }) as never,
    runSession: opts.runRejects
      ? (vi.fn().mockRejectedValue(opts.runRejects) as never)
      : (vi.fn().mockImplementation(async () => {
          // Deliver after the subscribe: these events reach the awaiter
          // (buffered until the run id arrives, exactly like the wire).
          for (const ev of opts.events ?? []) serverSend(sessionId, ev);
          return { run_id: opts.runId ?? "r1", session_id: sessionId };
        }) as never),
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
  wsTest.sessionHandlers.clear();
  wsTest.status = "open";
  wsTest.sent = [];
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
    expect(client.runSession).toHaveBeenCalledWith("daemon-s1", "hi", expect.any(AbortSignal));
  });

  it("tool_start + tool_result renders as a standalone tool entry", async () => {
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
    const tool = store.messages.find((m) => m.role === "tool")!;
    expect(tool.toolExec?.call.function.name).toBe("file_read");
    expect(tool.toolExec?.response.content).toBe("ok");
  });

  it("turn_done with finish_reason tool_calls is a round boundary, not turn end", async () => {
    // Regression: the daemon used to alias every round's StreamDone to
    // turn_done, so the first tool-deciding round stopped the observer and
    // tool calls never rendered. The observer must keep going past it.
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
    expect(store.messages.map((m) => m.role)).toEqual(["user", "tool", "assistant"]);
    const tool = store.messages.find((m) => m.role === "tool")!;
    expect(tool.toolExec?.call.function.name).toBe("file_read");
    expect(store.messages.find((m) => m.role === "assistant")?.content).toBe("final answer");
    expect(store.isRunning).toBe(false);
    expect(useSessionManager.getState().entries[id].status).toBe("idle");
  });

  it("text-less tool rounds render as standalone entries between text bubbles", async () => {
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
    expect(store.messages.map((m) => m.role)).toEqual([
      "user",
      "assistant",
      "tool",
      "tool",
      "assistant",
    ]);
    const tools = store.messages.filter((m) => m.role === "tool");
    expect(tools.map((t) => t.toolExec?.call.function.name)).toEqual(["grep", "file_read"]);
    const assistants = store.messages.filter((m) => m.role === "assistant");
    expect(assistants[0].content).toBe("first");
    expect(assistants[1].content).toBe("done");
    expect(assistants[1].round).toBe(2);
  });

  it("tool entries interleave as separate messages in stream order", async () => {
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

  it("stopSessionTurn POSTs /cancel and unwinds the observer", async () => {
    const client = fakeClient({
      events: [
        makeEvent(1, "content_delta", { text: "partial" }),
        // No turn_done — the turn is still "running" when we cancel.
      ],
    });
    const id = useSessionManager.getState().createLocalSession("s1");
    // Start the turn but don't await (it's still streaming).
    const p = runSessionTurn(client as unknown as DaemonClient, id, "x");
    // Give it a tick to subscribe and deliver the partial event.
    await new Promise((r) => setTimeout(r, 50));
    await stopSessionTurn(client as unknown as DaemonClient, id);
    await p;

    expect(client.cancelRun).toHaveBeenCalledWith("daemon-s1");
    expect(useSessionManager.getState().entries[id].status).toBe("idle");
    // The ws subscription was released with the turn.
    expect(wsTest.sent.at(-1)).toEqual({ op: "unsubscribe", session_id: "daemon-s1" });
  });

  it("subscribes to the ws session stream BEFORE POST /run (no missed fast turns)", async () => {
    // Regression: subscribing after POST /run let a fast turn's whole event
    // batch (turn_done included) fire before the live-only stream attached —
    // the observer then waited forever ("sent a message, nothing happens").
    const order: string[] = [];
    const client = fakeClient({
      events: [makeEvent(1, "turn_done", { finish_reason: "stop" })],
    });
    (client.runSession as ReturnType<typeof vi.fn>).mockImplementation(async () => {
      order.push("run");
      for (const ev of [makeEvent(1, "turn_done", { finish_reason: "stop" })]) {
        serverSend("daemon-s1", ev);
      }
      return { run_id: "r1", session_id: "daemon-s1" };
    });
    wsTest.sent.length = 0;
    const push = wsTest.sent.push.bind(wsTest.sent);
    wsTest.sent.push = (op) => {
      order.push(op.op);
      return push(op);
    };
    const id = useSessionManager.getState().createLocalSession("s1");
    await runSessionTurn(client as unknown as DaemonClient, id, "hi");

    expect(order.slice(0, 2)).toEqual(["subscribe", "run"]);
  });

  it("stale events from an older run do not end our turn (reconnect filter)", async () => {
    // Successor of the SSE `after=<lastSeq>` resubscribe test: the channel
    // now owns cursor resume (covered in wsChannel.test); what must hold at
    // THIS layer is that a replayed/repeated event stream cannot smuggle in
    // a terminal event from another run id.
    const stale = { ...makeEvent(2, "turn_done", { finish_reason: "stop" }), run_id: "old-run" };
    const client = fakeClient({
      events: [
        makeEvent(1, "content_delta", { text: "partial" }),
        stale,
        makeEvent(2, "content_delta", { text: " rest" }),
        makeEvent(3, "turn_done", { finish_reason: "stop" }),
      ],
    });
    const id = useSessionManager.getState().createLocalSession("s1");
    await runSessionTurn(client as unknown as DaemonClient, id, "x");

    const store = useSessionManager.getState().entries[id].store.getState();
    expect(store.messages.some((m) => m.content === "partial rest")).toBe(true);
    expect(store.isRunning).toBe(false);
    expect(useSessionManager.getState().entries[id].status).toBe("idle");
  });

  it("completes the turn even though the channel keeps delivering after terminal", async () => {
    // Successor of the "daemon never closes the stream" regression: the ws
    // channel stays open by design; the observer must resolve on turn_done,
    // not on connection end — trailing events are simply not consumed.
    const late = makeEvent(3, "content_delta", { text: "late" });
    const client = fakeClient({
      events: [
        makeEvent(1, "content_delta", { text: "answer" }),
        makeEvent(2, "turn_done", { finish_reason: "stop" }),
        late,
      ],
    });
    const id = useSessionManager.getState().createLocalSession("s1");
    await runSessionTurn(client as unknown as DaemonClient, id, "x");

    const store = useSessionManager.getState().entries[id].store.getState();
    expect(store.messages.some((m) => m.content === "answer")).toBe(true);
    expect(store.messages.some((m) => m.content === "late")).toBe(false);
    expect(store.isRunning).toBe(false);
    expect(useSessionManager.getState().entries[id].status).toBe("idle");
  });

  it("captures turn_context published after turn_done (grace window)", async () => {
    // The daemon publishes turn_context AFTER turn_done (final save first).
    // The observer must keep its subscription through a short grace window,
    // consume that snapshot, and settle early — not drop it by unsubscribing
    // on turn_done.
    const client = fakeClient({
      events: [
        makeEvent(1, "content_delta", { text: "answer" }),
        makeEvent(2, "turn_done", { finish_reason: "stop" }),
        makeEvent(3, "turn_context", {
          layers: [],
          recalled_memories: [],
          new_messages: [],
          reminder: null,
          usage: {
            prompt_tokens: 900,
            completion_tokens: 100,
            total_tokens: 1000,
            context_tokens: 900,
          },
        }),
      ],
    });
    const id = useSessionManager.getState().createLocalSession("s1");
    await runSessionTurn(client as unknown as DaemonClient, id, "x");

    const store = useSessionManager.getState().entries[id].store.getState();
    expect(store.turnContext?.usage.context_tokens).toBe(900);
    // The turn-end snapshot also syncs the live occupancy field.
    expect(store.contextTokens).toBe(900);
    expect(store.isRunning).toBe(false);
  });

  it("usage_update events set contextTokens live mid-turn", async () => {
    // Real-time context occupancy: each LLM call pushes usage_update; the
    // store must reflect it without waiting for the turn-end snapshot.
    const client = fakeClient({
      events: [
        makeEvent(1, "usage_update", { prompt_tokens: 4321 }),
        makeEvent(2, "turn_done", { finish_reason: "stop" }),
      ],
    });
    const id = useSessionManager.getState().createLocalSession("s1");
    await runSessionTurn(client as unknown as DaemonClient, id, "x");

    const store = useSessionManager.getState().entries[id].store.getState();
    expect(store.contextTokens).toBe(4321);
  });
});
