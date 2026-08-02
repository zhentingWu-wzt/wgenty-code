import { useState } from "react";
import { CircleAlert, Plus } from "lucide-react";
import type { DaemonClient } from "../api/client";
import { useSessionManager } from "../state/sessionManager";
import { RailSection } from "./RailSection";
import { NewSessionModal } from "./NewSessionModal";

/**
 * Open sessions with live status. "+ New session" opens a dialog with
 * workspace choices (main checkout / existing worktree / new worktree).
 */
export function SessionList({ client }: { client: DaemonClient }) {
  const order = useSessionManager((s) => s.order);
  const entries = useSessionManager((s) => s.entries);
  const activeId = useSessionManager((s) => s.activeId);
  const [showNew, setShowNew] = useState(false);

  return (
    <>
      <RailSection
        title="Sessions"
        actions={
          <button type="button" className="btn-xs" onClick={() => setShowNew(true)}>
            <Plus size={12} /> New session
          </button>
        }
      >
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
                    <span className="session-card-name">
                      {e.name}
                      {e.worktree && (
                        <span className="session-branch-tag">⎇ {e.worktree.branch}</span>
                      )}
                    </span>
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
      </RailSection>
      {showNew && <NewSessionModal client={client} onClose={() => setShowNew(false)} />}
    </>
  );
}
