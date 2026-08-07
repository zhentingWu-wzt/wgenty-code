import { useState } from "react";
import { DaemonClient } from "../../api/client";
import { usePolling } from "../../hooks/usePolling";

/**
 * Memory panel: status summary + filterable list + prune.
 *
 * Consumes the Tier 2 memory API (GET /memory/status, GET /memory,
 * POST /memory/prune). prune requires explicit confirmation.
 */
export function MemoryPanel({ client }: { client: DaemonClient }) {
  const [status, setStatus] = useState<import("../../api/types").MemoryStatus | null>(null);
  const [items, setItems] = useState<import("../../api/types").MemoryItem[]>([]);
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
    <div className="flex flex-col gap-2 p-2">
      {error && <div className="p-2 text-danger">{error}</div>}
      {status && (
        <div className="flex gap-4 px-1">
          <div className="flex flex-col">
            <span className="text-[15px] font-semibold">{status.total_memories}</span>
            <span className="text-[11px] text-muted-foreground">total</span>
          </div>
          <div className="flex flex-col">
            <span className="text-[15px] font-semibold">{status.project_count}</span>
            <span className="text-[11px] text-muted-foreground">project</span>
          </div>
          <div className="flex flex-col">
            <span className="text-[15px] font-semibold">{status.global_count}</span>
            <span className="text-[11px] text-muted-foreground">global</span>
          </div>
        </div>
      )}
      <div className="flex items-center gap-2">
        <select
          className="rounded-md border border-border bg-card px-1 py-0.5 text-[12px]"
          value={scope}
          onChange={(e) => setScope(e.target.value as "all" | "project" | "global")}
        >
          <option value="all">all</option>
          <option value="project">project</option>
          <option value="global">global</option>
        </select>
        <label className="text-[12px] text-muted-foreground">
          ≥{" "}
          <input
            className="w-14 rounded-md border border-border bg-card px-1 py-0.5 text-[12px]"
            type="number"
            min={0}
            max={1}
            step={0.1}
            value={minImportance}
            onChange={(e) => setMinImportance(Number(e.target.value))}
          />
        </label>
        <button
          type="button"
          className="rounded-sm border border-border px-2 py-0.5 text-[11px] text-danger hover:bg-accent disabled:opacity-50"
          onClick={onPrune}
          disabled={busy}
        >
          Prune
        </button>
      </div>
      {items.length === 0 ? (
        <div className="p-2 text-[12px] text-muted-foreground">No memories.</div>
      ) : (
        <ul className="flex flex-col gap-1">
          {items.map((m) => (
            <li key={`${m.origin}-${m.id}`} className="rounded-sm border border-border p-2">
              <div className="flex items-center gap-2 text-[11px] text-muted-foreground">
                <span className="text-primary">{m.origin}</span>
                <span>{m.memory_type}</span>
                <span className="ml-auto" title="importance">
                  {m.importance.toFixed(2)}
                </span>
              </div>
              <div className="pt-1 text-[13px]">{m.content}</div>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
