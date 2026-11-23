import { useCallback, useEffect, useState } from "react";
import type { DaemonClient } from "../../api/client";
import type {
  CheckpointDiff,
  CheckpointDiffEntry,
  CheckpointInfo,
  UndoTurnResult,
} from "../../api/types";

/** Per-turn file checkpoints with diff-preview undo.
 *  Global store — not per-session (daemon limitation, same as Todos). */
export function CheckpointsPanel({ client }: { client: DaemonClient }) {
  const [items, setItems] = useState<CheckpointInfo[]>([]);
  const [result, setResult] = useState<UndoTurnResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  /** turn_id -> diff preview being confirmed. */
  const [preview, setPreview] = useState<{ turnId: string; diff: CheckpointDiff } | null>(null);
  const [previewLoading, setPreviewLoading] = useState<string | null>(null);

  const refresh = useCallback(() => {
    client
      .listCheckpoints()
      .then(setItems)
      .catch((e) => setError(e instanceof Error ? e.message : String(e)));
  }, [client]);

  useEffect(refresh, [refresh]);

  const requestUndo = async (turnId: string) => {
    setError(null);
    setPreviewLoading(turnId);
    try {
      const diff = await client.checkpointDiff(turnId);
      setPreview({ turnId, diff });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setPreviewLoading(null);
    }
  };

  const confirmUndo = async () => {
    if (!preview) return;
    try {
      setResult(await client.undoTurns([preview.turnId]));
      setPreview(null);
      refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setPreview(null);
    }
  };

  return (
    <section className="flex flex-col gap-2 p-2">
      {error && <div className="text-danger">{error}</div>}
      {result && (
        <div className="rounded-sm bg-card px-2 py-1 text-[12px] text-muted-foreground">
          restored {result.restored}, skipped {result.skipped}, failed {result.failed}
        </div>
      )}
      {items.length === 0 && (
        <div className="text-[12px] text-muted-foreground">No checkpoints</div>
      )}
      <ul className="flex flex-col gap-0.5">
        {items.map((c) => (
          <li key={c.turn_id} className="flex flex-col gap-0.5 rounded-sm px-2 py-1">
            <div className="flex items-center gap-2">
              <span className="text-[12px] text-muted-foreground">
                {formatTime(c.created_at)}
              </span>
              <span className="truncate text-[13px]" title={c.user_summary ?? undefined}>
                {c.user_summary ?? shortPaths(c.files)}
              </span>
              <span className="text-[11px] text-muted-foreground">{c.file_count} files</span>
              <button
                type="button"
                className="ml-auto rounded-sm border border-border px-2 py-0.5 text-[11px] hover:bg-accent"
                onClick={() => requestUndo(c.turn_id)}
                disabled={previewLoading === c.turn_id}
              >
                {previewLoading === c.turn_id ? "loading…" : "Undo"}
              </button>
            </div>
            <span className="truncate text-[11px] text-muted-foreground">
              {shortPaths(c.files)}
            </span>
          </li>
        ))}
      </ul>
      {preview && (
        <DiffConfirmModal
          preview={preview}
          client={client}
          onCancel={() => setPreview(null)}
          onConfirm={confirmUndo}
        />
      )}
    </section>
  );
}

/** Confirmation modal: shows per-file projected changes + unified diffs,
 *  undo runs only after the user confirms. */
function DiffConfirmModal({
  preview,
  client,
  onCancel,
  onConfirm,
}: {
  preview: { turnId: string; diff: CheckpointDiff };
  client: DaemonClient;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const { diff } = preview;
  const deletions = diff.files.filter((f) => f.will_delete);
  const modified = diff.files.filter((f) => f.state === "saved");
  const skipped = diff.files.filter((f) => f.skipped);
  const totalAdded = diff.files.reduce((n, f) => n + f.added, 0);
  const totalRemoved = diff.files.reduce((n, f) => n + f.removed, 0);
  const [expanded, setExpanded] = useState(true);
  const [busy, setBusy] = useState(false);
  const summary = useCallback(
    () =>
      client
        .listCheckpoints()
        .catch(() => undefined),
    [client],
  );
  void summary;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4">
      <div className="flex max-h-[85vh] w-full max-w-3xl flex-col gap-2 overflow-hidden rounded-md border border-border bg-background p-3">
        <div className="flex items-center gap-2">
          <h3 className="text-[13px] font-semibold">Undo preview</h3>
          <span className="text-[12px] text-muted-foreground">
            {preview.diff.turn_id.slice(0, 8)}
          </span>
          <span className="ml-auto text-[12px]">
            <span className="text-success">+{totalAdded}</span>{" "}
            <span className="text-danger">−{totalRemoved}</span>
          </span>
        </div>

        <div className="flex flex-col gap-1 overflow-y-auto text-[12px]">
          {modified.map((f) => (
            <DiffFile key={f.path} entry={f} expanded={expanded} />
          ))}
          {deletions.length > 0 && (
            <div className="rounded-sm bg-danger/10 px-2 py-1">
              <div className="font-semibold text-danger">Will delete (created during turn):</div>
              <ul className="ml-3 list-disc">
                {deletions.map((f) => (
                  <li key={f.path}>{f.path}</li>
                ))}
              </ul>
            </div>
          )}
          {skipped.length > 0 && (
            <div className="text-muted-foreground">
              Not restored (binary/large at capture time):{" "}
              {skipped.map((f) => f.path).join(", ")}
            </div>
          )}
          {diff.files.length === 0 && (
            <div className="text-muted-foreground">
              This turn captured no files — undo is a no-op.
            </div>
          )}
        </div>

        <label className="flex items-center gap-1 text-[12px] text-muted-foreground">
          <input
            type="checkbox"
            checked={expanded}
            onChange={(e) => setExpanded(e.target.checked)}
          />
          show diffs
        </label>

        <div className="text-[11px] text-muted-foreground">
          Restores files to their state at capture time. Changes you made by hand after the
          turn to the same files are also reverted.
        </div>

        <div className="flex gap-2">
          <button
            type="button"
            className="rounded-sm border border-border px-3 py-1 text-[12px] hover:bg-accent"
            onClick={onCancel}
          >
            Cancel
          </button>
          <button
            type="button"
            className="rounded-sm bg-danger px-3 py-1 text-[12px] text-white hover:opacity-90 disabled:opacity-50"
            disabled={busy}
            onClick={async () => {
              setBusy(true);
              await onConfirm();
            }}
          >
            {busy ? "Undoing…" : `Undo ${diff.files.length} file change${diff.files.length === 1 ? "" : "s"}`}
          </button>
        </div>
      </div>
    </div>
  );
}

function DiffFile({
  entry,
  expanded,
}: {
  entry: CheckpointDiffEntry;
  expanded: boolean;
}) {
  return (
    <div className="rounded-sm border border-border">
      <div className="flex items-center gap-2 px-2 py-1">
        <span className="truncate font-mono text-[12px]">{entry.path}</span>
        <span className="ml-auto text-[11px]">
          <span className="text-success">+{entry.added}</span>{" "}
          <span className="text-danger">−{entry.removed}</span>
        </span>
      </div>
      {expanded && entry.diff && (
        <pre className="max-h-60 overflow-auto border-t border-border bg-card px-2 py-1 text-[11px] leading-snug">
          {entry.diff.split("\n").map((line, i) => (
            <div
              key={i}
              className={
                line.startsWith("+")
                  ? "text-success"
                  : line.startsWith("-")
                    ? "text-danger"
                    : "text-muted-foreground"
              }
            >
              {line || " "}
            </div>
          ))}
        </pre>
      )}
    </div>
  );
}

function formatTime(created: number | string): string {
  const d = typeof created === "number" ? new Date(created) : new Date(created);
  if (Number.isNaN(d.getTime())) return String(created);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${pad(d.getMonth() + 1)}/${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/** "a.ts, b.rs, +2 more" — first two paths as a scannable summary. */
function shortPaths(files?: CheckpointInfo["files"]): string {
  if (!files || files.length === 0) return "";
  const names = files.map((f) => f.path.split("/").pop() ?? f.path);
  const head = names.slice(0, 2).join(", ");
  return names.length > 2 ? `${head}, +${names.length - 2} more` : head;
}
