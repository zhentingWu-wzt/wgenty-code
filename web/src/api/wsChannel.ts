/**
 * Singleton WebSocket push channel (sse-to-websocket design D4).
 *
 * One multiplexed connection carries every daemon push: trace events, global
 * events, and per-session run events (`subscribe` control messages gate the
 * session stream). This replaces the four permanent SSE connections
 * (heartbeat, trace, global, per-run session) that each consumed a slot of
 * the browser's ~6-connections-per-origin HTTP/1.1 budget — the root cause
 * of `stream connect timed out` under subagent-heavy fan-out.
 *
 * Connection discipline mirrors the SSE consumers (`usePermissionTrace`):
 * exponential backoff 1s → 30s, reset on every successful open. URL
 * resolution mirrors `fetchStream`: direct daemon origin first (`ws://`
 * derived from `resolveDaemonDirect`, token in the `?token=` query because
 * browser WebSocket APIs cannot set headers), same-origin fallback second
 * (vite proxy forwards the upgrade and injects auth).
 *
 * After a reconnect the channel replays every live session subscription with
 * its tracked cursor (`after=`), then fires the `onReconnected` callbacks —
 * trace consumers run their `traceReplay` gap fill there (design D4).
 */

import { resolveDaemonDirect, type DaemonDirectInfo } from "./client";
import type { GlobalEvent, SessionEvent, TraceEvent } from "./types";

/** Downstream envelope (design D2, mirrors `DownstreamEnvelope` in ws_push.rs). */
type DownstreamEnvelope =
  | { type: "heartbeat" }
  | { type: "trace"; event: TraceEvent }
  | { type: "global"; event: GlobalEvent }
  | { type: "session"; session_id: string; event: SessionEvent }
  | { type: "subscribed"; session_id: string; latest_seq: number }
  | { type: "error"; message: string };

/** Upstream control message (design D2, mirrors `ClientMessage`). */
type ClientMessage =
  | { op: "subscribe"; session_id: string; after?: number }
  | { op: "unsubscribe"; session_id: string };

export type WsChannelStatus = "idle" | "connecting" | "open" | "backoff";

export interface SessionSubscription {
  unsubscribe(): void;
}

export interface WsChannel {
  /** Start the connection loop. Idempotent; the channel lives for the page. */
  connect(): void;
  status(): WsChannelStatus;
  /** Live trace events (global stream — filter by session in the handler). */
  subscribeTrace(handler: (event: TraceEvent) => void): () => void;
  /** Live global events (todos, task-group results, mode/model changes). */
  subscribeGlobal(handler: (event: GlobalEvent) => void): () => void;
  /**
   * Subscribe to a session's run events. Ref-counted: parallel subscribers
   * share ONE server-side subscription. `after` seeds the replay cursor for
   * the first subscriber; afterwards the channel tracks the highest seen
   * seq (advanced by `sync_lost`'s `data.latest_seq` per the recovery
   * convention) and resubscribes from it on reconnect, so missed events
   * replay without duplicates.
   */
  subscribeSession(
    sessionId: string,
    handler: (event: SessionEvent) => void,
    opts?: { after?: number },
  ): SessionSubscription;
  /** Fired after (re)connect once all session subscriptions are restored —
   *  trace consumers run their `traceReplay` gap fill here. */
  onReconnected(callback: () => void): () => void;
}

/** Injectable seams so unit tests can drive the state machine with a fake
 *  socket, URL resolver, and clock-free waits (via microtasks). */
export interface WsChannelOptions {
  resolveDirect?: () => Promise<DaemonDirectInfo | null>;
  createSocket?: (url: string) => WebSocket;
  sameOriginWsUrl?: () => string;
  initialBackoffMs?: number;
  maxBackoffMs?: number;
  log?: (...args: unknown[]) => void;
}

interface SessionSubEntry {
  handlers: Set<(event: SessionEvent) => void>;
  /** Replay cursor: the `after` seed, then the highest seq delivered (or the
   *  `sync_lost` realign). Resubscribes resume from it after a reconnect. */
  cursor: number | undefined;
}

function defaultSameOriginWsUrl(): string {
  const proto = window.location.protocol === "https:" ? "wss" : "ws";
  return `${proto}://${window.location.host}/api/v1/ws`;
}

export function createWsChannel(options: WsChannelOptions = {}): WsChannel {
  const {
    resolveDirect = resolveDaemonDirect,
    createSocket = (url) => new WebSocket(url),
    sameOriginWsUrl = defaultSameOriginWsUrl,
    initialBackoffMs = 1_000,
    maxBackoffMs = 30_000,
    log = console.warn.bind(console),
  } = options;

  const traceHandlers = new Set<(event: TraceEvent) => void>();
  const globalHandlers = new Set<(event: GlobalEvent) => void>();
  const sessionSubs = new Map<string, SessionSubEntry>();
  const reconnectedCbs = new Set<() => void>();

  let running = false;
  let status: WsChannelStatus = "idle";
  let socket: WebSocket | null = null;
  let closeWaiter: (() => void) | null = null;
  /** Set when the current socket closed before `awaitClose` hung a waiter
   *  (close can fire between the open resolution and the loop resuming). */
  let socketClosed = false;

  const send = (msg: ClientMessage): void => {
    if (socket?.readyState === WebSocket.OPEN) socket.send(JSON.stringify(msg));
  };

  const handleMessage = (text: string): void => {
    let env: DownstreamEnvelope;
    try {
      env = JSON.parse(text) as DownstreamEnvelope;
    } catch {
      return;
    }
    switch (env.type) {
      case "heartbeat":
        return;
      case "trace":
        for (const h of traceHandlers) h(env.event);
        return;
      case "global":
        for (const h of globalHandlers) h(env.event);
        return;
      case "session": {
        const entry = sessionSubs.get(env.session_id);
        if (!entry) return;
        const seq = env.event.seq;
        if (seq > 0) {
          // Duplicate guard: the server dedups the replay/live seam, this
          // protects against any re-delivery racing a resubscribe.
          if (entry.cursor !== undefined && seq <= entry.cursor) return;
          entry.cursor = seq;
        } else if (env.event.kind === "sync_lost") {
          // Recovery convention: realign to data.latest_seq so the next
          // resubscribe starts from the server's present, not the stale gap.
          const latest = env.event.data?.["latest_seq"];
          if (typeof latest === "number") entry.cursor = latest;
        }
        for (const h of entry.handlers) h(env.event);
        return;
      }
      case "subscribed":
        // Ack is an anchor only; replay events (if any) advance the cursor.
        return;
      case "error":
        log("ws: control error:", env.message);
        return;
    }
  };

  /** One socket attempt. Resolves `true` once open (stays attached for the
   *  connection's lifetime via `awaitClose`), `false` if it closed first. */
  const attempt = (url: string): Promise<boolean> =>
    new Promise((resolve) => {
      const sock = createSocket(url);
      let opened = false;
      sock.onopen = () => {
        opened = true;
        socketClosed = false;
        socket = sock;
        resolve(true);
      };
      sock.onclose = (ev) => {
        if (socket === sock) socket = null;
        if (!opened) {
          resolve(false);
          return;
        }
        socketClosed = true;
        if (ev.code === 4001) {
          // Token rotated server-side; the reconnect loop re-resolves the
          // direct info (fresh token) before the next attempt.
          log("ws: closed by server (token rotated); reconnecting…");
        }
        closeWaiter?.();
        closeWaiter = null;
      };
      sock.onmessage = (ev) => {
        if (typeof ev.data === "string") handleMessage(ev.data);
      };
    });

  const awaitClose = (): Promise<void> =>
    new Promise((resolve) => {
      if (socketClosed) {
        resolve();
        return;
      }
      closeWaiter = resolve;
    });

  /** Direct origin first (fresh resolution picks up daemon restarts), then
   *  the same-origin fallback — mirrors `fetchStream`'s strategy. */
  const tryConnect = async (): Promise<boolean> => {
    let direct: DaemonDirectInfo | null = null;
    try {
      direct = await resolveDirect();
    } catch {
      direct = null;
    }
    if (direct) {
      const url = `${direct.base.replace(/^http/, "ws")}/ws?token=${encodeURIComponent(direct.token)}`;
      if (await attempt(url)) return true;
      log("ws: direct connect failed; falling back to same-origin proxy");
    }
    return attempt(sameOriginWsUrl());
  };

  const resubscribeAll = (): void => {
    for (const [sessionId, entry] of sessionSubs) {
      send({ op: "subscribe", session_id: sessionId, after: entry.cursor });
    }
  };

  const run = async (): Promise<void> => {
    let backoff = initialBackoffMs;
    while (running) {
      status = "connecting";
      if (await tryConnect()) {
        if (!running) return;
        status = "open";
        backoff = initialBackoffMs;
        resubscribeAll();
        for (const cb of reconnectedCbs) {
          try {
            cb();
          } catch (err) {
            log("ws: reconnected callback threw:", err);
          }
        }
        await awaitClose();
      }
      if (!running) return;
      status = "backoff";
      await new Promise((resolve) => setTimeout(resolve, backoff));
      backoff = Math.min(backoff * 2, maxBackoffMs);
    }
  };

  return {
    connect() {
      if (running) return;
      running = true;
      void run();
    },
    status: () => status,
    subscribeTrace(handler) {
      traceHandlers.add(handler);
      return () => {
        traceHandlers.delete(handler);
      };
    },
    subscribeGlobal(handler) {
      globalHandlers.add(handler);
      return () => {
        globalHandlers.delete(handler);
      };
    },
    subscribeSession(sessionId, handler, opts) {
      let entry = sessionSubs.get(sessionId);
      if (!entry) {
        entry = { handlers: new Set(), cursor: opts?.after };
        sessionSubs.set(sessionId, entry);
        // The subscribe rides the next reconnect if the socket is down.
        send({ op: "subscribe", session_id: sessionId, after: opts?.after });
      }
      entry.handlers.add(handler);
      return {
        unsubscribe: () => {
          const current = sessionSubs.get(sessionId);
          if (!current) return;
          current.handlers.delete(handler);
          if (current.handlers.size === 0) {
            sessionSubs.delete(sessionId);
            send({ op: "unsubscribe", session_id: sessionId });
          }
        },
      };
    },
    onReconnected(callback) {
      reconnectedCbs.add(callback);
      return () => {
        reconnectedCbs.delete(callback);
      };
    },
  };
}

/** Page-level singleton. Connects lazily on the first `connect()` call
 *  (App mount); lives until the tab closes. */
export const wsChannel: WsChannel = createWsChannel();
