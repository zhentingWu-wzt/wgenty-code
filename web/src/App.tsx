import { useEffect, useRef } from "react";
import { DaemonClient } from "./api/client";
import { runAgentLoop } from "./agent/loop";
import { useChatStore } from "./state/chatStore";
import { StatusBar } from "./components/StatusBar";
import { Sidebar } from "./components/Sidebar";
import { ChatView } from "./components/ChatView";
import { Composer } from "./components/Composer";
import { PermissionModal } from "./components/PermissionModal";
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
  const clientRef = useRef<DaemonClient | null>(null);
  if (!clientRef.current) clientRef.current = new DaemonClient();

  const store = useChatStore;
  const setConnection = useChatStore((s) => s.setConnection);
  const setModelName = useChatStore((s) => s.setModelName);

  // ── Startup: probe the daemon + read current model ─────────────────────────
  useEffect(() => {
    const client = clientRef.current!;
    let cancelled = false;

    (async () => {
      try {
        await client.health();
        if (cancelled) return;
        setConnection("connected");
        const cfg = await client.getConfig();
        if (!cancelled) setModelName(cfg.model);
      } catch {
        if (!cancelled) setConnection("disconnected");
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [setConnection, setModelName]);

  // ── Send: push the user message, run a full agent turn ─────────────────────
  const handleSend = async (text: string) => {
    const client = clientRef.current!;
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
      if (msg !== "aborted") store.getState().setError(msg);
    } finally {
      store.getState().registerAbort(null);
      if (currentAssistantId) store.getState().finalizeAssistant(currentAssistantId);
      store.getState().setRunning(false);
    }
  };

  return (
    <div className="app">
      <StatusBar />
      <div className="app-body">
        <Sidebar client={clientRef.current} />
        <div className="app-main">
          <main className="main">
            <ChatView />
          </main>
          <Composer onSend={handleSend} />
        </div>
      </div>
      <PermissionModal />
    </div>
  );
}

/** Convert display messages back to wire `ChatMessage`s for the next turn. */
function toWireMessages(display: ReturnType<typeof useChatStore.getState>["messages"]): ChatMessage[] {
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

function lastAssistantId(messages: ReturnType<typeof useChatStore.getState>["messages"]): string | null {
  for (let i = messages.length - 1; i >= 0; i--) {
    if (messages[i].role === "assistant") return messages[i].id;
  }
  return null;
}
