import { useEffect } from "react";
import type { DaemonClient } from "../api/client";
import type { TraceEvent } from "../api/types";
import { useSessionManager } from "../state/sessionManager";

/**
 * Subscribe to the daemon's trace SSE stream and surface subagent permission
 * prompts as they arrive (design D2.1: push, not poll).
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
 * prompt that never surfaces. The reconnect loop is self-contained (not relying
 * on effect re-runs, since the deps are stable references).
 */
const INITIAL_BACKOFF_MS = 1000;
const MAX_BACKOFF_MS = 30_000;

export function usePermissionTrace(client: DaemonClient | null): void {
  useEffect(() => {
    if (!client) return;
    let cancelled = false;
    let reconnectTimer: ReturnType<typeof setTimeout> | null = null;

    const handleEvent = (ev: TraceEvent) => {
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
        if (
          target.status === "awaiting_approval" &&
          !target.store.getState().pendingPermission
        ) {
          m.setStatus(target.id, target.store.getState().isRunning ? "running" : "idle");
        }
      }
    };

    // One long-lived loop: connect → read until error/EOF → backoff → reconnect.
    // backoff resets to INITIAL on every successful connection.
    const run = async () => {
      let backoff = INITIAL_BACKOFF_MS;
      while (!cancelled) {
        let reader: ReadableStreamDefaultReader<Uint8Array> | null = null;
        try {
          const { body } = await client.traceStream();
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
              if (!line) continue;
              try {
                handleEvent(JSON.parse(line) as TraceEvent);
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
          reconnectTimer = setTimeout(() => {
            reconnectTimer = null;
            resolve();
          }, backoff);
          backoff = Math.min(backoff * 2, MAX_BACKOFF_MS);
        });
      }
    };

    run();
    return () => {
      cancelled = true;
      if (reconnectTimer) clearTimeout(reconnectTimer);
    };
  }, [client]);
}
