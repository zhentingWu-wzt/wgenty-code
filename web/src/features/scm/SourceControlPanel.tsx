import { useCallback, useEffect, useState } from "react";
import { FileDiff, Loader2, RefreshCw } from "lucide-react";
import type { DaemonClient } from "../../api/client";
import type { GitChangeKind, GitFileStatus } from "../../api/types";
import { cn } from "../../lib/utils";
import { useUiStore } from "../../state/uiStore";
import { basename, useWorkspaceRoot } from "../files/useWorkspaceRoot";

/** Badge letter + color per change kind — mirrors the file tree coloring. */
const STATUS_META: Record<GitChangeKind, { letter: string; color: string; label: string }> = {
  modified: { letter: "M", color: "text-warning", label: "修改" },
  added: { letter: "A", color: "text-success", label: "新增" },
  deleted: { letter: "D", color: "text-danger", label: "删除" },
};

/**
 * Source Control panel (right rail) — the active workspace's changed files.
 * Each row shows the git status badge; clicking opens a `diff:<path>` center
 * tab (full file content with inline +/- coloring) via openDiffTab.
 */
export function SourceControlPanel({ client }: { client: DaemonClient }) {
  const { root, error } = useWorkspaceRoot(client);
  const [files, setFiles] = useState<GitFileStatus[] | null>(null);
  const [attempt, setAttempt] = useState(0);

  const openDiffTab = useUiStore((s) => s.openDiffTab);

  const load = useCallback(async () => {
    if (!root) return;
    try {
      setFiles(await client.gitStatus(root));
    } catch {
      setFiles([]);
    }
  }, [client, root]);

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect -- fetch-on-mount
    void load();
  }, [load, attempt]);

  if (error) {
    return <div className="p-3 text-xs text-danger">{error}</div>;
  }

  if (!root) {
    return (
      <div className="p-3 text-xs text-muted-foreground">
        No workspace yet — add a project in the left sidebar.
      </div>
    );
  }

  const open = (f: GitFileStatus) => {
    const absPath = root.endsWith("/") ? `${root}${f.path}` : `${root}/${f.path}`;
    openDiffTab({ workspaceRoot: root, absPath, relPath: f.path, status: f.status });
  };

  return (
    <div className="flex h-full flex-col" data-testid="scm-panel">
      {/* Workspace context row: which checkout the changes belong to. */}
      <div
        className="flex shrink-0 items-center gap-1.5 border-b border-border px-2 py-1 text-[11px] text-muted-foreground"
        title={root}
      >
        <span className="truncate">{basename(root)}</span>
        {files && files.length > 0 && (
          <span className="shrink-0 text-foreground">{files.length} 项变更</span>
        )}
        <button
          type="button"
          title="Refresh"
          className="ml-auto shrink-0 rounded-sm p-0.5 text-muted-foreground hover:bg-accent hover:text-foreground"
          onClick={() => setAttempt((k) => k + 1)}
        >
          <RefreshCw size={11} />
        </button>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto p-1">
        {files === null ? (
          <div className="flex items-center gap-1 px-1 py-0.5 text-[13px] text-muted-foreground">
            <Loader2 size={12} className="animate-spin" />
            <span>加载中…</span>
          </div>
        ) : files.length === 0 ? (
          <div className="flex items-center gap-1.5 px-2 py-2 text-[13px] text-muted-foreground">
            <FileDiff size={13} />
            <span>没有变更</span>
          </div>
        ) : (
          files.map((f) => {
            const meta = STATUS_META[f.status];
            const name = f.path.slice(f.path.lastIndexOf("/") + 1);
            const dir = f.path.slice(0, f.path.lastIndexOf("/"));
            return (
              <button
                key={f.path}
                type="button"
                title={f.path}
                onClick={() => open(f)}
                className="flex w-full min-w-0 items-center gap-1.5 rounded-sm px-1 py-0.5 text-left text-[13px] hover:bg-accent"
              >
                <span
                  className={cn(
                    "w-3.5 shrink-0 text-center font-mono text-[11px] font-semibold",
                    meta.color,
                  )}
                >
                  {meta.letter}
                </span>
                <span className="truncate">{name}</span>
                {dir && <span className="truncate text-[11px] text-muted-foreground">{dir}</span>}
              </button>
            );
          })
        )}
      </div>
    </div>
  );
}
