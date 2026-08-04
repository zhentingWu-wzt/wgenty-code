/**
 * Runs one agent turn for a session as a SERVER-SIDE observer (Change 2 of the
 * server-side agent-loop design). The daemon owns the loop (LLM calls + tool
 * execution + persistence); we POST /run, then subscribe to the SSE event
 * stream and mirror SessionEvents into the session store for rendering.
 *
 * Replaces the old client-side runAgentLoop driver. Closing the browser no
 * longer kills the turn — the daemon keeps running; reconnect on return.
 *
 * This is THE send entry point — App and any future session UI call
 * `runSessionTurn` and nothing else. Module-level (not a component closure):
 * it only touches the session's store and the passed-in client.
 */
import type { DaemonClient } from "../api/client";
import { toast } from "sonner";
import { useSessionManager } from "../state/sessionManager";
import type { SessionStore } from "../state/sessionStore";
import type { SessionEvent, SessionEventKind } from "../api/types";

/** Pending tool invocation (started but not yet resulted). */
interface PendingTool {
  name: string;
  args: Record<string, unknown>;
}

export async function runSessionTurn(
  client: DaemonClient,
  sessionId: string,
  text: string,
): Promise<void> {
  const m = useSessionManager.getState();
  const entry = m.entries[sessionId];
  if (!entry) return;
  const store = entry.store;

  // 1. Ensure we have a daemon-side session id (POST /run needs one).
  let daemonId = entry.daemonId;
  if (!daemonId) {
    try {
      const created = await client.createSession({ name: entry.name });
      daemonId = created.id;
      m.setDaemonId(sessionId, daemonId);
    } catch (e) {
      store.getState().setError({
        message: e instanceof Error ? e.message : String(e),
        kind: "transport",
      });
      m.setStatus(sessionId, "error");
      return;
    }
  }

  // 2. Optimistic local render of the user message + running state.
  store.getState().pushUserMessage(text);
  store.getState().setError(null);
  store.getState().setRunning(true);
  m.setStatus(sessionId, "running");
  m.setPreview(sessionId, "");

  // AbortController lets the Stop button cancel the SSE reader; the actual
  // turn cancellation is POST /cancel (see stopSessionTurn below).
  const abort = new AbortController();
  store.getState().registerAbort(abort);

  // The assistant bubble events stream into; created lazily on first delta.
  let assistantId: string | null = null;
  const pendingTools: PendingTool[] = [];

  const ensureAssistant = (): string => {
    if (!assistantId) assistantId = store.getState().beginAssistantRound(1);
    return assistantId;
  };

  try {
    // 3. POST /run — daemon spawns the turn and returns immediately.
    await client.runSession(daemonId, text);

    // 4. Subscribe to the SSE event stream and mirror events into the store.
    const { body } = await client.sessionEvents(daemonId);
    const reader = body.getReader();
    let buffer = "";
    let turnFinished = false;

    while (!turnFinished) {
      const { done, value } = await reader.read();
      if (done) break;
      if (!value) continue;
      buffer += new TextDecoder().decode(value);
      let nl: number;
      while ((nl = buffer.indexOf("\n")) !== -1) {
        const line = buffer.slice(0, nl).trim();
        buffer = buffer.slice(nl + 1);
        if (!line || line.startsWith(":")) continue; // skip SSE comments/keepalives
        const payload = line.startsWith("data: ") ? line.slice(6) : line;
        let ev: SessionEvent;
        try {
          ev = JSON.parse(payload) as SessionEvent;
        } catch {
          continue;
        }
        handleEvent(ev, store, sessionId, ensureAssistant, pendingTools);
        if (ev.kind === "turn_done" || ev.kind === "turn_error") {
          // Turn finished — stop reading eagerly; the daemon also closes the
          // stream, but we don't wait for EOF to finalize UI state.
          turnFinished = true;
          reader.cancel().catch(() => {});
          break;
        }
      }
    }
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    if (msg === "aborted") {
      // User hit stop — the reader was cancelled; daemon turn may still be
      // running server-side. Status set by stopSessionTurn.
    } else {
      store.getState().setError({
        message: msg,
        kind: "transport",
        retry: () => runSessionTurn(client, sessionId, text),
      });
      m.setStatus(sessionId, "error");
      toast.error(`${entry.name}: connection lost`);
    }
  } finally {
    store.getState().registerAbort(null);
    if (assistantId) store.getState().finalizeAssistant(assistantId);
    store.getState().setRunning(false);
    if (m.entries[sessionId]?.store.getState().lastError === null) {
      m.setStatus(sessionId, "idle");
    }
  }
}

/** Cancel an active server-side turn (Stop button). */
export async function stopSessionTurn(
  client: DaemonClient,
  sessionId: string,
): Promise<void> {
  const m = useSessionManager.getState();
  const entry = m.entries[sessionId];
  if (!entry?.daemonId) return;

  // Abort the SSE reader locally (so the fetch loop unwinds).
  entry.store.getState().stopRunning();

  // Tell the daemon to cancel the run.
  try {
    await client.cancelRun(entry.daemonId);
  } catch {
    // Best-effort; the daemon may have already finished.
  }
  m.setStatus(sessionId, "idle");
}

/** Map a SessionEvent to store mutations (the rendering contract). */
function handleEvent(
  ev: SessionEvent,
  store: SessionStore,
  sessionId: string,
  ensureAssistant: () => string,
  pendingTools: PendingTool[],
): void {
  const id = ensureAssistant();
  const s = store.getState();
  switch (ev.kind as SessionEventKind) {
    case "content_delta": {
      const text = String(ev.data.text ?? "");
      s.appendAssistant(id, { type: "contentDelta", text });
      useSessionManager.getState().setPreview(sessionId, text);
      break;
    }
    case "reasoning_delta": {
      const text = String(ev.data.text ?? "");
      s.appendAssistant(id, { type: "reasoningDelta", text });
      break;
    }
    case "tool_start": {
      const name = String(ev.data.name ?? "unknown");
      const args = (ev.data.args as Record<string, unknown>) ?? {};
      pendingTools.push({ name, args });
      break;
    }
    case "tool_result": {
      const name = String(ev.data.name ?? "unknown");
      const args = (ev.data.args as Record<string, unknown>) ?? {};
      const content = String(ev.data.content ?? "");
      const pending = pendingTools.shift();
      s.attachToolExec(id, {
        call: {
          id: `server-${ev.seq}`,
          type: "function",
          function: {
            name: pending?.name ?? name,
            arguments: JSON.stringify(pending?.args ?? args),
          },
        },
        response: { success: !content.toLowerCase().startsWith("error"), content },
      });
      break;
    }
    case "turn_done":
      break; // finalization handled by the finally block
    case "turn_error": {
      const message = String(ev.data.message ?? "turn failed");
      s.setError({ message, kind: "upstream" });
      useSessionManager.getState().setStatus(sessionId, "error");
      break;
    }
    case "save":
      break; // daemon persisted; nothing to do client-side
  }
}
