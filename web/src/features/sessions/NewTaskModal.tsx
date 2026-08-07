import { useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import type { DaemonClient } from "../../api/client";
import type { ProjectInfo } from "../../api/types";
import { Button } from "../../components/ui/button";
import { CommandModal } from "../panels/CommandModal";

/**
 * "New task" dialog: prompts for a worktree branch name and creates the
 * worktree under `.worktrees/<branch>`. Replaces the old native window.prompt.
 * Visual language mirrors NewSessionModal (same CommandModal shell + input).
 */
export function NewTaskModal({
  client,
  project,
  onClose,
  onCreated,
}: {
  client: DaemonClient;
  project: ProjectInfo;
  onClose: () => void;
  /** Called after a successful create so the caller can refresh its tree. */
  onCreated: () => void;
}) {
  const [branch, setBranch] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  const create = async () => {
    const b = branch.trim();
    if (!b) {
      setError("Branch name required");
      return;
    }
    const path = `.worktrees/${b.replaceAll("/", "-")}`;
    setBusy(true);
    setError(null);
    try {
      await client.createWorktree({ path, branch: b, project: project.path });
      toast.success(`Task ${b} created`);
      onCreated();
      onClose();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <CommandModal title="New task" onClose={onClose}>
      <div className="flex flex-col gap-3">
        <label className="flex flex-col gap-1 text-[12px] text-muted-foreground">
          Branch name
          <input
            ref={inputRef}
            className="rounded-md border border-input bg-background px-2 py-1.5 text-[13px] text-foreground outline-none focus:border-ring"
            value={branch}
            onChange={(e) => setBranch(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") void create();
            }}
            placeholder="e.g. feature/login"
          />
        </label>
        {error && (
          <div className="rounded-sm border border-danger/40 bg-danger/10 px-2 py-1 text-xs text-danger">
            {error}
          </div>
        )}
        <div className="flex justify-end gap-2">
          <Button variant="ghost" size="sm" onClick={onClose} disabled={busy}>
            Cancel
          </Button>
          <Button size="sm" onClick={() => void create()} disabled={busy}>
            Create
          </Button>
        </div>
      </div>
    </CommandModal>
  );
}
