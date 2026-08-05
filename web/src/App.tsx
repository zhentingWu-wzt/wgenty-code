import { useEffect, useRef, useState } from "react";
import { Toaster, toast } from "sonner";
import { DaemonClient } from "./api/client";
import { runSessionTurn, stopSessionTurn } from "./agent/sessionRunner";
import { useSessionManager } from "./state/sessionManager";
import { SessionStoreContext } from "./state/sessionContext";
import { StatusBar } from "./components/StatusBar";
import { LeftSidebar } from "./components/layout/LeftSidebar";
import { SessionTabBar } from "./components/layout/SessionTabBar";
import { ChatView } from "./features/chat/ChatView";
import { Composer } from "./features/chat/Composer";
import { PermissionModal } from "./features/permissions/PermissionModal";
import { QuestionModal } from "./features/permissions/QuestionModal";
import { CommandModal } from "./features/panels/CommandModal";
import { RightRail } from "./components/layout/RightRail";
import { ModelPanel } from "./features/panels/ModelPanel";
import { AppTopbar } from "./components/layout/AppTopbar";
import type { SlashCommand } from "./components/slashCommands";
import { usePermissionTrace } from "./hooks/usePermissionTrace";
import { usePolling } from "./hooks/usePolling";
import { startUiSync } from "./state/uiSync";
import { useUiStore } from "./state/uiStore";

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
  const theme = useUiStore((s) => s.theme);

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

  // sessionManager → uiStore.openTabs 单向同步（激活补开 tab、删除剪 tab）。
  useEffect(() => startUiSync(), []);

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

  // Slash commands: `/model` opens a modal; `/sessions` `/memory` `/undo`
  // toggle the corresponding right-rail panel.
  const [openCommand, setOpenCommand] = useState<SlashCommand | null>(null);
  const closeCommand = () => setOpenCommand(null);
  const handleCommand = (cmd: SlashCommand) => {
    const ui = useUiStore.getState();
    switch (cmd.name) {
      case "/model":
        setOpenCommand(cmd);
        break;
      case "/sessions":
        ui.toggleRightPanel("sessions");
        break;
      case "/memory":
        ui.toggleRightPanel("memory");
        break;
      case "/undo":
        ui.toggleRightPanel("checkpoints");
        break;
    }
  };

  // First render (before the bootstrap effect runs): no session yet.
  if (!activeStore) return null;

  return (
    <div className="flex h-screen flex-col bg-background text-foreground">
      <AppTopbar />
      <div className="flex min-h-0 flex-1">
        <LeftSidebar client={client} />
        <SessionStoreContext.Provider value={activeStore}>
          <div className="flex min-w-0 flex-1 flex-col">
            <SessionTabBar />
            <main className="min-h-0 flex-1 overflow-y-auto">
              <ChatView />
            </main>
            <Composer
              onSend={(text) => {
                if (activeId) void runSessionTurn(client, activeId, text);
              }}
              onStop={() => {
                if (activeId) void stopSessionTurn(client, activeId);
              }}
              onCommand={handleCommand}
            />
          </div>
          <PermissionModal client={client} />
          <QuestionModal client={client} />
        </SessionStoreContext.Provider>
        <RightRail client={client} />
      </div>
      <StatusBar />
      {openCommand?.name === "/model" && (
        <CommandModal title="Switch model" onClose={closeCommand}>
          <ModelPanel client={client} />
        </CommandModal>
      )}
      <Toaster theme={theme} position="bottom-right" />
    </div>
  );
}
