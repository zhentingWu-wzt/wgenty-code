import { CircleAlert, MessagesSquare, Plus } from "lucide-react";
import { useSessionManager } from "../state/sessionManager";

/**
 * Open sessions with live status. Saved-session browsing moved to the
 * `/sessions` command modal (SessionsBrowserModal), mirroring the TUI.
 */
export function SessionList() {
  const order = useSessionManager((s) => s.order);
  const entries = useSessionManager((s) => s.entries);
  const activeId = useSessionManager((s) => s.activeId);

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
    </div>
  );
}
