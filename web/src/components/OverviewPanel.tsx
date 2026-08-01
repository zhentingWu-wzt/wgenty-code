import { useState } from "react";
import { DaemonClient } from "../api/client";
import { usePolling } from "../hooks/usePolling";

/**
 * Overview panel — assembled client-side from existing endpoints (design OQ2:
 * avoid a new /overview backend surface). Shows health, session/memory counts,
 * and the current model. Refreshes on a slow poll.
 */
export function OverviewPanel({ client }: { client: DaemonClient }) {
  const [health, setHealth] = useState<{ status: string; version: string } | null>(null);
  const [sessionCount, setSessionCount] = useState<number | null>(null);
  const [memCount, setMemCount] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  usePolling(
    async () => {
      try {
        const [h, sessions, mem] = await Promise.all([
          client.health(),
          client.listSessions(),
          client.memoryStatus().catch(() => null),
        ]);
        setHealth(h);
        setSessionCount(sessions.length);
        setMemCount(mem?.total_memories ?? null);
        setError(null);
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      }
    },
    true,
    10000,
  );

  return (
    <div className="overview-panel">
      {error && <div className="panel-error">{error}</div>}
      <div className="ov-grid">
        <OvRow label="Daemon" value={health?.status ?? "…"} ok={health?.status === "ok"} />
        <OvRow label="Version" value={health?.version ?? "…"} />
        <OvRow label="Sessions" value={sessionCount ?? "…"} />
        <OvRow label="Memories" value={memCount ?? "…"} />
      </div>
    </div>
  );
}

function OvRow({ label, value, ok }: { label: string; value: string | number; ok?: boolean }) {
  return (
    <div className="ov-row">
      <span className="ov-label">{label}</span>
      <span className={`ov-value ${ok === false ? "ov-bad" : ok === true ? "ov-ok" : ""}`}>
        {value}
      </span>
    </div>
  );
}
