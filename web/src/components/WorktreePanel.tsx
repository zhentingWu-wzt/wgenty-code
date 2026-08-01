import { useCallback, useEffect, useState } from "react";
import type { DaemonClient } from "../api/client";
import type { WorktreeInfo } from "../api/types";

/** Git worktree list + create/remove. Data: GET/POST/DELETE /api/v1/worktrees. */
export function WorktreePanel({ client }: { client: DaemonClient }) {
  const [items, setItems] = useState<WorktreeInfo[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    client
      .listWorktrees()
      .then((w) => {
        setItems(w);
        setError(null);
      })
      .catch((e) => setError(e instanceof Error ? e.message : String(e)));
  }, [client]);

  useEffect(refresh, [refresh]);

  const create = async () => {
    const branch = window.prompt("New branch name:");
    if (!branch?.trim()) return;
    const path = `.worktrees/${branch.trim().replaceAll("/", "-")}`;
    try {
      await client.createWorktree({ path, branch: branch.trim() });
      refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const remove = async (path: string) => {
    if (!window.confirm(`Remove worktree ${path}?`)) return;
    try {
      await client.deleteWorktree(path);
      refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <section className="rail-panel">
      <div className="session-list-head">
        <span className="rail-section-title">Worktrees</span>
        <button type="button" className="btn-xs" onClick={create}>
          + New
        </button>
      </div>
      {error && <div className="panel-error">{error}</div>}
      {items && items.length === 0 && <div className="panel-empty">No worktrees</div>}
      <ul className="wt-list">
        {(items ?? []).map((w) => (
          <li key={w.path} className="wt-item">
            <span className="wt-branch">{w.branch ?? "(detached)"}</span>
            {w.is_main && <span className="wt-main-tag">main</span>}
            {!w.is_main && (
              <button
                type="button"
                className="btn-xs wt-remove"
                onClick={() => remove(w.path)}
              >
                Remove
              </button>
            )}
          </li>
        ))}
      </ul>
    </section>
  );
}
