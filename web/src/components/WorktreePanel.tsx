import { useCallback, useEffect, useState } from "react";
import { Plus } from "lucide-react";
import { toast } from "sonner";
import type { DaemonClient } from "../api/client";
import type { SessionInfo, WorktreeInfo } from "../api/types";
import { useSessionManager } from "../state/sessionManager";
import { RailSection } from "./RailSection";

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
      toast.success(`Worktree ${branch.trim()} created`);
      refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const remove = async (w: WorktreeInfo) => {
    try {
      // Reverse-lookup sessions bound to this worktree (N:1). They must be
      // unbound first — they return to the main checkout.
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
        // Mirror the unbind into any locally open entry.
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

  return (
    <RailSection
      title="Worktrees"
      actions={
        <button type="button" className="btn-xs" onClick={create}>
          <Plus size={12} /> New
        </button>
      }
    >
      {error && <div className="panel-error">{error}</div>}
      {items && items.length === 0 && <div className="panel-empty">No worktrees</div>}
      <ul className="wt-list">
        {(items ?? []).map((w) => (
          <li key={w.path} className="wt-item">
            <span className="wt-branch">{w.branch ?? "(detached)"}</span>
            {w.is_main && <span className="wt-main-tag">main</span>}
            {!w.is_main && (
              <button type="button" className="btn-xs wt-remove" onClick={() => remove(w)}>
                Remove
              </button>
            )}
          </li>
        ))}
      </ul>
    </RailSection>
  );
}
