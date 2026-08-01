import { useEffect, useState } from "react";
import type { DaemonClient } from "../api/client";
import type { TraceEvent } from "../api/types";
import { useSessionManager } from "../state/sessionManager";

interface TraceLine {
  ts: number;
  text: string;
}

/** Live subagent activity for the active session, from the trace SSE stream.
 *  Line-oriented: each event becomes one row; capped at 100.
 *
 *  Opens a second traceStream subscription alongside usePermissionTrace's —
 *  acceptable because the daemon broadcasts to all subscribers. Both stay
 *  silent when the daemon is down (StatusBar owns connection reporting). */
export function SubagentPanel({ client }: { client: DaemonClient }) {
  const [lines, setLines] = useState<TraceLine[]>([]);
  const activeId = useSessionManager((s) => s.activeId);

  // Reset the timeline on session switch. This is the render-phase "adjust
  // state when props change" pattern — a synchronous setState inside the
  // effect below is rejected by react-hooks/set-state-in-effect.
  const [prevActiveId, setPrevActiveId] = useState(activeId);
  if (prevActiveId !== activeId) {
    setPrevActiveId(activeId);
    setLines([]);
  }

  useEffect(() => {
    if (!activeId) return;
    const abort = new AbortController();
    // Held at effect scope so the cleanup can cancel an in-flight read —
    // abort.abort() alone does not resolve a pending reader.read(), and
    // without cancel() every session switch would leak an SSE connection.
    let reader: ReadableStreamDefaultReader<Uint8Array> | null = null;
    (async () => {
      try {
        const { body } = await client.traceStream();
        reader = body.getReader();
        const decoder = new TextDecoder();
        let buf = "";
        for (;;) {
          const { done, value } = await reader.read();
          if (done || abort.signal.aborted) break;
          buf += decoder.decode(value, { stream: true });
          const rows = buf.split("\n");
          buf = rows.pop() ?? "";
          for (const row of rows) {
            if (!row.trim()) continue;
            try {
              const ev = JSON.parse(row) as TraceEvent;
              if (ev.session_id !== activeId) continue;
              const tool = ev.current_tool ? ` [${ev.current_tool}]` : "";
              const text = `${ev.kind ?? "event"}: ${ev.label ?? ""}${tool}`;
              setLines((prev) => [...prev.slice(-99), { ts: Date.now(), text }]);
            } catch {
              // 半行/非 JSON 行忽略
            }
          }
        }
      } catch {
        // daemon down — the panel just stays empty; StatusBar shows connection
      }
    })();
    return () => {
      abort.abort();
      reader?.cancel().catch(() => {});
    };
  }, [client, activeId]);

  return (
    <section className="ctx-section">
      <span className="rail-section-title">Subagents</span>
      {lines.length === 0 && <div className="panel-empty">No subagent activity</div>}
      <ul className="trace-list">
        {lines.map((l, i) => (
          <li key={i} className="trace-line">
            {l.text}
          </li>
        ))}
      </ul>
    </section>
  );
}
