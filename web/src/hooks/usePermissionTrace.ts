import { useEffect } from "react";
import type { DaemonClient } from "../api/client";
import type { TraceEvent } from "../api/types";
import { useSessionManager } from "../state/sessionManager";
import { useSubagentTraceStore } from "../state/subagentTraceStore";

/**
 * Subscribe to the daemon's trace SSE stream and surface subagent permission
 * prompts as they arrive (design D2.1: push, not poll).
 *
 * ONE global live stream (no session filter): live progress + permission /
 * question prompts for EVERY session. Cold-start recovery uses one-shot REST
 * instead of a second session-scoped stream:
 * - `GET /tools/pending-permissions` re-surfaces still-blocked prompts after
 *   any (re)connect (pending state otherwise lives only in page memory);
 * - `GET /subagents/trace/replay?session_id=…` replays the active session's
 *   persisted transcript headers (terminal TraceEvent.result) on session
 *   switch and reconnect.
 * Every permanent SSE connection counts against the browser's ~6 per-origin
 * HTTP/1.1 connection budget — the old two-stream design (plus heartbeat,
 * HMR websocket, session event stream) let two open tabs starve every other
 * request, wedging the page.
 *
 * On `permission_pending` we push the approval into the session store the
 * event's `session_id` points to (falling back to the active session — daemon
 * session ids don't always match local session ids); the PermissionModal
 * renders it and, on user choice, calls `client.resolveSubagentPermission`.
 * `permission_resolved` events clear a prompt answered elsewhere.
 *
 * RECONNECT (design D7.2): if the stream dies (daemon restart, network drop),
 * reconnect with exponential backoff (1s → 30s cap, reset on success). Without
 * this, a daemon restart would permanently and silently kill the subagent
 * permission-push channel — the agent would appear to hang while waiting for a
 * prompt that never surfaces.
 */
const INITIAL_BACKOFF_MS = 1000;
const MAX_BACKOFF_MS = 30_000;

export function usePermissionTrace(client: DaemonClient | null): void {
  // Session switches re-run the one-shot replay effect below — they must NOT
  // restart the permanent stream (a restarted stream whose fetch isn't
  // aborted leaks a connection from the per-origin budget).
  const activeId = useSessionManager((s) => s.activeId);

  // Route a trace event (or a recovered pending permission) to the session
  // store it belongs to, falling back to the active session — daemon session
  // ids don't always match local session ids.
  const routeTarget = (sessionId: string) => {
    const m = useSessionManager.getState();
    return m.entries[sessionId] ?? (m.activeId ? m.entries[m.activeId] : null);
  };

  const handleEvent = (ev: TraceEvent) => {
    // `progress` events update the subagent trace tree (consumed by
    // SubagentTreePanel). All other kinds are permission/question routing.
    if (ev.kind === "progress" || !ev.kind) {
      useSubagentTraceStore.getState().upsertFromEvent(ev);
    }

    const target = routeTarget(ev.session_id);
    if (!target) return;
    const m = useSessionManager.getState();
    if (ev.kind === "permission_pending" && ev.permission) {
      target.store.getState().pushSubagentPermission(ev.permission);
      m.setStatus(target.id, "awaiting_approval");
    } else if (ev.kind === "permission_resolved") {
      // Resolved elsewhere (timeout, or another client) — dismiss.
      target.store.getState().clearSubagentPermission();
      // Back to running only if nothing else is still awaiting a decision
      // (a root-tool prompt from the local loop may still be open).
      if (target.status === "awaiting_approval" && !target.store.getState().pendingPermission) {
        m.setStatus(target.id, target.store.getState().isRunning ? "running" : "idle");
      }
    } else if (ev.kind === "question_pending" && ev.question) {
      target.store.getState().pushQuestion(ev.question);
      m.setStatus(target.id, "awaiting_approval");
    } else if (ev.kind === "question_resolved") {
      target.store.getState().clearQuestion();
      if (target.status === "awaiting_approval") {
        m.setStatus(target.id, target.store.getState().isRunning ? "running" : "idle");
      }
    }
  };

  // One-shot cold-start recovery for the active session: replay persisted
  // transcript headers (terminal TraceEvent.result) on mount and on every
  // session switch. This replaces the old session-scoped SECOND SSE stream —
  // every permanent stream consumes one of the browser's ~6 per-origin
  // connections, and two app tabs were enough to starve every other request.
  useEffect(() => {
    if (!client) return;
    const sid = useSessionManager.getState().activeId;
    if (!sid) return;
    const daemonId = useSessionManager.getState().entries[sid]?.daemonId ?? sid;
    void (async () => {
      try {
        for (const ev of await client.traceReplay(daemonId)) handleEvent(ev);
      } catch {
        // Best-effort; retried on the next reconnect / session switch.
      }
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [client, activeId]);

  useEffect(() => {
    if (!client) return;
    let cancelled = false;
    // Aborting this controller is the ONLY reliable way to tear down the
    // in-flight SSE fetch on cleanup: `reader.read()` on a keepalive-only
    // stream never resolves, so a cleanup that merely flips a flag leaks the
    // connection forever — each leak burns one slot of the browser's ~6
    // per-origin budget until every new request starves behind them
    // ("stream connect timed out").
    const ctl = new AbortController();
    // Cleanup hooks for in-flight backoff waits: resolve them immediately so
    // the stream loops observe `cancelled` without waiting out the timer.
    const cancelWaiters: Array<() => void> = [];

    // Re-surface still-blocked permission prompts after a (re)connect:
    // pending state lives only in memory, so a page refresh or a reconnect
    // gap would otherwise leave the daemon's bridge waiting forever with no
    // modal to answer (the prompt simply never reappears).
    const recoverPendingPermissions = async () => {
      try {
        const { pending } = await client.listPendingPermissions();
        const m = useSessionManager.getState();
        for (const p of pending) {
          const target = routeTarget(p.from);
          if (!target) continue;
          target.store.getState().pushSubagentPermission(p);
          m.setStatus(target.id, "awaiting_approval");
        }
      } catch {
        // Best-effort; the next reconnect retries.
      }
    };

    // One long-lived loop: connect → read until error/EOF → backoff →
    // reconnect. Backoff resets to INITIAL on every successful connection.
    const runStream = async () => {
      let backoff = INITIAL_BACKOFF_MS;
      while (!cancelled) {
        let reader: ReadableStreamDefaultReader<Uint8Array> | null = null;
        try {
          const { body } = await client.traceStream(undefined, undefined, ctl.signal);
          if (cancelled) return;
          reader = body.getReader();
          // Connection succeeded — reset backoff.
          backoff = INITIAL_BACKOFF_MS;
          // The global stream doubles as the permission push channel; after
          // any (re)connect, re-surface prompts still blocked server-side.
          void recoverPendingPermissions();
          let buffer = "";
          for (;;) {
            const { done, value } = await reader.read();
            if (done) break;
            if (!value) continue;
            buffer += new TextDecoder().decode(value);
            let nl: number;
            while ((nl = buffer.indexOf("\n")) !== -1) {
              const line = buffer.slice(0, nl).trim();
              buffer = buffer.slice(nl + 1);
              if (!line || line.startsWith(":")) continue; // skip SSE comments/keepalives
              // The daemon uses standard SSE `data: {json}` framing; strip the
              // prefix before parsing (mirrors sessionRunner.ts).
              const payload = line.startsWith("data: ") ? line.slice(6) : line;
              try {
                handleEvent(JSON.parse(payload) as TraceEvent);
              } catch {
                // Keep-alive or partial; ignore unparseable lines.
              }
            }
          }
          // Stream ended cleanly (daemon closed). Reconnect after backoff.
        } catch {
          // Aborted by cleanup, or transient (daemon down, network reset).
          // Fall through to backoff.
        } finally {
          reader?.cancel().catch(() => {});
        }

        if (cancelled) return;
        // Exponential backoff before reconnecting. Await the timer as a
        // promise so cancellation resolves immediately.
        await new Promise<void>((resolve) => {
          const timer = setTimeout(() => resolve(), backoff);
          backoff = Math.min(backoff * 2, MAX_BACKOFF_MS);
          cancelWaiters.push(() => {
            clearTimeout(timer);
            resolve();
          });
        });
      }
    };

    // Global live stream — all sessions' live events + permission push.
    void runStream();

    return () => {
      cancelled = true;
      ctl.abort();
      for (const cancel of cancelWaiters) cancel();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [client]);
}
