import { useCallback, useEffect, useState } from "react";
import { Archive, ArchiveRestore, Trash2 } from "lucide-react";
import type { DaemonClient } from "../api/client";
import type { SessionInfo } from "../api/types";
import { sessionMessagesToDisplay } from "../agent/sessionLoad";
import { useSessionManager } from "../state/sessionManager";
import { CommandModal } from "../features/panels/CommandModal";
import { RailSection } from "../features/sessions/RailSection";

/**
 * `/sessions` browser: saved daemon sessions with open/archive/delete.
 * Archived sessions hide from the default view and live in the collapsed
 * "Archived (N)" section with an Unarchive action (spec: archive = visibility
 * flag, daemon `SessionStatus::Archived`).
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
    const localId = m.createLocalSession(info.name ?? "Session", {
      id: info.id,
      daemonId: info.id,
      projectPath: info.project_path ?? null,
      ...(info.worktree ? { worktree: info.worktree } : {}),
    });
    const store = useSessionManager.getState().entries[localId].store;
    for (const dm of sessionMessagesToDisplay(full.messages ?? [])) {
      store.getState().pushLoadedMessage(dm);
    }
    onClose();
  };

  const setArchived = async (info: SessionInfo, archived: boolean) => {
    try {
      await client.setSessionArchived(info.id, archived);
      refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
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

  const renderRow = (info: SessionInfo, archivedRow: boolean) => (
    <li key={info.id} className="saved-session-row">
      <button type="button" className="session-card" onClick={() => openSession(info)}>
        <span className="session-card-main">
          <span className="session-card-name">
            {info.name ?? info.id}
            {info.worktree && <span className="session-branch-tag">⎇ {info.worktree.branch}</span>}
          </span>
          <span className="session-card-preview">{info.message_count} messages</span>
        </span>
      </button>
      {archivedRow ? (
        <button
          type="button"
          className="btn-xs saved-session-delete"
          title={`Unarchive ${info.name ?? info.id}`}
          onClick={() => setArchived(info, false)}
        >
          <ArchiveRestore size={12} />
        </button>
      ) : (
        <button
          type="button"
          className="btn-xs saved-session-delete"
          title={`Archive ${info.name ?? info.id}`}
          onClick={() => setArchived(info, true)}
        >
          <Archive size={12} />
        </button>
      )}
      <button
        type="button"
        className="btn-xs saved-session-delete"
        title={`Delete ${info.name ?? info.id}`}
        onClick={() => deleteSession(info)}
      >
        <Trash2 size={12} />
      </button>
    </li>
  );

  const active = (saved ?? []).filter((s) => s.status !== "Archived");
  const archived = (saved ?? []).filter((s) => s.status === "Archived");

  return (
    <CommandModal title="Saved sessions" onClose={onClose}>
      {error && <div className="panel-error">{error}</div>}
      {saved && active.length === 0 && archived.length === 0 && (
        <div className="panel-empty">No saved sessions</div>
      )}
      <ul className="session-cards">{active.map((info) => renderRow(info, false))}</ul>
      {archived.length > 0 && (
        <RailSection title={`Archived (${archived.length})`} defaultCollapsed>
          <ul className="session-cards">{archived.map((info) => renderRow(info, true))}</ul>
        </RailSection>
      )}
    </CommandModal>
  );
}
