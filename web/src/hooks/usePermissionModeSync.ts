import { useEffect } from "react";
import type { DaemonClient } from "../api/client";
import { wsChannel } from "../api/wsChannel";
import { useSessionManager } from "../state/sessionManager";

/** Keep each entry's effective model/mode aligned with daemon truth. Activation
 *  and reconnect both refetch; cleanup discards responses belonging to a
 *  previously active session. Global events carry their own routed session id. */
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
    if (!client || !activeId || !routedId || connection !== "connected") return;
    let cancelled = false;

    client
      .listModels(routedId)
      .then((response) => {
        if (cancelled) return;
        for (const provider of response.providers) {
          const model = provider.models.find((candidate) => candidate.active);
          if (model) {
            useSessionManager.getState().setSessionModel(activeId, {
              provider: provider.key,
              model: model.name,
              isOverride: response.is_override === true,
            });
            return;
          }
        }
        useSessionManager.getState().setSessionModel(activeId, null);
      })
      .catch(() => {
        // Non-fatal: retain the last known value until the next activation.
      });

    client
      .getPermissionMode(routedId)
      .then((pm) => {
        if (!cancelled && pm.mode) {
          useSessionManager.getState().setSessionMode(activeId, {
            mode: pm.mode,
            isOverride: pm.is_override === true,
          });
        }
      })
      .catch(() => {
        // Non-fatal: retain the last known value until the next activation.
      });
    return () => {
      cancelled = true;
    };
  }, [client, activeId, routedId, connection]);

  useEffect(() => {
    return wsChannel.subscribeGlobal((ev) => {
      if (ev.kind === "mode_changed") {
        useSessionManager.getState().applyModeChanged(ev.data);
      }
    });
  }, []);
}
