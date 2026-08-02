import { useCallback, useEffect, useState } from "react";
import type { DaemonClient } from "../api/client";
import type { CheckpointInfo, UndoTurnResult } from "../api/types";

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
    <section className="ctx-section">
      {error && <div className="panel-error">{error}</div>}
      {result && (
        <div className="cp-result">
          restored {result.restored}, skipped {result.skipped}, failed {result.failed}
        </div>
      )}
      {items.length === 0 && <div className="panel-empty">No checkpoints</div>}
      <ul className="cp-list">
        {items.map((c) => (
          <li key={c.turn_id} className="cp-item">
            <span className="cp-turn">{c.turn_id}</span>
            <span className="cp-meta">{c.file_count} files</span>
            <button type="button" className="btn-xs" onClick={() => undo(c.turn_id)}>
              Undo {c.turn_id}
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}
