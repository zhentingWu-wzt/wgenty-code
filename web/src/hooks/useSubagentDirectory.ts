import { useEffect, useRef } from "react";
import type { DaemonClient } from "../api/client";
import { useSessionManager } from "../state/sessionManager";
import { useSubagentDirectoryStore } from "../state/subagentDirectoryStore";

/**
 * Poll the daemon's agent directory (`GET /agents/directory`) and cache the
 * per-session tree in `subagentDirectoryStore` (design: poll, not push — the
 * directory endpoint is lightweight and always available, unlike the
 * SSE stream which stays silent unless subagents emit trace events).
 *
 * Semantics:
 * - Fixed 3s cadence (no backoff — a missed poll is cheap to retry); skipped
 *   entirely while the tab is hidden (`document.hidden`).
 * - Session id follows the App.tsx precedent: `daemonId ?? entry.id`, so
 *   bound sessions poll their daemon identity and local sessions fall back
 *   to the local id. The id is re-resolved on every tick because `daemonId`
 *   is assigned asynchronously when a local session lands on the daemon.
 * - Consecutive failures are tolerated silently; after 3 in a row the cached
 *   tree is flagged `stale` (kept for display). A single success clears it.
 * - Ticks never throw: polling must not take the app down with it.
 */
const POLL_INTERVAL_MS = 3000;
const MAX_CONSECUTIVE_FAILURES = 3;

export function useSubagentDirectory(client: DaemonClient | null): void {
  // Reactive subscription to the active session: the poll effect re-runs
  // (clear interval + immediate tick) whenever the user switches sessions.
  const activeId = useSessionManager((s) => s.activeId);
  // Survives interval restarts; reset on session switch and on any success.
  const failuresRef = useRef(0);

  useEffect(() => {
    if (!client || !activeId) return;
    const directory = useSubagentDirectoryStore.getState();
    failuresRef.current = 0;

    const tick = async (): Promise<void> => {
      if (document.hidden) return;
      const m = useSessionManager.getState();
      const entry = m.activeId ? m.entries[m.activeId] : null;
      // daemonId when the session is bound to the daemon, else local id
      // (same precedence as App.tsx's permission-mode routing).
      const sid = entry ? (entry.daemonId ?? entry.id) : null;
      if (!sid) return;
      try {
        const res = await client.getAgentDirectory(sid);
        directory.apply(sid, res);
        failuresRef.current = 0;
      } catch {
        failuresRef.current += 1;
        if (failuresRef.current >= MAX_CONSECUTIVE_FAILURES) {
          directory.markStale(sid);
        }
      }
    };

    // Immediate tick on mount / session switch, then a fixed-interval poll.
    void tick();
    const timer = setInterval(() => void tick(), POLL_INTERVAL_MS);
    return () => clearInterval(timer);
  }, [client, activeId]);
}
