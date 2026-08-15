import { useEffect } from "react";
import type { DaemonClient } from "../api/client";
import type { TraceEvent } from "../api/types";
import { useSessionManager } from "../state/sessionManager";
import { useSubagentTraceStore } from "../state/subagentTraceStore";

/**
 * Subscribe to the daemon's trace SSE stream and surface subagent permission
 * prompts as they arrive (design D2.1: push, not poll).
 *
 * Two parallel streams, both routed through the same `handleEvent`:
 * 1. Global live stream (no session filter): live progress + permission /
 *    question prompts for EVERY session. This is the permission push channel
 *    and must not be scoped, or a background session's subagent prompt would
 *    never surface.
 * 2. Session-scoped stream (session_id = active session): enables cold-start
 *    replay of that session's persisted transcript headers, so terminal
 *    results (TraceEvent.result) that fired before connecting — page load,
 *    refresh, or a reconnect gap — are recovered. Without a session_id the
 *    daemon streams live-only and those events are permanently missed (root
 *    cause of "subagent finished but web never got the result"). Restarted
 *    when the active session changes. No `since` watermark: replayed headers
 *    are filtered by started_at, so a watermark taken from live event
 *    timestamps would filter out subagents that started earlier and completed
 *    during the gap — the exact case this stream recovers. Upserts are
 *    idempotent, so full replays are safe.
 *
 * On `permission_pending` we push the approval into the session store the
 * event's `session_id` points to (falling back to the active session — daemon
 * session ids don't always match local session ids); the PermissionModal
 * renders it and, on user choice, calls `client.resolveSubagentPermission`.
 * `permission_resolved` events clear a prompt answered elsewhere.
 *
 * RECONNECT (design D7.2): if a stream dies (daemon restart, network drop),
 * reconnect with exponential backoff (1s → 30s cap, reset on success). Without
 * this, a daemon restart would permanently and silently kill the subagent
 * permission-push channel — the agent would appear to hang while waiting for a
 * prompt that never surfaces.
 */
const INITIAL_BACKOFF_MS = 1000;
const MAX_BACKOFF_MS = 30_000;

export function usePermissionTrace(client: DaemonClient | null): void {
  // Replay (cold-start) only works on a session-scoped stream, so track the
  // active session to subscribe for. The primitive selector re-renders only on
  // active-session changes; the effect dependency below restarts the scoped
  // stream when it does.
  const activeId = useSessionManager((s) => s.activeId);

  useEffect(() => {
    if (!client) return;
    let cancelled = false;
    // Cleanup hooks for in-flight backoff waits: resolve them immediately so
    // the stream loops observe `cancelled` without waiting out the timer.
    const cancelWaiters: Array<() => void> = [];

    const handleEvent = (ev: TraceEvent) => {
      // `progress` events update the subagent trace tree (consumed by
      // SubagentTreePanel). All other kinds are permission/question routing.
      if (ev.kind === "progress" || !ev.kind) {
        useSubagentTraceStore.getState().upsertFromEvent(ev);
      }

      // Route by the trace event's session_id; fall back to the active session
      // when the id doesn't match a local session (subagent trace ids are
      // daemon-side and may not map 1:1 onto local sessions).
      const m = useSessionManager.getState();
      const target = m.entries[ev.session_id] ?? (m.activeId ? m.entries[m.activeId] : null);
      if (!target) return;
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

    // One long-lived loop per stream: connect → read until error/EOF → backoff
    // → reconnect. Backoff resets to INITIAL on every successful connection.
    // `sessionId` scopes the stream (which enables header replay); omitted, it
    // is the global live stream.
    const runStream = async (sessionId?: string) => {
      let backoff = INITIAL_BACKOFF_MS;
      while (!cancelled) {
        let reader: ReadableStreamDefaultReader<Uint8Array> | null = null;
        try {
          const { body } = await client.traceStream(sessionId);
          if (cancelled) return;
          reader = body.getReader();
          // Connection succeeded — reset backoff.
          backoff = INITIAL_BACKOFF_MS;
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
          // Transient (daemon down, network reset). Fall through to backoff.
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

    // 1. Global live stream — all sessions' live events + permission push.
    runStream();
    // 2. Session-scoped stream — cold-start replay for the active session.
    if (activeId) runStream(activeId);

    return () => {
      cancelled = true;
      for (const cancel of cancelWaiters) cancel();
    };
  }, [client, activeId]);
}
