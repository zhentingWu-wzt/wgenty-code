import { useEffect, useState } from "react";
import type { DaemonClient } from "../../api/client";
import type { WorktreeBinding, WorktreeInfo } from "../../api/types";
import { useSessionManager } from "../../state/sessionManager";
import { CommandModal } from "../panels/CommandModal";
import { Button } from "../../components/ui/button";

type Mode = "main" | "existing" | "new";

/**
 * "New session" dialog: name + workspace choice. Main checkout = current
 * behavior (plain local session). Bound modes create the daemon session first
 * (its id becomes the session's single identity), then bind a worktree —
 * either an existing one or a freshly created branch (spec: N:1 binding).
 */
export function NewSessionModal({
  client,
  onClose,
}: {
  client: DaemonClient;
  onClose: () => void;
}) {
  const [name, setName] = useState("");
  const [mode, setMode] = useState<Mode>("main");
  const [worktrees, setWorktrees] = useState<WorktreeInfo[]>([]);
  const [selectedPath, setSelectedPath] = useState("");
  const [branch, setBranch] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    client
      .listWorktrees()
      .then((ws) => {
        const linked = ws.filter((w) => !w.is_main);
        setWorktrees(linked);
        if (linked.length > 0) setSelectedPath((p) => p || linked[0].path);
      })
      .catch(() => setWorktrees([]));
  }, [client]);

  const create = async () => {
    setBusy(true);
    setError(null);
    try {
      if (mode === "main") {
        useSessionManager.getState().createLocalSession(name.trim() || undefined);
        onClose();
        return;
      }

      let wt: WorktreeBinding;
      if (mode === "existing") {
        const w = worktrees.find((x) => x.path === selectedPath);
        if (!w) {
          setError("Select a worktree");
          return;
        }
        wt = { path: w.path, branch: w.branch ?? "" };
      } else {
        const b = branch.trim();
        if (!b) {
          setError("Branch name required");
          return;
        }
        const path = `.worktrees/${b.replaceAll("/", "-")}`;
        await client.createWorktree({ path, branch: b });
        wt = { path, branch: b };
      }

      // Single identity: the daemon session id doubles as the runtime id.
      const created = await client.createSession({ name: name.trim() || undefined });
      await client.bindWorktree(created.id, wt);
      useSessionManager.getState().createLocalSession(name.trim() || "Session", {
        id: created.id,
        daemonId: created.id,
        worktree: wt,
      });
      onClose();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <CommandModal title="New session" onClose={onClose}>
      <div className="flex flex-col gap-3">
        <label className="flex flex-col gap-1 text-[12px] text-muted-foreground">
          Name (optional)
          <input
            className="rounded-md border border-input bg-background px-2 py-1.5 text-[13px] text-foreground outline-none focus:border-ring"
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="Session name"
          />
        </label>

        <div className="flex flex-col gap-1" role="radiogroup" aria-label="Workspace">
          <label className="flex items-center gap-1.5 text-[13px]">
            <input
              type="radio"
              name="workspace"
              checked={mode === "main"}
              onChange={() => setMode("main")}
            />
            Main checkout
          </label>
          <label className="flex items-center gap-1.5 text-[13px]">
            <input
              type="radio"
              name="workspace"
              checked={mode === "existing"}
              onChange={() => setMode("existing")}
            />
            Existing worktree
          </label>
          <label className="flex items-center gap-1.5 text-[13px]">
            <input
              type="radio"
              name="workspace"
              checked={mode === "new"}
              onChange={() => setMode("new")}
            />
            New worktree
          </label>
        </div>

        {mode === "existing" && (
          <select
            className="rounded-md border border-input bg-background px-2 py-1.5 text-[13px] text-foreground outline-none focus:border-ring"
            value={selectedPath}
            onChange={(e) => setSelectedPath(e.target.value)}
          >
            {worktrees.map((w) => (
              <option key={w.path} value={w.path}>
                {w.branch ?? "(detached)"}
              </option>
            ))}
            {worktrees.length === 0 && <option value="">No linked worktrees</option>}
          </select>
        )}

        {mode === "new" && (
          <input
            className="rounded-md border border-input bg-background px-2 py-1.5 text-[13px] text-foreground outline-none focus:border-ring"
            value={branch}
            onChange={(e) => setBranch(e.target.value)}
            placeholder="Branch name (e.g. feat-x)"
          />
        )}

        {error && <div className="text-[12px] text-danger">{error}</div>}

        <div className="flex justify-end gap-2">
          <Button variant="outline" onClick={onClose} disabled={busy}>
            Cancel
          </Button>
          <Button
            onClick={create}
            disabled={busy || (mode === "existing" && worktrees.length === 0)}
          >
            Create
          </Button>
        </div>
      </div>
    </CommandModal>
  );
}
