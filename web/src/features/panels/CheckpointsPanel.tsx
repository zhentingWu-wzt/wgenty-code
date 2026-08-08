import { useCallback, useEffect, useState } from "react";
import type { DaemonClient } from "../../api/client";
import type { CheckpointInfo, UndoTurnResult } from "../../api/types";

/** Per-turn file checkpoints with undo (GET /checkpoints, POST /tools/undo-turn).
 *  Global store — not per-session (daemon limitation, same as Todos). */
export function CheckpointsPanel({ client }: { client: DaemonClient }) {
  const [items, setItems] = useState<CheckpointInfo[]>([]);
  const [result, setResult] = useState<UndoTurnResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    client
      .listCheckpoints()
      .then(setItems)
      .catch((e) => setError(e instanceof Error ? e.message : String(e)));
  }, [client]);

  useEffect(refresh, [refresh]);

  const undo = async (turnId: string) => {
    if (!window.confirm(`Undo turn ${turnId}? Files return to their pre-turn state.`)) return;
    try {
      setResult(await client.undoTurns([turnId]));
      refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
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
          <li key={c.turn_id} className="flex items-center gap-2 rounded-sm px-2 py-1">
            <span className="text-[13px]">{c.turn_id}</span>
            <span className="text-[11px] text-muted-foreground">{c.file_count} files</span>
            <button
              type="button"
              className="ml-auto rounded-sm border border-border px-2 py-0.5 text-[11px] hover:bg-accent"
              onClick={() => undo(c.turn_id)}
            >
              Undo {c.turn_id}
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}
