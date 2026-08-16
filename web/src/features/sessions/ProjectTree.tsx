import { useCallback, useEffect, useState, type ReactNode } from "react";
import {
  Archive,
  ChevronDown,
  ChevronRight,
  CircleAlert,
  FolderGit2,
  FolderMinus,
  FolderTree,
  GitBranchPlus,
  MessageSquarePlus,
  Trash2,
} from "lucide-react";
import { toast } from "sonner";
import type { DaemonClient } from "../../api/client";
import type { ProjectInfo, SessionInfo, WorktreeInfo } from "../../api/types";
import { cn } from "../../lib/utils";
import { useSessionManager, type SessionEntry } from "../../state/sessionManager";
import { FileTree } from "../files/FileTree";
import { NewSessionModal, type NewSessionPreset } from "./NewSessionModal";
import { NewTaskModal } from "./NewTaskModal";

/**
 * Project tree — the LeftSidebar's unified hierarchy:
 *   project (registered repo, main project first)
 *   └── task (worktree, main checkout first)
 *       └── session (conversations running in that workspace)
 *
 * Replaces the old split Sessions / Worktrees panels. Worktree remove
 * unbinds bound sessions first (they return to the main checkout).
 *
 * Multi-project: the daemon keeps a project registry (main project = its
 * working dir, always first). Sessions group under a project by
 * `entry.projectPath` (null = main project). Non-git projects skip all
 * worktree calls — the daemon 400s on them — and hide git-only actions.
 *
 * Action buttons are icon-only and live on the node they create under:
 * "+ task" on the project node, "+ session" on each task node. Node actions
 * reveal on hover/focus to keep the tree uncluttered.
 */

const TREE_ACTION_BTN =
  "inline-flex items-center rounded-sm p-1 text-muted-foreground hover:bg-accent hover:text-foreground";

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
      <div className="group flex items-center gap-1 rounded-sm px-1 py-0.5 hover:bg-accent">
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
        {actions && (
          <span className="hidden items-center gap-0.5 group-hover:flex group-focus-within:flex">
            {actions}
          </span>
        )}
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

export function ProjectTree({
  client,
  refreshKey = 0,
}: {
  client: DaemonClient;
  /** Bumped by the parent (LeftSidebar's "Add project") to force a refetch. */
  refreshKey?: number;
}) {
  const order = useSessionManager((s) => s.order);
  const entries = useSessionManager((s) => s.entries);
  const activeId = useSessionManager((s) => s.activeId);

  const [projects, setProjects] = useState<ProjectInfo[] | null>(null);
  const [worktreesByProject, setWorktreesByProject] = useState<Record<string, WorktreeInfo[]>>({});
  const [error, setError] = useState<string | null>(null);
  const [newSession, setNewSession] = useState<{ preset?: NewSessionPreset } | null>(null);
  const [newTask, setNewTask] = useState<{ project: ProjectInfo } | null>(null);

  const refresh = useCallback(() => {
    client
      .listProjects()
      .then(async (ps) => {
        setProjects(ps);
        setError(null);
        // Worktrees are per-project; non-git projects 400 on the endpoint,
        // so only query git repos.
        const results = await Promise.all(
          ps
            .filter((p) => p.is_git_repo)
            .map(
              async (p) =>
                [p.path, await client.listWorktrees(p.path).catch(() => [])] as const,
            ),
        );
        setWorktreesByProject(Object.fromEntries(results));
      })
      .catch((e) => setError(e instanceof Error ? e.message : String(e)));
  }, [client]);

  useEffect(refresh, [refresh, refreshKey]);

  // Sessions with projectPath null belong to the main project.
  const mainPath = projects?.find((p) => p.is_main)?.path ?? null;

  const sessionsIn = (project: ProjectInfo, wt: WorktreeInfo | null): SessionEntry[] =>
    order
      .map((id) => entries[id])
      .filter((e) => {
        if ((e.projectPath ?? mainPath) !== project.path) return false;
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
  // Opens the New task dialog; the worktree is created inside the modal.
  const createTask = (project: ProjectInfo) => {
    setNewTask({ project });
  };

  const removeTask = async (project: ProjectInfo, w: WorktreeInfo) => {
    try {
      // Reverse-lookup bound sessions; unbind them first (they return to the
      // main checkout). Only this project's sessions can be bound here.
      const sessions = await client.listSessions().catch(() => [] as SessionInfo[]);
      const bound = sessions.filter(
        (s) =>
          (s.project_path ?? mainPath) === project.path &&
          s.worktree &&
          (s.worktree.path === w.path || s.worktree.branch === w.branch),
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
      await client.deleteWorktree(w.path, project.path);
      toast.success(`Worktree ${w.path} removed`);
      refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  // ── Project actions ────────────────────────────────────────────────────────
  const removeProject = async (p: ProjectInfo) => {
    if (
      !window.confirm(`Remove project ${p.path} from the list? Files on disk are not touched.`)
    )
      return;
    try {
      await client.removeProject(p.path);
      // uiSync only prunes tabs, it never rebuilds entries from the daemon —
      // drop this project's local entries here or they would strand invisible.
      const m = useSessionManager.getState();
      for (const e of Object.values(m.entries)) {
        if (e.projectPath === p.path) m.removeSession(e.id);
      }
      toast.success(`Project ${p.name} removed`);
      refresh();
    } catch (e) {
      toast.error(`Remove failed: ${e instanceof Error ? e.message : String(e)}`);
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
      {(projects ?? []).map((p) => {
        const wts = worktreesByProject[p.path] ?? [];
        const main = wts.find((w) => w.is_main) ?? null;
        const linked = wts.filter((w) => !w.is_main);
        return (
          <TreeNode
            key={p.path}
            icon={<FolderGit2 size={13} />}
            title={p.name}
            actions={
              <>
                {p.is_git_repo && (
                  <button
                    type="button"
                    className={TREE_ACTION_BTN}
                    title={`New task in ${p.name}`}
                    onClick={() => createTask(p)}
                  >
                    <GitBranchPlus size={12} />
                  </button>
                )}
                {!p.is_main && (
                  <button
                    type="button"
                    className={cn(TREE_ACTION_BTN, "hover:text-danger")}
                    title={`Remove project ${p.name}`}
                    onClick={() => removeProject(p)}
                  >
                    <FolderMinus size={12} />
                  </button>
                )}
              </>
            }
          >
            {/* Main checkout = default workspace (unbound sessions). Non-git
                projects have no branch, so show the path instead of git terms. */}
            <TreeNode
              icon={<span className="text-[13px] text-primary">⎇</span>}
              title={p.is_git_repo ? (main?.branch ?? "main") : "main"}
              subtitle={p.is_git_repo ? "main checkout" : p.path}
              count={sessionsIn(p, null).length}
              actions={
                <button
                  type="button"
                  className={TREE_ACTION_BTN}
                  title="New session"
                  onClick={() => setNewSession({ preset: { mode: "main", project: p.path } })}
                >
                  <MessageSquarePlus size={12} />
                </button>
              }
            >
              {renderSessions(sessionsIn(p, null))}
              {/* Workspace files (read-only preview). The group node is
                  default-collapsed; expanding mounts FileTree, which fires the
                  first listEntries. Works for non-git projects too (the main
                  checkout task node uses the project path directly). */}
              <TreeNode icon={<FolderTree size={13} />} title="文件" defaultCollapsed>
                <FileTree workspaceRoot={main?.path ?? p.path} client={client} />
              </TreeNode>
            </TreeNode>

            {/* One task node per linked worktree. */}
            {linked.map((w) => (
              <TreeNode
                key={w.path}
                icon={<span className="text-[13px] text-primary">⎇</span>}
                title={w.branch ?? "(detached)"}
                count={sessionsIn(p, w).length}
                actions={
                  <>
                    <button
                      type="button"
                      className={TREE_ACTION_BTN}
                      title={`New session in ${w.branch ?? w.path}`}
                      onClick={() =>
                        setNewSession({
                          preset: {
                            mode: "existing",
                            project: p.path,
                            path: w.path,
                            branch: w.branch ?? "",
                          },
                        })
                      }
                    >
                      <MessageSquarePlus size={12} />
                    </button>
                    <button
                      type="button"
                      className={cn(TREE_ACTION_BTN, "hover:text-danger")}
                      title={`Remove worktree ${w.branch ?? w.path}`}
                      onClick={() => removeTask(p, w)}
                    >
                      <Trash2 size={11} />
                    </button>
                  </>
                }
              >
                {renderSessions(sessionsIn(p, w))}
                {/* Workspace files for this task's worktree (see above). */}
                <TreeNode icon={<FolderTree size={13} />} title="文件" defaultCollapsed>
                  <FileTree workspaceRoot={w.path} client={client} />
                </TreeNode>
              </TreeNode>
            ))}
          </TreeNode>
        );
      })}
      {newSession && (
        <NewSessionModal
          client={client}
          preset={newSession.preset}
          onClose={() => setNewSession(null)}
        />
      )}
      {newTask && (
        <NewTaskModal
          client={client}
          project={newTask.project}
          onClose={() => setNewTask(null)}
          onCreated={refresh}
        />
      )}
    </div>
  );
}
