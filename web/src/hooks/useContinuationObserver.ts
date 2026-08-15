import { useEffect } from "react";
import type { DaemonClient } from "../api/client";
import { observeDaemonRun } from "../agent/sessionRunner";

const INITIAL_BACKOFF_MS = 1000;
const MAX_BACKOFF_MS = 30_000;

/** Envelope of one event on the daemon-wide global bus (`GET /api/v1/events`). */
interface GlobalEvent {
  seq: number;
  kind: string;
  data: Record<string, unknown>;
}

/**
 * Subscribe to the daemon-wide global event stream and attach live observers
 * to daemon-initiated runs.
 *
 * Why: the daemon's continuation scheduler claims ready subagent task groups
 * server-side and spawns a synthesis turn for the main agent (so subagent
 * results reach the main agent even when no polling client like the TUI is
 * attached). Web only renders runs it started itself, so without this hook
 * those synthesis turns — the subagent results reaching the main agent —
 * were invisible until a manual refresh.
 *
 * On `task_group_result` (broadcast by whoever claimed the group — daemon
 * scheduler or the TUI's claim endpoint) we attach `observeDaemonRun` to the
 * named session. One permanent stream, same reconnect discipline as
 * usePermissionTrace.
 */
export function useContinuationObserver(client: DaemonClient | null): void {
  useEffect(() => {
    if (!client) return;
    const ctl = new AbortController();
    let cancelled = false;
    const cancelWaiters: Array<() => void> = [];

    const runStream = async () => {
      let backoff = INITIAL_BACKOFF_MS;
      while (!cancelled) {
        let reader: ReadableStreamDefaultReader<Uint8Array> | null = null;
        try {
          const { body } = await client.globalEvents(ctl.signal);
          if (cancelled) return;
          reader = body.getReader();
          backoff = INITIAL_BACKOFF_MS;
          let buffer = "";
          for (;;) {
            const { done, value } = await reader.read();
            if (done) break;
            if (!value) continue;
            buffer += new TextDecoder().decode(value);
            let nl: number;
            while ((nl = buffer.indexOf("\n")) !== -1) {
              const line = buffer.slice(0, nl).trim();
              buffer = buffer.slice(nl + 1);
              if (!line || line.startsWith(":")) continue; // SSE keepalives
              const payload = line.startsWith("data: ") ? line.slice(6) : line;
              let ev: GlobalEvent;
              try {
                ev = JSON.parse(payload) as GlobalEvent;
              } catch {
                continue;
              }
              if (ev.kind === "task_group_result") {
                const sessionId = String(ev.data.session_id ?? "");
                if (sessionId) void observeDaemonRun(client, sessionId);
              }
            }
          }
        } catch {
          // Aborted by cleanup, or a transient drop — fall through to backoff.
        } finally {
          reader?.cancel().catch(() => {});
        }

        if (cancelled) return;
        await new Promise<void>((resolve) => {
          const timer = setTimeout(() => resolve(), backoff);
          backoff = Math.min(backoff * 2, MAX_BACKOFF_MS);
          cancelWaiters.push(() => {
            clearTimeout(timer);
            resolve();
          });
        });
      }
    };

    void runStream();

    return () => {
      cancelled = true;
      ctl.abort();
      for (const cancel of cancelWaiters) cancel();
    };
  }, [client]);
}
