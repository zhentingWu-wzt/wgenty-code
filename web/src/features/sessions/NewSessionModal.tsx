import { useEffect, useState } from "react";
import type { DaemonClient } from "../../api/client";
import type { WorktreeBinding, WorktreeInfo } from "../../api/types";
import { useSessionManager } from "../../state/sessionManager";
import { CommandModal } from "../panels/CommandModal";
import { Button } from "../../components/ui/button";

type Mode = "main" | "existing" | "new";

/** Preset workspace choice, used when the dialog is opened from a specific
 *  tree node (e.g. a worktree's "+ session" button). `project` is the owning
 *  project's canonical path — all daemon calls are scoped to it. */
export type NewSessionPreset =
  | { mode: "main"; project: string }
  | { mode: "existing"; project: string; path: string; branch: string };

/**
 * "New session" dialog: name + workspace choice. Main checkout without a
 * project preset = current behavior (plain local session); with a preset the
 * session is created daemon-side in that project (`project_path`). Bound
 * modes create the daemon session first (its id becomes the session's single
 * identity), then bind a worktree — either an existing one or a freshly
 * created branch (spec: N:1 binding).
 */
export function NewSessionModal({
  client,
  onClose,
  preset,
}: {
  client: DaemonClient;
  onClose: () => void;
  preset?: NewSessionPreset;
}) {
  const [name, setName] = useState("");
  const [mode, setMode] = useState<Mode>(preset?.mode ?? "main");
  const project = preset?.project;
  const [worktrees, setWorktrees] = useState<WorktreeInfo[]>([]);
  const [selectedPath, setSelectedPath] = useState(
    preset?.mode === "existing" ? preset.path : "",
  );
  const [branch, setBranch] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    // Scoped to the preset's project; non-git projects 400 here and simply
    // end up with an empty dropdown.
    client
      .listWorktrees(project)
      .then((ws) => {
        const linked = ws.filter((w) => !w.is_main);
        setWorktrees(linked);
        if (linked.length > 0) setSelectedPath((p) => p || linked[0].path);
      })
      .catch(() => setWorktrees([]));
  }, [client, project]);

  const create = async () => {
    setBusy(true);
    setError(null);
    try {
      if (mode === "main") {
        if (!project) {
          useSessionManager.getState().createLocalSession(name.trim() || undefined);
        } else {
          // Project-scoped main session lives daemon-side so it aggregates
          // under that project in GET /sessions.
          const created = await client.createSession({
            name: name.trim() || undefined,
            project_path: project,
          });
          useSessionManager.getState().createLocalSession(name.trim() || undefined, {
            id: created.id,
            daemonId: created.id,
            projectPath: project,
          });
        }
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
        await client.createWorktree({ path, branch: b, project });
        wt = { path, branch: b };
      }

      // Single identity: the daemon session id doubles as the runtime id.
      const created = await client.createSession({
        // Unnamed → daemon auto-titles from the first message (no UUID).
        name: name.trim() || undefined,
        ...(project ? { project_path: project } : {}),
      });
      await client.bindWorktree(created.id, wt);
      // Leave the local entry unnamed so the placeholder shows until the
      // daemon auto-title is mirrored back after the first turn.
      useSessionManager.getState().createLocalSession(name.trim() || undefined, {
        id: created.id,
        daemonId: created.id,
        worktree: wt,
        projectPath: project ?? null,
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
