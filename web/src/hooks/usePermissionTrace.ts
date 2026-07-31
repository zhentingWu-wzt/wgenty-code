import { useEffect } from "react";
import type { DaemonClient } from "../api/client";
import type { TraceEvent } from "../api/types";
import { useChatStore } from "../state/chatStore";

/**
 * Subscribe to the daemon's trace SSE stream and surface subagent permission
 * prompts as they arrive (design D2.1: push, not poll).
 *
 * On `permission_pending` we push the approval into the chat store; the
 * PermissionModal renders it and, on user choice, the hook's companion
 * `resolveSubagent` (wired in App) calls `client.resolveSubagentPermission`.
 * `permission_resolved` events clear a prompt that was answered elsewhere.
 *
 * `progress` events are ignored here (a future trace panel could render them).
 */
export function usePermissionTrace(client: DaemonClient | null): void {
  const pushSubagent = useChatStore((s) => s.pushSubagentPermission);
  const clearSubagent = useChatStore((s) => s.clearSubagentPermission);

  useEffect(() => {
    if (!client) return;
    let cancelled = false;
    let reader: ReadableStreamDefaultReader<Uint8Array> | null = null;
    let buffer = "";

    const start = async () => {
      try {
        const { body } = await client.traceStream();
        if (cancelled) return;
        reader = body.getReader();
        for (;;) {
          const { done, value } = await reader.read();
          if (done) break;
          if (!value) continue;
          buffer += new TextDecoder().decode(value);
          // Trace SSE is newline-delimited JSON (one TraceEvent per line).
          let nl: number;
          while ((nl = buffer.indexOf("\n")) !== -1) {
            const line = buffer.slice(0, nl).trim();
            buffer = buffer.slice(nl + 1);
            if (!line) continue;
            try {
              handleEvent(JSON.parse(line) as TraceEvent);
            } catch {
              // Keep-alive or partial; ignore unparseable lines.
            }
          }
        }
      } catch {
        // Transient (daemon restart, network). The effect cleanup + re-run on
        // next render will retry; for now just bail.
      }
    };

    const handleEvent = (ev: TraceEvent) => {
      if (ev.kind === "permission_pending" && ev.permission) {
        pushSubagent(ev.permission);
      } else if (ev.kind === "permission_resolved") {
        // Resolved elsewhere (timeout, or another client) — dismiss.
        clearSubagent();
      }
    };

    start();
    return () => {
      cancelled = true;
      if (reader) reader.cancel().catch(() => {});
    };
  }, [client, pushSubagent, clearSubagent]);
}
