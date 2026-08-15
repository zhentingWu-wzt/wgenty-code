import { useEffect } from "react";
import type { DaemonClient } from "../api/client";
import { wsChannel } from "../api/wsChannel";
import type { TraceEvent } from "../api/types";
import { useSessionManager } from "../state/sessionManager";
import { useSubagentTraceStore } from "../state/subagentTraceStore";

/**
 * Subscribe to the daemon's trace stream and surface subagent permission
 * prompts as they arrive (design D2.1: push, not poll).
 *
 * ONE global live subscription on the shared WebSocket push channel (no
 * session filter): live progress + permission / question prompts for EVERY
 * session. Cold-start recovery uses one-shot REST instead of a second
 * session-scoped stream:
 * - `GET /tools/pending-permissions` re-surfaces still-blocked prompts after
 *   any (re)connect (pending state otherwise lives only in page memory);
 * - `GET /subagents/trace/replay?session_id=…` replays the active session's
 *   persisted transcript headers (terminal TraceEvent.result) on session
 *   switch and reconnect.
 * The connection itself (connect/backoff/resubscribe) is owned by the
 * wsChannel singleton — this hook only registers handlers, so it adds zero
 * transport connections to the browser's per-origin budget.
 *
 * On `permission_pending` we push the approval into the session store the
 * event's `session_id` points to (falling back to the active session — daemon
 * session ids don't always match local session ids); the PermissionModal
 * renders it and, on user choice, calls `client.resolveSubagentPermission`.
 * `permission_resolved` events clear a prompt answered elsewhere.
 *
 * RECONNECT: the channel owns the backoff loop; on every (re)open it fires
 * onReconnected, where we re-run both recovery paths. Without this, a daemon
 * restart would permanently and silently kill the subagent permission-push
 * channel — the agent would appear to hang while waiting for a prompt that
 * never surfaces.
 */
export function usePermissionTrace(client: DaemonClient | null): void {
  // Session switches re-run the one-shot replay effect below — they must NOT
  // re-register the permanent subscription (handlers are session-agnostic).
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

  // One-shot recovery for the active session: replay persisted transcript
  // headers (terminal TraceEvent.result). Runs on mount, on every session
  // switch, and after every channel (re)connect — the upsert in the trace
  // store is idempotent (keyed by node), so a full replay doubles as the
  // reconnect gap fill.
  const replayActiveSession = () => {
    const sid = useSessionManager.getState().activeId;
    if (!sid) return;
    const daemonId = useSessionManager.getState().entries[sid]?.daemonId ?? sid;
    void (async () => {
      try {
        for (const ev of await client!.traceReplay(daemonId)) handleEvent(ev);
      } catch {
        // Best-effort; retried on the next reconnect / session switch.
      }
    })();
  };

  useEffect(() => {
    if (!client) return;
    replayActiveSession();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [client, activeId]);

  useEffect(() => {
    if (!client) return;

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

    const unsubTrace = wsChannel.subscribeTrace(handleEvent);
    const unsubReconnect = wsChannel.onReconnected(() => {
      void recoverPendingPermissions();
      replayActiveSession();
    });
    wsChannel.connect();

    return () => {
      unsubTrace();
      unsubReconnect();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [client]);
}
