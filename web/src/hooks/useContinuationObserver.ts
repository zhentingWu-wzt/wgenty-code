import { useEffect } from "react";
import type { DaemonClient } from "../api/client";
import { wsChannel } from "../api/wsChannel";
import { observeDaemonRun } from "../agent/sessionRunner";

/**
 * Subscribe to the daemon-wide global event stream and attach live observers
 * to daemon-initiated runs.
 *
 * Why: the daemon's continuation scheduler claims ready subagent task groups
 * server-side and spawns a synthesis turn for the main agent (so subagent
 * results reach the main agent even when no polling client like the TUI is
 * attached). Web only renders runs it started itself, so without this hook
 * those synthesis turns — the subagent results reaching the main agent —
 * were invisible until a manual refresh.
 *
 * On `task_group_result` (broadcast by whoever claimed the group — daemon
 * scheduler or the TUI's claim endpoint) we attach `observeDaemonRun` to the
 * named session. One handler on the shared wsChannel singleton — the channel
 * owns connect/backoff/reconnect, so this adds no transport connections.
 *
 * `background_result` gets the same treatment: when a background task (e.g. a
 * monitored CI run) completes while its session is idle, the daemon scheduler
 * claims the result and spawns a daemon-owned continuation turn server-side.
 * Without attaching an observer here that turn — typically the assistant's
 * report on the finished task — was invisible in web until a manual refresh,
 * even though it was persisted.
 */
export function useContinuationObserver(client: DaemonClient | null): void {
  useEffect(() => {
    if (!client) return;

    const unsubGlobal = wsChannel.subscribeGlobal((ev) => {
      // `task_group_result` carries `session_id` at the payload top level;
      // `background_result` nests it inside `data.result.session_id` (the
      // serialized BackgroundResult, which may omit it for unowned tasks —
      // those have no session to observe).
      const result = ev.data["result"] as { session_id?: string } | undefined;
      const sessionId =
        ev.kind === "background_result"
          ? String(result?.session_id ?? "")
          : String(ev.data["session_id"] ?? "");
      if (sessionId) void observeDaemonRun(client, sessionId);
    });
    wsChannel.connect();

    return unsubGlobal;
  }, [client]);
}
