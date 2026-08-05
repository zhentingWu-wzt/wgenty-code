import { useCallback, useEffect, useState } from "react";
import { Archive, ArchiveRestore, Trash2 } from "lucide-react";
import type { DaemonClient } from "../../api/client";
import type { SessionInfo } from "../../api/types";
import { sessionMessagesToDisplay } from "../../agent/sessionLoad";
import { useSessionManager } from "../../state/sessionManager";
import { useUiStore } from "../../state/uiStore";
import { RailSection } from "../sessions/RailSection";

/**
 * 右栏 Sessions 面板：已保存的 daemon 会话列表，支持打开/归档/删除。
 * Archived sessions hide from the default view and live in the collapsed
 * "Archived (N)" section with an Unarchive action (spec: archive = visibility
 * flag, daemon `SessionStatus::Archived`).
 * 选中会话后加载并激活该会话，同时关闭右栏面板。
 */
export function SessionsPanel({ client }: { client: DaemonClient }) {
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
      useUiStore.getState().setRightPanel(null);
      return;
    }
    const full = await client.loadSession(info.id).catch(() => null);
    if (!full) return; // daemon down → stay put, don't open an empty entry
    const m = useSessionManager.getState();
    const localId = m.createLocalSession(info.name ?? "Session", {
      id: info.id,
      daemonId: info.id,
      ...(info.worktree ? { worktree: info.worktree } : {}),
    });
    const store = useSessionManager.getState().entries[localId].store;
    for (const dm of sessionMessagesToDisplay(full.messages ?? [])) {
      store.getState().pushLoadedMessage(dm);
    }
    useUiStore.getState().setRightPanel(null);
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
    <li key={info.id} className="flex items-center gap-1">
      <button
        type="button"
        className="flex min-w-0 flex-1 rounded-sm px-2 py-1 text-left hover:bg-accent"
        onClick={() => openSession(info)}
      >
        <span className="flex min-w-0 flex-col">
          <span className="truncate text-[13px]">
            {info.name ?? info.id}
            {info.worktree && (
              <span className="ml-1 text-[11px] text-muted-foreground">
                ⎇ {info.worktree.branch}
              </span>
            )}
          </span>
          <span className="truncate text-[11px] text-muted-foreground">
            {info.message_count} messages
          </span>
        </span>
      </button>
      {archivedRow ? (
        <button
          type="button"
          className="shrink-0 rounded-sm p-1 text-muted-foreground hover:bg-accent hover:text-foreground"
          title={`Unarchive ${info.name ?? info.id}`}
          onClick={() => setArchived(info, false)}
        >
          <ArchiveRestore size={12} />
        </button>
      ) : (
        <button
          type="button"
          className="shrink-0 rounded-sm p-1 text-muted-foreground hover:bg-accent hover:text-foreground"
          title={`Archive ${info.name ?? info.id}`}
          onClick={() => setArchived(info, true)}
        >
          <Archive size={12} />
        </button>
      )}
      <button
        type="button"
        className="shrink-0 rounded-sm p-1 text-muted-foreground hover:bg-accent hover:text-foreground"
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
    <div className="p-2">
      {error && <div className="p-2 text-danger">{error}</div>}
      {saved && active.length === 0 && archived.length === 0 && (
        <div className="p-2 text-[12px] text-muted-foreground">No saved sessions</div>
      )}
      <ul className="flex flex-col gap-0.5">{active.map((info) => renderRow(info, false))}</ul>
      {archived.length > 0 && (
        <RailSection title={`Archived (${archived.length})`} defaultCollapsed>
          <ul className="flex flex-col gap-0.5">
            {archived.map((info) => renderRow(info, true))}
          </ul>
        </RailSection>
      )}
    </div>
  );
}
