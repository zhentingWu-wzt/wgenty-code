import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createWsChannel, type WsChannel } from "./wsChannel";
import type { SessionEvent, TraceEvent } from "./types";

/** Controllable WebSocket stand-in: tests drive open/close/server frames. */
class FakeWebSocket {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSING = 2;
  static readonly CLOSED = 3;

  static instances: FakeWebSocket[] = [];
  static get last(): FakeWebSocket {
    return FakeWebSocket.instances[FakeWebSocket.instances.length - 1];
  }

  readonly url: string;
  readyState = FakeWebSocket.CONNECTING;
  onopen: (() => void) | null = null;
  onclose: ((ev: { code: number }) => void) | null = null;
  onmessage: ((ev: { data: string }) => void) | null = null;
  sent: string[] = [];

  constructor(url: string) {
    this.url = url;
    FakeWebSocket.instances.push(this);
  }

  send(data: string): void {
    this.sent.push(data);
  }

  // ── test drivers ──────────────────────────────────────────────────────────
  open(): void {
    this.readyState = FakeWebSocket.OPEN;
    this.onopen?.();
  }

  close(code = 1006): void {
    this.readyState = FakeWebSocket.CLOSED;
    this.onclose?.({ code });
  }

  serverSend(env: unknown): void {
    this.onmessage?.({ data: JSON.stringify(env) });
  }

  sentJson(): unknown[] {
    return this.sent.map((s) => JSON.parse(s));
  }
}

const DIRECT = { base: "http://127.0.0.1:9999/api/v1", token: "tok-1" };

/** Drain deep microtask chains (resolve → await → resolve …): a single
 *  timer-advance tick does not reach the channel's async loop. */
async function flush(): Promise<void> {
  for (let i = 0; i < 20; i++) await Promise.resolve();
}

/** Open the (already created) socket and let the channel loop settle. */
async function openAndSettle(): Promise<void> {
  FakeWebSocket.last.open();
  await flush();
}

/** Close the open socket and let the channel loop observe the drop. */
async function closeAndSettle(code = 1006): Promise<void> {
  FakeWebSocket.last.close(code);
  await flush();
}

function makeChannel(resolveDirect = vi.fn().mockResolvedValue(DIRECT)): WsChannel {
  return createWsChannel({
    resolveDirect,
    createSocket: (url) => new FakeWebSocket(url) as unknown as WebSocket,
    sameOriginWsUrl: () => "ws://localhost:5173/api/v1/ws",
    log: () => {},
  });
}

async function startChannel(resolveDirect?: ReturnType<typeof vi.fn>): Promise<WsChannel> {
  const ch = makeChannel(resolveDirect);
  ch.connect();
  await flush();
  await openAndSettle();
  return ch;
}

function sessionEvent(sessionId: string, seq: number): { type: string } & Record<string, unknown> {
  return {
    type: "session",
    session_id: sessionId,
    event: {
      seq,
      session_id: sessionId,
      run_id: "run-1",
      kind: "content_delta",
      data: {},
    },
  };
}

describe("wsChannel connection state machine", () => {
  beforeEach(() => {
    FakeWebSocket.instances = [];
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("connects direct-first with the token in the query, then reports open", async () => {
    const ch = makeChannel();
    ch.connect();
    await flush();

    expect(FakeWebSocket.last.url).toBe("ws://127.0.0.1:9999/api/v1/ws?token=tok-1");
    expect(ch.status()).toBe("connecting");

    await openAndSettle();
    expect(ch.status()).toBe("open");
  });

  it("falls back to the same-origin URL when the direct socket fails to open", async () => {
    const ch = makeChannel();
    ch.connect();
    await flush();

    await closeAndSettle(); // direct attempt failed pre-open
    await flush();

    expect(FakeWebSocket.last.url).toBe("ws://localhost:5173/api/v1/ws");
    await openAndSettle();
    expect(ch.status()).toBe("open");
  });

  it("reconnects after a drop and resubscribes from the tracked cursor", async () => {
    const ch = makeChannel();
    const events: number[] = [];
    ch.subscribeSession("s1", (ev: SessionEvent) => events.push(ev.seq), { after: 3 });
    ch.connect();
    await flush();
    await openAndSettle();

    // Initial resubscribe (subscribe pre-dated the connection) used the seed.
    expect(FakeWebSocket.last.sentJson()).toEqual([
      { op: "subscribe", session_id: "s1", after: 3 },
    ]);

    FakeWebSocket.last.serverSend(sessionEvent("s1", 4));
    FakeWebSocket.last.serverSend(sessionEvent("s1", 5));
    expect(events).toEqual([4, 5]);

    await closeAndSettle();
    expect(ch.status()).toBe("backoff");
    await vi.advanceTimersByTimeAsync(1_000);
    await flush();

    await openAndSettle();
    expect(FakeWebSocket.last.sentJson()).toEqual([
      { op: "subscribe", session_id: "s1", after: 5 },
    ]);
  });

  it("doubles the backoff while the daemon stays down", async () => {
    const ch = makeChannel(vi.fn().mockResolvedValue(null)); // no direct info
    ch.connect();
    await flush();
    await closeAndSettle(); // same-origin attempt failed

    await vi.advanceTimersByTimeAsync(999);
    expect(FakeWebSocket.instances.length, "still inside the initial 1s backoff").toBe(1);
    await vi.advanceTimersByTimeAsync(1);
    await flush();
    expect(FakeWebSocket.instances.length, "retried after 1s").toBe(2);
    await closeAndSettle();

    await vi.advanceTimersByTimeAsync(1_999);
    expect(FakeWebSocket.instances.length, "doubled backoff (2s) not yet elapsed").toBe(2);
    await vi.advanceTimersByTimeAsync(1);
    await flush();
    expect(FakeWebSocket.instances.length).toBe(3);
  });
});

describe("wsChannel ref-counted session subscriptions", () => {
  beforeEach(() => {
    FakeWebSocket.instances = [];
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("shares one server subscription across parallel subscribers; unsubscribes when the last leaves", async () => {
    const ch = await startChannel();
    const a: number[] = [];
    const b: number[] = [];
    const subA = ch.subscribeSession("s1", (ev: SessionEvent) => a.push(ev.seq));
    const subB = ch.subscribeSession("s1", (ev: SessionEvent) => b.push(ev.seq));

    expect(FakeWebSocket.last.sentJson()).toEqual([{ op: "subscribe", session_id: "s1" }]);

    subA.unsubscribe();
    expect(FakeWebSocket.last.sentJson(), "refcount > 0: no unsubscribe sent").toHaveLength(1);

    FakeWebSocket.last.serverSend(sessionEvent("s1", 1));
    expect(a).toEqual([]);
    expect(b, "remaining subscriber still receives events").toEqual([1]);

    subB.unsubscribe();
    expect(FakeWebSocket.last.sentJson()).toEqual([
      { op: "subscribe", session_id: "s1" },
      { op: "unsubscribe", session_id: "s1" },
    ]);

    FakeWebSocket.last.serverSend(sessionEvent("s1", 2));
    expect(b, "no events after the last unsubscribe").toEqual([1]);
  });

  it("drops duplicate seqs at or below the cursor (seam dedup)", async () => {
    const ch = await startChannel();
    const seen: number[] = [];
    ch.subscribeSession("s1", (ev: SessionEvent) => seen.push(ev.seq), { after: 3 });

    FakeWebSocket.last.serverSend(sessionEvent("s1", 3)); // at cursor → dropped
    FakeWebSocket.last.serverSend(sessionEvent("s1", 4));
    FakeWebSocket.last.serverSend(sessionEvent("s1", 4)); // duplicate → dropped
    FakeWebSocket.last.serverSend(sessionEvent("s1", 5));
    expect(seen).toEqual([4, 5]);
  });

  it("realigns the cursor from sync_lost data.latest_seq", async () => {
    const ch = await startChannel();
    const kinds: string[] = [];
    ch.subscribeSession("s1", (ev: SessionEvent) => kinds.push(ev.kind), { after: 2 });

    FakeWebSocket.last.serverSend({
      type: "session",
      session_id: "s1",
      event: {
        seq: 0,
        session_id: "s1",
        run_id: "",
        kind: "sync_lost",
        data: { reason: "evicted", latest_seq: 42 },
      },
    });
    expect(kinds, "sync_lost itself is delivered to the handler").toEqual(["sync_lost"]);

    await closeAndSettle();
    await vi.advanceTimersByTimeAsync(1_000);
    await flush();
    await openAndSettle();

    expect(FakeWebSocket.last.sentJson()).toEqual([
      { op: "subscribe", session_id: "s1", after: 42 },
    ]);
  });
});

describe("wsChannel envelope dispatch", () => {
  beforeEach(() => {
    FakeWebSocket.instances = [];
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("dispatches trace and global envelopes by type and ignores heartbeat", async () => {
    const ch = await startChannel();
    const traces: TraceEvent[] = [];
    const globals: string[] = [];
    ch.subscribeTrace((ev) => traces.push(ev));
    ch.subscribeGlobal((ev) => globals.push(ev.kind));

    FakeWebSocket.last.serverSend({ type: "heartbeat" });
    FakeWebSocket.last.serverSend({
      type: "trace",
      event: { ts: 1, session_id: "s1", node_id: "n1", label: "l", status: "ok" },
    });
    FakeWebSocket.last.serverSend({
      type: "global",
      event: { seq: 1, kind: "todos_changed", data: {} },
    });

    expect(traces).toHaveLength(1);
    expect(traces[0].node_id).toBe("n1");
    expect(globals).toEqual(["todos_changed"]);
  });

  it("fires onReconnected after session subscriptions are restored", async () => {
    const order: string[] = [];
    const ch = makeChannel();
    ch.subscribeSession("s1", () => {});
    ch.onReconnected(() => order.push("reconnected"));
    ch.connect();
    await flush();
    await openAndSettle();
    order.push("open");

    await closeAndSettle();
    await vi.advanceTimersByTimeAsync(1_000);
    await flush();
    await openAndSettle();

    expect(order).toEqual([
      "reconnected", // fires on the FIRST open too (cold-start replay gap fill)
      "open",
      "reconnected", // and again after the reconnect
    ]);
    // The resubscribe was sent before the callback ran.
    expect(FakeWebSocket.last.sentJson()).toContainEqual({ op: "subscribe", session_id: "s1" });
  });
});
