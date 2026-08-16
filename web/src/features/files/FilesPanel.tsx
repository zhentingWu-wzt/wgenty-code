import { useState } from "react";
import { FolderGit2, RefreshCw } from "lucide-react";
import type { DaemonClient } from "../../api/client";
import { cn } from "../../lib/utils";
import { useSessionManager } from "../../state/sessionManager";
import { basename, useWorkspaceRoot } from "./useWorkspaceRoot";
import { FileTree } from "./FileTree";

/**
 * Files panel (right rail) — the file tree's first-class home, replacing the
 * old nested「文件」group inside ProjectTree. Follows the active session's
 * workspace: bound worktree path first, else the session's project path,
 * else the main project (local sessions with projectPath null).
 */

export function FilesPanel({ client }: { client: DaemonClient }) {
  const entry = useSessionManager((s) => (s.activeId ? s.entries[s.activeId] : null));
  const { root, error: loadError } = useWorkspaceRoot(client);
  /** Bumped by the refresh button: remounts the tree (refetches listings and
   *  git status) after the agent has changed files on disk. */
  const [refreshKey, setRefreshKey] = useState(0);

  if (loadError) {
    return <div className="p-3 text-xs text-danger">{loadError}</div>;
  }

  if (!root) {
    return (
      <div className="flex flex-col items-start gap-2 p-3 text-xs text-muted-foreground">
        <FolderGit2 size={14} />
        <span>No workspace yet — add a project in the left sidebar.</span>
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col" data-testid="files-panel">
      {/* Workspace context row: which checkout/worktree the tree browses. */}
      <div
        className="flex shrink-0 items-center gap-1.5 border-b border-border px-2 py-1 text-[11px] text-muted-foreground"
        title={root}
      >
        <FolderGit2 size={12} className="shrink-0" />
        <span className="truncate">{basename(root)}</span>
        {entry?.worktree?.branch && (
          <span className="ml-auto shrink-0 text-primary">⎇ {entry.worktree.branch}</span>
        )}
        <button
          type="button"
          title="Refresh (listings + git status)"
          className={cn(
            "shrink-0 rounded-sm p-0.5 text-muted-foreground hover:bg-accent hover:text-foreground",
            entry?.worktree?.branch && "ml-1",
          )}
          onClick={() => setRefreshKey((k) => k + 1)}
        >
          <RefreshCw size={11} />
        </button>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto p-1">
        {/* key=root: switching workspaces remounts the tree so stale expanded
            dirs from the previous root never leak into the new one. */}
        <FileTree key={`${root}:${refreshKey}`} workspaceRoot={root} client={client} />
      </div>
    </div>
  );
}
