import { useEffect, useRef, useState } from "react";
import { DaemonClient } from "./api/client";
import { runAgentLoop } from "./agent/loop";
import { useSessionManager, getActiveSessionStore } from "./state/sessionManager";
import { SessionStoreContext } from "./state/sessionContext";
import type { SessionState } from "./state/sessionStore";
import { StatusBar } from "./components/StatusBar";
import { Sidebar } from "./components/Sidebar";
import { ChatView } from "./components/ChatView";
import { Composer } from "./components/Composer";
import { PermissionModal } from "./components/PermissionModal";
import { usePermissionTrace } from "./hooks/usePermissionTrace";
import { usePolling } from "./hooks/usePolling";
import type { ChatMessage } from "./api/types";

/**
 * App — wires the agent loop to the UI store.
 *
 * The architecture mirrors `src/tui` as a parallel thin client: this React app
 * is just another frontend over the same daemon API. The agent loop runs in the
 * browser (the daemon's `/chat/stream` is a pure passthrough proxy; tools are
 * executed client-side via `/tools/execute`).
 */
export function App() {
  // One stable client for the app's lifetime. useState's lazy initializer is
  // the idiomatic way to hold a non-reactive instance (avoids reading a ref
  // during render).
  const [client] = useState(() => new DaemonClient());

  const setConnection = useSessionManager((s) => s.setConnection);
  const setModelName = useSessionManager((s) => s.setModelName);

  // Active session's store. Still single-session: the bootstrap effect below
  // creates one local session on first mount and nothing ever creates more.
  const activeStore = useSessionManager((s) =>
    s.activeId ? s.entries[s.activeId].store : null,
  );

  // Bootstrap one local session. Must live in an effect — creating a session
  // during render is a render-phase side effect (react-hooks purity rules).
  useEffect(() => {
    if (!useSessionManager.getState().activeId) {
      useSessionManager.getState().createLocalSession();
    }
  }, []);

  // Subscribe to the trace SSE for pushed subagent permission prompts
  // (design D2.1: replaces 500ms polling of /tools/pending-permissions).
  usePermissionTrace(client);

  // ── Daemon health heartbeat (design D7.1) ──────────────────────────────────
  // Poll /health on a slow cadence so the status bar reflects daemon
  // death/recovery (was: one-shot probe that lied forever after a restart).
  // Model name is read once on first connect.
  const modelLoadedRef = useRef(false);
  usePolling(
    async () => {
      try {
        await client.health();
        setConnection("connected");
        if (!modelLoadedRef.current) {
          modelLoadedRef.current = true;
          const cfg = await client.getConfig();
          setModelName(cfg.model);
        }
      } catch {
        setConnection("disconnected");
      }
    },
    true,
    10_000,
  );

  // First render (before the bootstrap effect runs): no session yet.
  if (!activeStore) return null;

  return (
    <SessionStoreContext.Provider value={activeStore}>
      <div className="app">
        <StatusBar />
        <div className="app-body">
          <Sidebar client={client} />
          <div className="app-main">
            <main className="main">
              <ChatView />
            </main>
            <Composer onSend={(text) => handleSend(client, text)} />
          </div>
        </div>
        <PermissionModal client={client} />
      </div>
    </SessionStoreContext.Provider>
  );
}

// ── Send: push the user message, run a full agent turn ───────────────────────
// Module-level (not a component closure): it only touches the active session's
// store and the passed-in client, so it needs no component state — and keeping
// it out of the render scope keeps impure calls like Date.now() out of the
// render path.
async function handleSend(client: DaemonClient, text: string) {
  const store = getActiveSessionStore();
  if (!store) return;
  const s = store.getState();

  s.pushUserMessage(text);
  s.setError(null);
  s.setRunning(true);

  // The working message history for this turn. In MVP we keep a single
  // in-memory conversation; session persistence is a second-phase feature.
  const messages: ChatMessage[] = [...toWireMessages(s.messages), { role: "user", content: text }];
  const sessionId = `web-${Date.now()}`;

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
        onPermissionRequired: (info) => store.getState().requestPermission(info),
      },
    });
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    // User-initiated stop surfaces as "aborted" — don't show it as an error.
    if (msg === "aborted") {
      // no-op
    } else {
      // Classify: transport failures (daemon down, network reset, stream
      // interrupted) are retryable; upstream LLM errors (rejected prompt,
      // stream error: ...) are not. Design D7.3.
      const isTransport =
        /fetch|network|failed to fetch|stream interrupted|aborted/i.test(msg) &&
        !msg.startsWith("stream error:");
      store.getState().setError({
        message: msg,
        kind: isTransport ? "transport" : "upstream",
        retry: isTransport ? () => handleSend(client, text) : undefined,
      });
    }
  } finally {
    store.getState().registerAbort(null);
    if (currentAssistantId) store.getState().finalizeAssistant(currentAssistantId);
    store.getState().setRunning(false);
  }
}

/** Convert display messages back to wire `ChatMessage`s for the next turn. */
function toWireMessages(display: SessionState["messages"]): ChatMessage[] {
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

function lastAssistantId(messages: SessionState["messages"]): string | null {
  for (let i = messages.length - 1; i >= 0; i--) {
    if (messages[i].role === "assistant") return messages[i].id;
  }
  return null;
}
