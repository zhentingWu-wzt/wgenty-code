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
import type { DaemonClient } from "../../api/client";
import type { SessionInfo, WorktreeInfo } from "../../api/types";
import { cn } from "../../lib/utils";
import { useSessionManager, type SessionEntry } from "../../state/sessionManager";
import { NewSessionModal } from "./NewSessionModal";

/**
 * Project tree — the LeftSidebar's unified hierarchy:
 *   project (repo working dir)
 *   └── task (worktree, main checkout first)
 *       └── session (conversations running in that workspace)
 *
 * Replaces the old split Sessions / Worktrees panels. Worktree remove
 * unbinds bound sessions first (they return to the main checkout).
 */

const TREE_ACTION_BTN =
  "inline-flex items-center gap-0.5 rounded-sm px-1.5 py-0.5 text-[11px] text-muted-foreground hover:bg-accent hover:text-foreground";

const STATUS_DOT: Record<string, string> = {
  running: "text-success",
  awaiting_approval: "text-warning",
  error: "text-danger",
};

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
    <div>
      <div className="flex items-center gap-1 rounded-sm px-1 py-0.5 hover:bg-accent">
        <button
          type="button"
          aria-expanded={!collapsed}
          onClick={() => setCollapsed((c) => !c)}
          className="flex min-w-0 flex-1 items-center gap-1 text-left text-[13px]"
        >
          {collapsed ? <ChevronRight size={12} /> : <ChevronDown size={12} />}
          {icon}
          <span className="truncate font-medium">{title}</span>
          {subtitle && <span className="truncate text-muted-foreground">{subtitle}</span>}
          {count !== undefined && (
            <span className="ml-auto text-[11px] text-muted-foreground">{count}</span>
          )}
        </button>
        {actions && <span className="flex items-center gap-0.5">{actions}</span>}
      </div>
      {!collapsed && <div className="ml-4 flex flex-col gap-0.5">{children}</div>}
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
    <li className="group flex items-center gap-1">
      <button
        type="button"
        className={cn(
          "flex min-w-0 flex-1 items-center gap-1.5 rounded-md border px-2 py-1 text-left text-[13px]",
          active ? "border-primary/50 bg-accent" : "border-border bg-card hover:bg-accent",
        )}
        onClick={() => useSessionManager.getState().setActive(entry.id)}
      >
        <span
          className={cn(
            "h-[7px] w-[7px] shrink-0 rounded-full bg-current",
            STATUS_DOT[entry.status] ?? "text-muted-foreground",
          )}
        />
        <span className="flex min-w-0 flex-1 flex-col">
          <span className="truncate font-medium">{entry.name}</span>
          {entry.lastPreview && (
            <span className="truncate text-[11px] text-muted-foreground">{entry.lastPreview}</span>
          )}
        </span>
        {entry.status === "awaiting_approval" && (
          <span className="flex h-3.5 w-3.5 shrink-0 items-center justify-center rounded-full bg-warning text-background">
            <CircleAlert size={11} />
          </span>
        )}
      </button>
      <span className="hidden shrink-0 items-center gap-0.5 group-hover:flex">
        <button
          type="button"
          className={TREE_ACTION_BTN}
          title={`Archive ${entry.name}`}
          onClick={() => onArchive(entry)}
        >
          <Archive size={11} />
        </button>
        <button
          type="button"
          className={cn(TREE_ACTION_BTN, "hover:text-danger")}
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
    <ul className="m-0 flex list-none flex-col gap-1 p-0">
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
    <div className="flex flex-col gap-1">
      {error && (
        <div className="mb-1 rounded-sm border border-danger/40 bg-danger/10 px-2 py-1 text-xs text-danger">
          {error}
        </div>
      )}
      <TreeNode
        icon={<FolderGit2 size={13} />}
        title={projectName}
        subtitle={main?.path}
        actions={
          <>
            <button
              type="button"
              className={TREE_ACTION_BTN}
              onClick={() => setShowNewSession(true)}
            >
              <Plus size={12} /> Session
            </button>
            <button type="button" className={TREE_ACTION_BTN} onClick={createTask}>
              <Plus size={12} /> Task
            </button>
          </>
        }
      >
        {/* Main checkout = default workspace (unbound sessions). */}
        <TreeNode
          icon={<span className="text-[13px] text-primary">⎇</span>}
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
            icon={<span className="text-[13px] text-primary">⎇</span>}
            title={w.branch ?? "(detached)"}
            count={sessionsIn(w).length}
            actions={
              <button
                type="button"
                className={cn(TREE_ACTION_BTN, "hover:text-danger")}
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
