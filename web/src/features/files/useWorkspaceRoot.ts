import { useEffect, useState } from "react";
import type { DaemonClient } from "../../api/client";
import { useSessionManager } from "../../state/sessionManager";

/** Last path segment ("/" and "" stay as-is). */
export function basename(path: string): string {
  const parts = path.split("/").filter(Boolean);
  return parts.length > 0 ? (parts[parts.length - 1] as string) : path;
}

export interface WorkspaceRootInfo {
  /** Active session's worktree path, else its project path, else the main
   *  project (local sessions with projectPath null). Null while resolving or
   *  when no project is registered. */
  root: string | null;
  /** listProjects failure text (root stays null). */
  error: string | null;
}

/**
 * Resolve the workspace root the active session operates on — shared by the
 * right rail's Files and Source Control panels so both always agree on
 * "which checkout am I browsing".
 */
export function useWorkspaceRoot(client: DaemonClient): WorkspaceRootInfo {
  const entry = useSessionManager((s) => (s.activeId ? s.entries[s.activeId] : null));

  // Main project path — the fallback for sessions with projectPath null
  // (the daemon groups them under the main project). Fetched once per hook
  // consumer; projects rarely change while a panel is open.
  const [mainPath, setMainPath] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    client
      .listProjects()
      .then((ps) => {
        if (cancelled) return;
        setMainPath(ps.find((p) => p.is_main)?.path ?? null);
        setError(null);
      })
      .catch((e) => {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [client]);

  return { root: entry?.worktree?.path ?? entry?.projectPath ?? mainPath, error };
}
