import { useEffect, useRef, useState } from "react";
import { Toaster, toast } from "sonner";
import { DaemonClient } from "./api/client";
import { runSessionTurn, stopSessionTurn } from "./agent/sessionRunner";
import { useSessionManager } from "./state/sessionManager";
import { SessionStoreContext } from "./state/sessionContext";
import { StatusBar } from "./components/StatusBar";
import { LeftRail } from "./components/LeftRail";
import { SessionHeader } from "./components/SessionHeader";
import { ChatView } from "./components/ChatView";
import { Composer } from "./components/Composer";
import { PermissionModal } from "./components/PermissionModal";
import { QuestionModal } from "./components/QuestionModal";
import { CommandModal } from "./components/CommandModal";
import { SessionsBrowserModal } from "./components/SessionsBrowserModal";
import { ModelPanel } from "./components/ModelPanel";
import { MemoryPanel } from "./components/MemoryPanel";
import { CheckpointsPanel } from "./components/CheckpointsPanel";
import type { SlashCommand } from "./components/slashCommands";
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
  const activeStore = useSessionManager((s) => (s.activeId ? s.entries[s.activeId].store : null));

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
  // Dedupes the "Daemon disconnected" toast — the poll runs every 10s and
  // would otherwise re-toast on every failed tick while the daemon is down.
  const disconnectToastedRef = useRef(false);
  usePolling(
    async () => {
      try {
        await client.health();
        setConnection("connected");
        disconnectToastedRef.current = false;
        if (!modelLoadedRef.current) {
          modelLoadedRef.current = true;
          const cfg = await client.getConfig();
          setModelName(cfg.model);
        }
      } catch {
        setConnection("disconnected");
        if (!disconnectToastedRef.current) {
          disconnectToastedRef.current = true;
          toast.error("Daemon disconnected");
        }
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

  // Slash-command modal (/model, /sessions, /memory, /undo) — the TUI-style
  // replacement for permanent side panels.
  const [openCommand, setOpenCommand] = useState<SlashCommand | null>(null);
  const closeCommand = () => setOpenCommand(null);

  // First render (before the bootstrap effect runs): no session yet.
  if (!activeStore) return null;

  return (
    <SessionStoreContext.Provider value={activeStore}>
      <div className="app">
        <StatusBar />
        <div className="app-body">
          <LeftRail client={client} />
          <div className="app-main">
            <SessionHeader />
            <main className="main">
              <ChatView />
            </main>
            <Composer
              onSend={(text) => {
                if (activeId) void runSessionTurn(client, activeId, text);
              }}
              onStop={() => {
                if (activeId) void stopSessionTurn(client, activeId);
              }}
              onCommand={setOpenCommand}
            />
          </div>
        </div>
        <PermissionModal client={client} />
        <QuestionModal client={client} />
        {openCommand?.name === "/model" && (
          <CommandModal title="Switch model" onClose={closeCommand}>
            <ModelPanel client={client} />
          </CommandModal>
        )}
        {openCommand?.name === "/sessions" && (
          <SessionsBrowserModal client={client} onClose={closeCommand} />
        )}
        {openCommand?.name === "/memory" && (
          <CommandModal title="Memory" onClose={closeCommand}>
            <MemoryPanel client={client} />
          </CommandModal>
        )}
        {openCommand?.name === "/undo" && (
          <CommandModal title="Undo turn" onClose={closeCommand}>
            <CheckpointsPanel client={client} />
          </CommandModal>
        )}
        <Toaster theme="dark" position="bottom-right" />
      </div>
    </SessionStoreContext.Provider>
  );
}
