/**
 * Runs one agent turn for a session: pushes the user message, drives
 * runAgentLoop, mirrors progress into the sessionManager meta (status /
 * preview), and autosaves a snapshot to the daemon after the turn.
 *
 * This is THE send entry point — App and any future session UI call
 * `runSessionTurn` and nothing else. Module-level (not a component closure):
 * it only touches the session's store and the passed-in client, so it needs no
 * component state — and keeping it out of the render scope keeps impure calls
 * like Date.now() out of the render path.
 */
import type { DaemonClient } from "../api/client";
import type { ChatMessage } from "../api/types";
import { runAgentLoop } from "./loop";
import { useSessionManager } from "../state/sessionManager";
import type { DisplayMessage } from "../state/sessionStore";

export async function runSessionTurn(
  client: DaemonClient,
  sessionId: string,
  text: string,
): Promise<void> {
  const m = useSessionManager.getState();
  const entry = m.entries[sessionId];
  if (!entry) return;
  const store = entry.store;

  store.getState().pushUserMessage(text);
  store.getState().setError(null);
  store.getState().setRunning(true);
  m.setStatus(sessionId, "running");

  // The working message history for this turn.
  const messages: ChatMessage[] = [
    ...toWireMessages(store.getState().messages),
    { role: "user", content: text },
  ];

  // Track which assistant display message is currently streaming so stream
  // events can be appended to it. Reassigned each round.
  let currentAssistantId: string | null = null;

  // AbortController for the Stop button — registered in the store so any
  // component (Composer / StatusBar) can cancel the running turn.
  const abort = new AbortController();
  store.getState().registerAbort(abort);

  try {
    await runAgentLoop({
      client,
      messages,
      sessionId,
      signal: abort.signal,
      callbacks: {
        onStreamEvent: (round, ev) => {
          // First event of a round → open a new assistant bubble.
          if (currentAssistantId === null) {
            currentAssistantId = store.getState().beginAssistantRound(round);
          }
          store.getState().appendAssistant(currentAssistantId, ev);
          if (ev.type === "contentDelta") {
            useSessionManager.getState().setPreview(sessionId, ev.text);
          }
        },
        onToolExecution: (exec) => {
          // Attach the tool card to whichever assistant message is streaming
          // (or the last assistant message if the round already finalized).
          const id =
            currentAssistantId ??
            lastAssistantId(store.getState().messages) ??
            store.getState().beginAssistantRound(0);
          store.getState().attachToolExec(id, exec);
        },
        onPermissionRequired: (info) => {
          useSessionManager.getState().setStatus(sessionId, "awaiting_approval");
          return store
            .getState()
            .requestPermission(info)
            .then((decision) => {
              // Back to running once the user decides (loop may still finish
              // with more rounds).
              useSessionManager.getState().setStatus(sessionId, "running");
              return decision;
            });
        },
      },
    });
    useSessionManager.getState().setStatus(sessionId, "idle");
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    // User-initiated stop surfaces as "aborted" — don't show it as an error.
    if (msg !== "aborted") {
      // Classify: transport failures (daemon down, network reset, stream
      // interrupted) are retryable; upstream LLM errors (rejected prompt,
      // stream error: ...) are not. Design D7.3.
      const isTransport =
        /fetch|network|failed to fetch|stream interrupted|aborted/i.test(msg) &&
        !msg.startsWith("stream error:");
      store.getState().setError({
        message: msg,
        kind: isTransport ? "transport" : "upstream",
        retry: isTransport ? () => runSessionTurn(client, sessionId, text) : undefined,
      });
      useSessionManager.getState().setStatus(sessionId, "error");
    } else {
      useSessionManager.getState().setStatus(sessionId, "idle");
    }
  } finally {
    store.getState().registerAbort(null);
    if (currentAssistantId) store.getState().finalizeAssistant(currentAssistantId);
    store.getState().setRunning(false);
  }

  await autosave(client, sessionId);
}

/** Persist a snapshot: create the daemon session on first save, then PUT. */
async function autosave(client: DaemonClient, sessionId: string): Promise<void> {
  const m = useSessionManager.getState();
  const entry = m.entries[sessionId];
  if (!entry) return;
  try {
    let daemonId = entry.daemonId;
    if (!daemonId) {
      const created = await client.createSession({ name: entry.name });
      daemonId = created.id;
      m.setDaemonId(sessionId, daemonId);
    }
    await client.saveSession(daemonId, {
      messages: toWireMessages(entry.store.getState().messages),
    });
  } catch {
    // Autosave is best-effort; the next turn retries. Don't flip the session
    // to error over a persistence hiccup.
  }
}

/** Convert display messages back to wire `ChatMessage`s for the next turn. */
function toWireMessages(display: DisplayMessage[]): ChatMessage[] {
  const out: ChatMessage[] = [];
  for (const m of display) {
    if (m.role === "user") {
      out.push({ role: "user", content: m.content });
    } else if (m.role === "assistant") {
      out.push({
        role: "assistant",
        content: m.content || undefined,
        ...(m.toolExecs && m.toolExecs.length > 0
          ? { tool_calls: m.toolExecs.map((e) => e.call) }
          : {}),
      });
      // Append tool results so the next request is well-formed.
      if (m.toolExecs) {
        for (const exec of m.toolExecs) {
          out.push({
            role: "tool",
            tool_call_id: exec.call.id,
            content: JSON.stringify(exec.response),
          });
        }
      }
    }
  }
  return out;
}

function lastAssistantId(messages: DisplayMessage[]): string | null {
  for (let i = messages.length - 1; i >= 0; i--) {
    if (messages[i].role === "assistant") return messages[i].id;
  }
  return null;
}
