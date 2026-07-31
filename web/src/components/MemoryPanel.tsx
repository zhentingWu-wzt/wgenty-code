import { useState } from "react";
import { DaemonClient } from "../api/client";
import { usePolling } from "../hooks/usePolling";

/**
 * Memory panel: status summary + filterable list + prune.
 *
 * Consumes the Tier 2 memory API (GET /memory/status, GET /memory,
 * POST /memory/prune). prune requires explicit confirmation.
 */
export function MemoryPanel({ client }: { client: DaemonClient }) {
  const [status, setStatus] = useState<import("../api/types").MemoryStatus | null>(null);
  const [items, setItems] = useState<import("../api/types").MemoryItem[]>([]);
  const [scope, setScope] = useState<"all" | "project" | "global">("all");
  const [minImportance, setMinImportance] = useState(0);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = async () => {
    try {
      const [s, list] = await Promise.all([
        client.memoryStatus(),
        client.listMemory({ scope, min_importance: minImportance, limit: 200 }),
      ]);
      setStatus(s);
      setItems(list.items);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  usePolling(refresh, true, 10000);

  const onPrune = async () => {
    if (!confirm("Prune low-importance memories? This cannot be undone.")) return;
    setBusy(true);
    setError(null);
    try {
      await client.pruneMemory(false);
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="memory-panel">
      {error && <div className="panel-error">{error}</div>}
      {status && (
        <div className="memory-status">
          <div className="mem-stat">
            <span className="mem-stat-num">{status.total_memories}</span>
            <span className="mem-stat-label">total</span>
          </div>
          <div className="mem-stat">
            <span className="mem-stat-num">{status.project_count}</span>
            <span className="mem-stat-label">project</span>
          </div>
          <div className="mem-stat">
            <span className="mem-stat-num">{status.global_count}</span>
            <span className="mem-stat-label">global</span>
          </div>
        </div>
      )}
      <div className="memory-toolbar">
        <select
          className="mem-scope"
          value={scope}
          onChange={(e) => setScope(e.target.value as "all" | "project" | "global")}
        >
          <option value="all">all</option>
          <option value="project">project</option>
          <option value="global">global</option>
        </select>
        <label className="mem-min-label">
          ≥{" "}
          <input
            className="mem-min"
            type="number"
            min={0}
            max={1}
            step={0.1}
            value={minImportance}
            onChange={(e) => setMinImportance(Number(e.target.value))}
          />
        </label>
        <button type="button" className="btn btn-xs btn-danger" onClick={onPrune} disabled={busy}>
          Prune
        </button>
      </div>
      {items.length === 0 ? (
        <div className="panel-empty">No memories.</div>
      ) : (
        <ul className="memory-list">
          {items.map((m) => (
            <li key={`${m.origin}-${m.id}`} className={`memory-item origin-${m.origin}`}>
              <div className="memory-head">
                <span className="memory-origin">{m.origin}</span>
                <span className="memory-type">{m.memory_type}</span>
                <span className="memory-imp" title="importance">
                  {m.importance.toFixed(2)}
                </span>
              </div>
              <div className="memory-content">{m.content}</div>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
