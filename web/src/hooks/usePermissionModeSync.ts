import { useCallback, useEffect, useRef } from "react";
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

  const requestGenerations = useRef(new Map<string, { model: number; mode: number }>());
  const refresh = useCallback(
    (id: string, route: string) => {
      if (!client) return;
      const manager = useSessionManager.getState();
      const modelRevision = manager.getSessionModelRevision(id);
      const modeRevision = manager.getSessionModeRevision(id);
      const generations = requestGenerations.current.get(id) ?? { model: 0, mode: 0 };
      requestGenerations.current.set(id, generations);
      const modelGeneration = ++generations.model;
      const modeGeneration = ++generations.mode;

      client
        .listModels(route)
        .then((response) => {
          if (requestGenerations.current.get(id)?.model !== modelGeneration) return;
          for (const provider of response.providers) {
            const model = provider.models.find((candidate) => candidate.active);
            if (model) {
              useSessionManager.getState().setSessionModelIfRevision(id, modelRevision, {
                provider: provider.key,
                model: model.name,
                isOverride: response.is_override === true,
              });
              return;
            }
          }
          useSessionManager.getState().setSessionModelIfRevision(id, modelRevision, null);
        })
        .catch(() => {
          // Non-fatal: retain the last known value until the next activation.
        });

      client
        .getPermissionMode(route)
        .then((pm) => {
          if (requestGenerations.current.get(id)?.mode !== modeGeneration || !pm.mode) return;
          useSessionManager.getState().setSessionModeIfRevision(id, modeRevision, {
            mode: pm.mode,
            isOverride: pm.is_override === true,
          });
        })
        .catch(() => {
          // Non-fatal: retain the last known value until the next activation.
        });
    },
    [client],
  );

  useEffect(() => {
    if (!activeId || !routedId || connection !== "connected") return;
    refresh(activeId, routedId);
    const generationsById = requestGenerations.current;
    return () => {
      const generations = generationsById.get(activeId);
      if (generations) {
        generations.model += 1;
        generations.mode += 1;
      }
    };
  }, [activeId, routedId, connection, refresh]);

  useEffect(() => {
    return wsChannel.onReconnected(() => {
      const manager = useSessionManager.getState();
      manager.resetModelModeEventSeq();
      const id = manager.activeId;
      if (!id) return;
      const entry = manager.entries[id];
      if (!entry) return;
      refresh(id, entry.daemonId ?? id);
    });
  }, [refresh]);

  useEffect(() => {
    return wsChannel.subscribeGlobal((ev) => {
      if (ev.kind === "mode_changed") {
        useSessionManager.getState().applyModeChanged(ev);
      }
    });
  }, []);
}
