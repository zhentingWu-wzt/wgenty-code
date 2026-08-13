import { useCallback, useEffect, useState } from "react";
import { Archive, ArchiveRestore, Search, Trash2 } from "lucide-react";
import type { DaemonClient } from "../../api/client";
import type { SessionInfo } from "../../api/types";
import { sessionMessagesToDisplay } from "../../agent/sessionLoad";
import { useSessionManager } from "../../state/sessionManager";
import { useUiStore } from "../../state/uiStore";
import { RailSection } from "../sessions/RailSection";

/**
 * 右栏 Sessions 面板：已保存的 daemon 会话列表，支持搜索/打开/归档/删除。
 * Archived sessions hide from the default view and live in the collapsed
 * "Archived (N)" section with an Unarchive action (spec: archive = visibility
 * flag, daemon `SessionStatus::Archived`).
 * 选中会话后加载并激活该会话，同时关闭右栏面板。
 *
 * 搜索：输入关键词时调用 daemon 的 /sessions/search（匹配会话名和消息内容），
 * 清空时回退到全量列表。搜索结果不区分 active/archived 分组。
 */
export function SessionsPanel({ client }: { client: DaemonClient }) {
  const [saved, setSaved] = useState<SessionInfo[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [searchQuery, setSearchQuery] = useState("");

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

  // Debounced search: when query changes, switch to search API; when empty,
  // fall back to the full list.
  useEffect(() => {
    const q = searchQuery.trim();
    if (!q) {
      refresh();
      return;
    }
    const timer = setTimeout(() => {
      client
        .searchSessions(q)
        .then((s) => {
          setSaved(s);
          setError(null);
        })
        .catch((e) => setError(e instanceof Error ? e.message : String(e)));
    }, 300); // 300ms debounce — avoids hammering the daemon on every keystroke
    return () => clearTimeout(timer);
  }, [searchQuery, client, refresh]);

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
      projectPath: info.project_path ?? null,
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

  const isSearching = searchQuery.trim().length > 0;
  const active = (saved ?? []).filter((s) => s.status !== "Archived");
  const archived = (saved ?? []).filter((s) => s.status === "Archived");

  return (
    <div className="p-2">
      {/* Search input — switches to /sessions/search when non-empty */}
      <div className="relative mb-2">
        <Search
          size={12}
          className="pointer-events-none absolute left-2 top-1/2 -translate-y-1/2 text-muted-foreground"
        />
        <input
          type="text"
          value={searchQuery}
          onChange={(e) => setSearchQuery(e.target.value)}
          placeholder="Search sessions..."
          className="w-full rounded-sm border border-border bg-background py-1 pl-7 pr-2 text-[12px] outline-none focus:border-primary"
        />
      </div>
      {error && <div className="p-2 text-danger">{error}</div>}
      {saved && active.length === 0 && archived.length === 0 && (
        <div className="p-2 text-[12px] text-muted-foreground">
          {isSearching ? "No matching sessions" : "No saved sessions"}
        </div>
      )}
      {isSearching ? (
        // Search results: flat list, no active/archived split
        <ul className="flex flex-col gap-0.5">
          {(saved ?? []).map((info) => renderRow(info, info.status === "Archived"))}
        </ul>
      ) : (
        <>
          <ul className="flex flex-col gap-0.5">{active.map((info) => renderRow(info, false))}</ul>
          {archived.length > 0 && (
            <RailSection title={`Archived (${archived.length})`} defaultCollapsed>
              <ul className="flex flex-col gap-0.5">
                {archived.map((info) => renderRow(info, true))}
              </ul>
            </RailSection>
          )}
        </>
      )}
    </div>
  );
}
