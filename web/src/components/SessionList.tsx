import { useState } from "react";
import { Archive, CircleAlert, Plus, Trash2 } from "lucide-react";
import { toast } from "sonner";
import type { DaemonClient } from "../api/client";
import { useSessionManager, type SessionEntry } from "../state/sessionManager";
import { RailSection } from "./RailSection";
import { NewSessionModal } from "./NewSessionModal";

/** Group entries: unbound under "Main checkout", bound under their branch. */
function groupByWorkspace(entries: Record<string, SessionEntry>, order: string[]) {
  const groups: Array<{ title: string; ids: string[] }> = [];
  const main = order.filter((id) => !entries[id].worktree);
  if (main.length > 0 || order.length === 0) groups.push({ title: "Main checkout", ids: main });
  const branches = [
    ...new Set(order.map((id) => entries[id].worktree?.branch).filter(Boolean)),
  ] as string[];
  for (const b of branches) {
    groups.push({
      title: `⎇ ${b}`,
      ids: order.filter((id) => entries[id].worktree?.branch === b),
    });
  }
  return groups;
}

/**
 * Open sessions grouped by workspace, with live status and per-card
 * archive/delete actions. "+ New session" opens the workspace-choice dialog.
 */
export function SessionList({ client }: { client: DaemonClient }) {
  const order = useSessionManager((s) => s.order);
  const entries = useSessionManager((s) => s.entries);
  const activeId = useSessionManager((s) => s.activeId);
  const [showNew, setShowNew] = useState(false);

  const archive = async (e: SessionEntry) => {
    try {
      if (e.daemonId) await client.setSessionArchived(e.daemonId, true);
      useSessionManager.getState().removeSession(e.id);
    } catch (err) {
      toast.error(`Archive failed: ${err instanceof Error ? err.message : String(err)}`);
    }
  };

  const remove = async (e: SessionEntry) => {
    if (!window.confirm(`Delete session "${e.name}"? This removes its saved history.`)) return;
    try {
      if (e.daemonId) await client.deleteSession(e.daemonId);
      useSessionManager.getState().removeSession(e.id);
    } catch (err) {
      toast.error(`Delete failed: ${err instanceof Error ? err.message : String(err)}`);
    }
  };

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
        {groupByWorkspace(entries, order).map((g) => (
          <div key={g.title} className="session-group">
            <div className="session-group-title">
              {g.title}
              <span className="session-group-count">{g.ids.length}</span>
            </div>
            <ul className="session-cards">
              {g.ids.map((id) => {
                const e = entries[id];
                return (
                  <li key={id} className="session-card-row">
                    <button
                      type="button"
                      className={`session-card ${id === activeId ? "active" : ""}`}
                      onClick={() => useSessionManager.getState().setActive(id)}
                    >
                      <span className={`session-dot session-status-${e.status}`} />
                      <span className="session-card-main">
                        <span className="session-card-name">{e.name}</span>
                        {e.lastPreview && (
                          <span className="session-card-preview">{e.lastPreview}</span>
                        )}
                      </span>
                      {e.status === "awaiting_approval" && (
                        <span className="session-card-badge">
                          <CircleAlert size={11} />
                        </span>
                      )}
                    </button>
                    <span className="session-card-actions">
                      <button
                        type="button"
                        className="btn-xs session-action"
                        title={`Archive ${e.name}`}
                        onClick={() => archive(e)}
                      >
                        <Archive size={11} />
                      </button>
                      <button
                        type="button"
                        className="btn-xs session-action session-action-danger"
                        title={`Delete ${e.name}`}
                        onClick={() => remove(e)}
                      >
                        <Trash2 size={11} />
                      </button>
                    </span>
                  </li>
                );
              })}
            </ul>
          </div>
        ))}
      </RailSection>
      {showNew && <NewSessionModal client={client} onClose={() => setShowNew(false)} />}
    </>
  );
}
