import { useCallback, useEffect, useState, type ReactNode } from "react";
import {
  Archive,
  ChevronDown,
  ChevronRight,
  CircleAlert,
  FolderGit2,
  Plus,
  Trash2,
} from "lucide-react";
import { toast } from "sonner";
import type { DaemonClient } from "../api/client";
import type { SessionInfo, WorktreeInfo } from "../api/types";
import { useSessionManager, type SessionEntry } from "../state/sessionManager";
import { NewSessionModal } from "./NewSessionModal";

/**
 * Project tree — the LeftRail's unified hierarchy:
 *   project (repo working dir)
 *   └── task (worktree, main checkout first)
 *       └── session (conversations running in that workspace)
 *
 * Replaces the old split Sessions / Worktrees panels. Worktree remove
 * unbinds bound sessions first (they return to the main checkout).
 */

function TreeNode({
  icon,
  title,
  subtitle,
  count,
  actions,
  defaultCollapsed = false,
  children,
}: {
  icon: ReactNode;
  title: string;
  subtitle?: string;
  count?: number;
  actions?: ReactNode;
  defaultCollapsed?: boolean;
  children: ReactNode;
}) {
  const [collapsed, setCollapsed] = useState(defaultCollapsed);
  return (
    <div className="tree-node">
      <div className="tree-node-head">
        <button
          type="button"
          className="tree-node-toggle"
          aria-expanded={!collapsed}
          onClick={() => setCollapsed((c) => !c)}
        >
          {collapsed ? <ChevronRight size={12} /> : <ChevronDown size={12} />}
          {icon}
          <span className="tree-node-title">{title}</span>
          {subtitle && <span className="tree-node-subtitle">{subtitle}</span>}
          {count !== undefined && <span className="session-group-count">{count}</span>}
        </button>
        {actions && <span className="tree-node-actions">{actions}</span>}
      </div>
      {!collapsed && <div className="tree-children">{children}</div>}
    </div>
  );
}

function SessionCard({
  entry,
  active,
  onArchive,
  onDelete,
}: {
  entry: SessionEntry;
  active: boolean;
  onArchive: (e: SessionEntry) => void;
  onDelete: (e: SessionEntry) => void;
}) {
  return (
    <li className="session-card-row">
      <button
        type="button"
        className={`session-card ${active ? "active" : ""}`}
        onClick={() => useSessionManager.getState().setActive(entry.id)}
      >
        <span className={`session-dot session-status-${entry.status}`} />
        <span className="session-card-main">
          <span className="session-card-name">{entry.name}</span>
          {entry.lastPreview && <span className="session-card-preview">{entry.lastPreview}</span>}
        </span>
        {entry.status === "awaiting_approval" && (
          <span className="session-card-badge">
            <CircleAlert size={11} />
          </span>
        )}
      </button>
      <span className="session-card-actions">
        <button
          type="button"
          className="btn-xs session-action"
          title={`Archive ${entry.name}`}
          onClick={() => onArchive(entry)}
        >
          <Archive size={11} />
        </button>
        <button
          type="button"
          className="btn-xs session-action session-action-danger"
          title={`Delete ${entry.name}`}
          onClick={() => onDelete(entry)}
        >
          <Trash2 size={11} />
        </button>
      </span>
    </li>
  );
}

export function ProjectTree({ client }: { client: DaemonClient }) {
  const order = useSessionManager((s) => s.order);
  const entries = useSessionManager((s) => s.entries);
  const activeId = useSessionManager((s) => s.activeId);

  const [worktrees, setWorktrees] = useState<WorktreeInfo[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [showNewSession, setShowNewSession] = useState(false);

  const refresh = useCallback(() => {
    client
      .listWorktrees()
      .then((w) => {
        setWorktrees(w);
        setError(null);
      })
      .catch((e) => setError(e instanceof Error ? e.message : String(e)));
  }, [client]);

  useEffect(refresh, [refresh]);

  const main = worktrees?.find((w) => w.is_main) ?? null;
  const linked = (worktrees ?? []).filter((w) => !w.is_main);
  const projectName = main ? (main.path.split("/").filter(Boolean).pop() ?? main.path) : "Project";

  const sessionsIn = (wt: WorktreeInfo | null): SessionEntry[] =>
    order
      .map((id) => entries[id])
      .filter((e) => {
        if (!wt) return !e.worktree;
        return e.worktree && (e.worktree.branch === wt.branch || e.worktree.path === wt.path);
      });

  // ── Session actions ────────────────────────────────────────────────────────
  const archiveSession = async (e: SessionEntry) => {
    try {
      if (e.daemonId) await client.setSessionArchived(e.daemonId, true);
      useSessionManager.getState().removeSession(e.id);
    } catch (err) {
      toast.error(`Archive failed: ${err instanceof Error ? err.message : String(err)}`);
    }
  };

  const deleteSession = async (e: SessionEntry) => {
    if (!window.confirm(`Delete session "${e.name}"? This removes its saved history.`)) return;
    try {
      if (e.daemonId) await client.deleteSession(e.daemonId);
      useSessionManager.getState().removeSession(e.id);
    } catch (err) {
      toast.error(`Delete failed: ${err instanceof Error ? err.message : String(err)}`);
    }
  };

  // ── Task (worktree) actions ────────────────────────────────────────────────
  const createTask = async () => {
    const branch = window.prompt("New task (worktree) branch name:");
    if (!branch?.trim()) return;
    const path = `.worktrees/${branch.trim().replaceAll("/", "-")}`;
    try {
      await client.createWorktree({ path, branch: branch.trim() });
      toast.success(`Task ${branch.trim()} created`);
      refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const removeTask = async (w: WorktreeInfo) => {
    try {
      // Reverse-lookup bound sessions; unbind them first (they return to the
      // main checkout).
      const sessions = await client.listSessions().catch(() => [] as SessionInfo[]);
      const bound = sessions.filter(
        (s) => s.worktree && (s.worktree.path === w.path || s.worktree.branch === w.branch),
      );
      const msg =
        bound.length > 0
          ? `"${w.branch ?? w.path}" has ${bound.length} bound session(s); they will be unbound and return to the main checkout. Remove the worktree?`
          : `Remove worktree ${w.path}?`;
      if (!window.confirm(msg)) return;

      for (const s of bound) {
        await client.unbindWorktree(s.id);
        const m = useSessionManager.getState();
        const entry = Object.values(m.entries).find((e) => e.daemonId === s.id);
        if (entry) m.setWorktree(entry.id, null);
      }
      await client.deleteWorktree(w.path);
      toast.success(`Worktree ${w.path} removed`);
      refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const renderSessions = (list: SessionEntry[]) => (
    <ul className="session-cards">
      {list.map((e) => (
        <SessionCard
          key={e.id}
          entry={e}
          active={e.id === activeId}
          onArchive={archiveSession}
          onDelete={deleteSession}
        />
      ))}
    </ul>
  );

  return (
    <div className="project-tree">
      {error && <div className="panel-error">{error}</div>}
      <TreeNode
        icon={<FolderGit2 size={13} />}
        title={projectName}
        subtitle={main?.path}
        actions={
          <>
            <button type="button" className="btn-xs" onClick={() => setShowNewSession(true)}>
              <Plus size={12} /> Session
            </button>
            <button type="button" className="btn-xs" onClick={createTask}>
              <Plus size={12} /> Task
            </button>
          </>
        }
      >
        {/* Main checkout = default workspace (unbound sessions). */}
        <TreeNode
          icon={<span className="tree-branch-icon">⎇</span>}
          title={main?.branch ?? "main"}
          subtitle="main checkout"
          count={sessionsIn(null).length}
        >
          {renderSessions(sessionsIn(null))}
        </TreeNode>

        {/* One task node per linked worktree. */}
        {linked.map((w) => (
          <TreeNode
            key={w.path}
            icon={<span className="tree-branch-icon">⎇</span>}
            title={w.branch ?? "(detached)"}
            count={sessionsIn(w).length}
            actions={
              <button
                type="button"
                className="btn-xs session-action session-action-danger"
                title={`Remove worktree ${w.branch ?? w.path}`}
                onClick={() => removeTask(w)}
              >
                <Trash2 size={11} />
              </button>
            }
          >
            {renderSessions(sessionsIn(w))}
          </TreeNode>
        ))}
      </TreeNode>
      {showNewSession && (
        <NewSessionModal client={client} onClose={() => setShowNewSession(false)} />
      )}
    </div>
  );
}
