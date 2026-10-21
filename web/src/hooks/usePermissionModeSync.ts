import { useEffect } from "react";
import type { DaemonClient } from "../api/client";
import type { PermissionMode } from "../api/types";
import { wsChannel } from "../api/wsChannel";
import { useSessionManager } from "../state/sessionManager";

const KNOWN_MODES: readonly string[] = ["normal", "accept_edits", "yolo"];

/**
 * Keep the StatusBar's permission-mode label aligned with daemon truth.
 *
 * The daemon stores modes per project in memory only, so every one of these
 * desync paths showed a stale label while runs silently fell back to Normal
 * ("mode 没有生效"):
 * 1. Daemon restart (idle auto-shutdown wipes the store) — the label kept the
 *    pre-restart value forever because it was only fetched once per page load.
 * 2. Switching the active session/project — the label is a single global
 *    field and was never re-fetched per session.
 * 3. Mode switches from other clients (TUI Shift+Tab, another tab) — the
 *    daemon broadcasts `mode_changed` on every switch, but the web ignored it.
 *
 * Fix: re-fetch on activation change AND on every connected transition of the
 * health poll (covers initial load + restart recovery), and adopt every
 * `mode_changed` broadcast. The StatusBar is a single indicator, so the latest
 * daemon-wide switch wins; switching tabs re-fetches for the new session.
 */
export function usePermissionModeSync(client: DaemonClient | null): void {
  const activeId = useSessionManager((s) => s.activeId);
  const daemonId = useSessionManager((s) =>
    s.activeId ? s.entries[s.activeId]?.daemonId : undefined,
  );
  const connection = useSessionManager((s) => s.connection);

  // Route by the active session: its daemon id once landed, the local id
  // before that (the daemon resolves unknown ids to the main working root —
  // which is also where not-yet-created sessions run).
  const routedId = daemonId ?? activeId;

  useEffect(() => {
    if (!client || !routedId || connection !== "connected") return;
    let cancelled = false;
    client
      .getPermissionMode(routedId)
      .then((pm) => {
        if (!cancelled && pm.mode) {
          useSessionManager.getState().setPermissionMode(pm.mode);
        }
      })
      .catch(() => {
        // Non-fatal: StatusBar falls back to "-" until a switch succeeds.
      });
    return () => {
      cancelled = true;
    };
  }, [client, routedId, connection]);

  useEffect(() => {
    const unsubscribe = wsChannel.subscribeGlobal((ev) => {
      if (ev.kind !== "mode_changed") return;
      const mode = ev.data["mode"];
      if (typeof mode === "string" && KNOWN_MODES.includes(mode)) {
        useSessionManager.getState().setPermissionMode(mode as PermissionMode);
      }
    });
    return unsubscribe;
  }, []);
}
