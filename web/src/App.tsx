import { useEffect, useRef, useState } from "react";
import { DaemonClient } from "./api/client";
import { runSessionTurn } from "./agent/sessionRunner";
import { useSessionManager } from "./state/sessionManager";
import { SessionStoreContext } from "./state/sessionContext";
import { StatusBar } from "./components/StatusBar";
import { Sidebar } from "./components/Sidebar";
import { SessionHeader } from "./components/SessionHeader";
import { ChatView } from "./components/ChatView";
import { Composer } from "./components/Composer";
import { PermissionModal } from "./components/PermissionModal";
import { usePermissionTrace } from "./hooks/usePermissionTrace";
import { usePolling } from "./hooks/usePolling";

/**
 * App — wires the per-session agent runners to the UI stores.
 *
 * The architecture mirrors `src/tui` as a parallel thin client: this React app
 * is just another frontend over the same daemon API. Agent loops run in the
 * browser (the daemon's `/chat/stream` is a pure passthrough proxy; tools are
 * executed client-side via `/tools/execute`), one `runSessionTurn` per session,
 * so sessions progress concurrently and independently.
 */
export function App() {
  // One stable client for the app's lifetime. useState's lazy initializer is
  // the idiomatic way to hold a non-reactive instance (avoids reading a ref
  // during render).
  const [client] = useState(() => new DaemonClient());

  const setConnection = useSessionManager((s) => s.setConnection);
  const setModelName = useSessionManager((s) => s.setModelName);
  const activeId = useSessionManager((s) => s.activeId);

  // Active session's store. Each session keeps its own store; only the active
  // one is provided to the center pane.
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

  // Warn before unloading the page while any session is mid-turn (the loops
  // live in the browser; leaving kills them).
  useEffect(() => {
    const handler = (e: BeforeUnloadEvent) => {
      const running = Object.values(useSessionManager.getState().entries).filter(
        (x) => x.status === "running" || x.status === "awaiting_approval",
      ).length;
      if (running > 0) e.preventDefault();
    };
    window.addEventListener("beforeunload", handler);
    return () => window.removeEventListener("beforeunload", handler);
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
        // Daemon is gone — any in-flight turn is dead. Mark running sessions
        // errored so they don't sit in "running" forever.
        const m = useSessionManager.getState();
        for (const e of Object.values(m.entries)) {
          if (e.status === "running" || e.status === "awaiting_approval") {
            m.setStatus(e.id, "error");
          }
        }
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
            <SessionHeader />
            <main className="main">
              <ChatView />
            </main>
            <Composer
              onSend={(text) => {
                if (activeId) void runSessionTurn(client, activeId, text);
              }}
            />
          </div>
        </div>
        <PermissionModal client={client} />
      </div>
    </SessionStoreContext.Provider>
  );
}
