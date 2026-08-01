import { useEffect, useState } from "react";
import { CircleAlert, MessagesSquare, Plus } from "lucide-react";
import type { DaemonClient } from "../api/client";
import type { SessionInfo } from "../api/types";
import { sessionMessagesToDisplay } from "../agent/sessionLoad";
import { useSessionManager } from "../state/sessionManager";

/**
 * Open sessions (live, with status) on top; daemon-saved sessions below —
 * clicking one loads its history into a new local entry.
 */
export function SessionList({ client }: { client: DaemonClient }) {
  const order = useSessionManager((s) => s.order);
  const entries = useSessionManager((s) => s.entries);
  const activeId = useSessionManager((s) => s.activeId);
  const [saved, setSaved] = useState<SessionInfo[]>([]);

  useEffect(() => {
    client
      .listSessions()
      .then(setSaved)
      .catch(() => setSaved([])); // daemon down → hide the section, not an error storm
  }, [client, order.length]); // refresh after autosave creates daemon sessions

  const openDaemonSession = async (info: SessionInfo) => {
    // Already open? Just focus it.
    const existing = Object.values(useSessionManager.getState().entries).find(
      (e) => e.daemonId === info.id,
    );
    if (existing) {
      useSessionManager.getState().setActive(existing.id);
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
  };

  return (
    <div className="session-list-panel">
      <div className="session-list-head">
        <span className="rail-section-title">
          <MessagesSquare size={12} /> Sessions
        </span>
        <button
          type="button"
          className="btn-xs"
          onClick={() => useSessionManager.getState().createLocalSession()}
        >
          <Plus size={12} /> New session
        </button>
      </div>
      <ul className="session-cards">
        {order.map((id) => {
          const e = entries[id];
          return (
            <li key={id}>
              <button
                type="button"
                className={`session-card ${id === activeId ? "active" : ""}`}
                onClick={() => useSessionManager.getState().setActive(id)}
              >
                <span className={`session-dot session-status-${e.status}`} />
                <span className="session-card-main">
                  <span className="session-card-name">{e.name}</span>
                  {e.lastPreview && <span className="session-card-preview">{e.lastPreview}</span>}
                </span>
                {e.status === "awaiting_approval" && (
                  <span className="session-card-badge">
                    <CircleAlert size={11} />
                  </span>
                )}
              </button>
            </li>
          );
        })}
      </ul>
      {saved.length > 0 && (
        <>
          <div className="rail-section-title">Saved</div>
          <ul className="session-cards">
            {saved.map((info) => (
              <li key={info.id}>
                <button
                  type="button"
                  className="session-card"
                  onClick={() => openDaemonSession(info)}
                >
                  <span className="session-card-main">
                    <span className="session-card-name">{info.name ?? info.id}</span>
                    <span className="session-card-preview">
                      {info.message_count} messages
                    </span>
                  </span>
                </button>
              </li>
            ))}
          </ul>
        </>
      )}
    </div>
  );
}
