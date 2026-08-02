import { useCallback, useEffect, useState } from "react";
import { Trash2 } from "lucide-react";
import type { DaemonClient } from "../api/client";
import type { SessionInfo } from "../api/types";
import { sessionMessagesToDisplay } from "../agent/sessionLoad";
import { useSessionManager } from "../state/sessionManager";
import { CommandModal } from "./CommandModal";

/**
 * `/sessions` browser: saved daemon sessions with open/delete. Opening loads
 * the history into a new local entry (bound to the same daemonId) and closes
 * the modal. Mirrors the TUI's `/session` browser (src/tui/app/input.rs:195).
 */
export function SessionsBrowserModal({
  client,
  onClose,
}: {
  client: DaemonClient;
  onClose: () => void;
}) {
  const [saved, setSaved] = useState<SessionInfo[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    client
      .listSessions()
      .then((s) => {
        setSaved(s);
        setError(null);
      })
      .catch((e) => setError(e instanceof Error ? e.message : String(e)));
  }, [client]);

  useEffect(refresh, [refresh]);

  const openSession = async (info: SessionInfo) => {
    // Already open? Just focus it.
    const existing = Object.values(useSessionManager.getState().entries).find(
      (e) => e.daemonId === info.id,
    );
    if (existing) {
      useSessionManager.getState().setActive(existing.id);
      onClose();
      return;
    }
    const full = await client.loadSession(info.id).catch(() => null);
    if (!full) return; // daemon down → stay put, don't open an empty entry
    const m = useSessionManager.getState();
    const localId = m.createLocalSession(info.name ?? "Session");
    m.setDaemonId(localId, info.id);
    const store = useSessionManager.getState().entries[localId].store;
    for (const dm of sessionMessagesToDisplay(full.messages ?? [])) {
      store.getState().pushLoadedMessage(dm);
    }
    onClose();
  };

  const deleteSession = async (info: SessionInfo) => {
    if (!window.confirm(`Delete session "${info.name ?? info.id}"?`)) return;
    try {
      await client.deleteSession(info.id);
      refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <CommandModal title="Saved sessions" onClose={onClose}>
      {error && <div className="panel-error">{error}</div>}
      {saved && saved.length === 0 && <div className="panel-empty">No saved sessions</div>}
      <ul className="session-cards">
        {(saved ?? []).map((info) => (
          <li key={info.id} className="saved-session-row">
            <button type="button" className="session-card" onClick={() => openSession(info)}>
              <span className="session-card-main">
                <span className="session-card-name">{info.name ?? info.id}</span>
                <span className="session-card-preview">{info.message_count} messages</span>
              </span>
            </button>
            <button
              type="button"
              className="btn-xs saved-session-delete"
              title={`Delete ${info.name ?? info.id}`}
              onClick={() => deleteSession(info)}
            >
              <Trash2 size={12} />
            </button>
          </li>
        ))}
      </ul>
    </CommandModal>
  );
}
