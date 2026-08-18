import { useEffect, useRef, useState } from "react";
import { Toaster, toast } from "sonner";
import { DaemonClient } from "./api/client";
import { wsChannel } from "./api/wsChannel";
import { getPlatform } from "./platform";
import { runSessionTurn, stopSessionTurn } from "./agent/sessionRunner";
import { useContinuationObserver } from "./hooks/useContinuationObserver";
import { useSessionManager } from "./state/sessionManager";
import { SessionStoreContext } from "./state/sessionContext";
import { ConfirmProvider } from "./components/ui/ConfirmModal";
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
import { SubagentDetailPanel } from "./features/panels/SubagentDetailPanel";
import { PreviewPanel } from "./features/files/PreviewPanel";
import { DiffView } from "./features/scm/DiffView";
import { AppTopbar } from "./components/layout/AppTopbar";
import type { SlashCommand } from "./components/slashCommands";
import { sessionMessagesToDisplay } from "./agent/sessionLoad";
import { usePermissionTrace } from "./hooks/usePermissionTrace";
import { usePermissionModeSync } from "./hooks/usePermissionModeSync";
import { useSubagentDirectory } from "./hooks/useSubagentDirectory";
import { usePolling } from "./hooks/usePolling";
import { startUiSync } from "./state/uiSync";
import { useUiStore } from "./state/uiStore";

/** How many most-recent daemon sessions to restore into the left rail on
 *  startup (newest first). Mirrors the TUI's behavior of resuming the
 *  latest conversation history. */
const RECENT_SESSION_LIMIT = 5;

/**
 * App — wires the per-session agent runners to the UI stores.
 *
 * The architecture mirrors `src/tui` as a parallel thin client: this React app
 * is just another frontend over the same daemon API. The daemon owns the agent
 * loop (`POST /sessions/:id/run` runs LLM + tools + persistence server-side);
 * each `runSessionTurn` POSTs a run then subscribes to the session-event SSE
 * stream, mirroring events into the store. Sessions progress concurrently and
 * independently, and survive a browser close/reopen.
 */
export function App() {
  // One stable client for the app's lifetime. useState's lazy initializer is
  // the idiomatic way to hold a non-reactive instance (avoids reading a ref
  // during render).
  const [client] = useState(() => new DaemonClient());

  const setConnection = useSessionManager((s) => s.setConnection);
  const setModelName = useSessionManager((s) => s.setModelName);
  const setContextWindow = useSessionManager((s) => s.setContextWindow);
  const activeId = useSessionManager((s) => s.activeId);
  const theme = useUiStore((s) => s.theme);

  // Active session's store. Each session keeps its own store; only the active
  // one is provided to the center pane.
  const activeStore = useSessionManager((s) => (s.activeId ? s.entries[s.activeId].store : null));

  // Unified active tab: a session id, `subagent:<nodeId>`, or
  // `preview:<absPath>`. When it's a subagent tab, the center pane shows the
  // subagent detail panel instead of the chat view (the active session store
  // stays bound for the Composer); a preview tab likewise swaps in the file
  // preview panel.
  const activeTabId = useUiStore((s) => s.activeTabId);
  const subagentTabs = useUiStore((s) => s.subagentTabs);
  const subagentTabMeta =
    activeTabId && activeTabId.startsWith("subagent:") ? subagentTabs[activeTabId] : undefined;
  const previewTabs = useUiStore((s) => s.previewTabs);
  const previewTabMeta =
    activeTabId && activeTabId.startsWith("preview:") ? previewTabs[activeTabId] : undefined;
  const diffTabs = useUiStore((s) => s.diffTabs);
  const diffTabMeta =
    activeTabId && activeTabId.startsWith("diff:") ? diffTabs[activeTabId] : undefined;

  // Bootstrap: restore the most recent daemon sessions so the left rail shows
  // real history on startup, and activate the newest one (TUI-aligned). When
  // the daemon is unreachable (e.g. browser before the user starts it) or there
  // is no history yet, fall back to one fresh local session. Must live in an
  // effect — creating a session during render is a render-phase side effect
  // (react-hooks purity rules).
  useEffect(() => {
    if (useSessionManager.getState().activeId) return; // already bootstrapped
    let cancelled = false;
    (async () => {
      try {
        const sessions = await client.listSessions();
        if (cancelled) return;
        // Newest first; archived sessions stay hidden from the default view
        // (same client-side filtering as the right-rail Sessions panel).
        const recent = sessions
          .filter((s) => s.status !== "Archived")
          .sort((a, b) => b.updated_at.localeCompare(a.updated_at))
          .slice(0, RECENT_SESSION_LIMIT);
        if (recent.length > 0) {
          // Load full history in parallel, then restore each session — the
          // same path the right-rail Sessions panel uses to open one.
          const loaded = await Promise.all(
            recent.map(async (info) => {
              const full = await client.loadSession(info.id).catch(() => null);
              return full ? { info, full } : null;
            }),
          );
          if (cancelled) return;
          for (const r of loaded) {
            if (!r) continue; // raced / daemon down mid-restore → skip
            const { info, full } = r;
            const state = useSessionManager.getState();
            if (Object.values(state.entries).some((e) => e.daemonId === info.id)) {
              continue; // already open (e.g. opened via the Sessions panel)
            }
            const localId = state.createLocalSession(info.name ?? "Session", {
              id: info.id,
              daemonId: info.id,
              projectPath: info.project_path ?? null,
              ...(info.worktree ? { worktree: info.worktree } : {}),
            });
            const store = useSessionManager.getState().entries[localId].store;
            for (const dm of sessionMessagesToDisplay(full.messages ?? [])) {
              store.getState().pushLoadedMessage(dm);
            }
          }
          // Activate the newest restored session (first after the desc sort;
          // createLocalSession appends to `order`, so order[0] is the newest).
          const st = useSessionManager.getState();
          if (st.order.length > 0) {
            st.setActive(st.order[0]);
            return;
          }
          // History existed but every load failed (daemon died mid-restore) —
          // fall through to a fresh local session rather than a blank rail.
        }
      } catch {
        // Daemon unreachable at startup — fall through to a fresh session.
      }
      if (!cancelled && !useSessionManager.getState().activeId) {
        useSessionManager.getState().createLocalSession();
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  // Ensure the daemon is running before the first health check. In the browser
  // this is a no-op (user starts the daemon manually). In Tauri, the host
  // spawns/reuses the daemon. Failures are non-fatal — the health poll below
  // will keep retrying and surface a "disconnected" status, but we also toast
  // the specific spawn error so the user knows why (e.g. binary not found).
  useEffect(() => {
    getPlatform()
      .ensureDaemon?.()
      .catch((e) => {
        console.warn("ensureDaemon failed (non-fatal):", e);
        // Only toast on desktop — in the browser ensureDaemon is a no-op and
        // can't fail. The message helps the user diagnose spawn failures.
        if (getPlatform().name === "desktop") {
          toast.error(`Failed to start daemon: ${String(e).slice(0, 120)}`);
        }
      });
  }, []);

  // sessionManager → uiStore.openTabs 单向同步（激活补开 tab、删除剪 tab）。
  useEffect(() => startUiSync(), []);

  // Warn before unloading the page while any session is mid-turn. The run
  // itself lives on the daemon (closing the tab will not kill it), but leaving
  // disconnects the live SSE observer - a heads-up avoids surprise.
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

  // Thin-client heartbeat: the singleton WebSocket push channel IS the
  // heartbeat — the daemon counts an authenticated ws connection as an
  // active client (idle-shutdown deferral) exactly like the old SSE
  // heartbeat, and the channel's reconnect loop replaces EventSource's
  // auto-retry. Connect is idempotent; the channel lives until the tab
  // closes. The daemon shuts down after 5 minutes with no connected client
  // and no authenticated API activity, so page refreshes and brief
  // disconnects are tolerated.
  useEffect(() => {
    wsChannel.connect();
  }, []);

  // Subscribe to the trace SSE for pushed subagent permission prompts
  // (design D2.1: replaces 500ms polling of /tools/pending-permissions).
  usePermissionTrace(client);

  // Keep the StatusBar's permission-mode label aligned with daemon truth:
  // per-session on tab switch, re-fetch on daemon restart recovery, and live
  // adoption of `mode_changed` broadcasts (TUI / other tabs).
  usePermissionModeSync(client);

  // Observe daemon-initiated runs (subagent synthesis continuations) live —
  // otherwise web only renders turns it started itself.
  useContinuationObserver(client);

  // A `/model` switch (from ANY client — web modal, TUI, CLI) can change the
  // context window along with the name, so re-read /config on the broadcast
  // instead of trusting the first-load snapshot forever.
  useEffect(() => {
    return wsChannel.subscribeGlobal((ev) => {
      if (ev.kind !== "model_changed") return;
      client
        .getConfig()
        .then((cfg) => {
          useSessionManager.getState().setModelName(cfg.model);
          useSessionManager.getState().setContextWindow(cfg.context_window);
        })
        .catch(() => {
          // Non-fatal: the health poll's one-shot load remains in effect.
        });
    });
  }, [client]);

  // Poll the lightweight agent directory into the per-session store so the
  // Subagents panel always has the whole-session tree (not just SSE-active
  // nodes). Pauses while the tab is hidden; caches per session on switch.
  useSubagentDirectory(client);

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
          setContextWindow(cfg.context_window);
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
    <ConfirmProvider>
      {/* h-dvh（动态视口高度）：手机地址栏伸缩时 100vh 会大于可视区域，
          导致文档级滚动条；overflow-hidden + overscroll-none 兜底禁止
          页面级双轴滚动 —— 滚动只发生在内部容器（消息列表、代码块）。 */}
      <div className="flex h-dvh flex-col overflow-hidden overscroll-none bg-background text-foreground">
        <AppTopbar />
        <div className="flex min-h-0 flex-1">
          <LeftSidebar client={client} />
          <SessionStoreContext.Provider value={activeStore}>
            <div className="flex min-w-0 flex-1 flex-col">
              <SessionTabBar />
              <main className="min-h-0 flex-1 overflow-y-auto">
                {subagentTabMeta ? (
                  <SubagentDetailPanel
                    key={subagentTabMeta.nodeId}
                    client={client}
                    nodeId={subagentTabMeta.nodeId}
                    rootSessionId={subagentTabMeta.rootSessionId}
                    label={subagentTabMeta.label}
                  />
                ) : previewTabMeta ? (
                  // key = absPath so switching preview files remounts the
                  // panel (fetch/blob-URL state resets per file).
                  <PreviewPanel
                    key={previewTabMeta.absPath}
                    client={client}
                    meta={previewTabMeta}
                  />
                ) : diffTabMeta ? (
                  <DiffView key={diffTabMeta.absPath} client={client} meta={diffTabMeta} />
                ) : (
                  <ChatView />
                )}
              </main>
              {/* 权限/问答卡片内嵌在聊天区与输入框之间（非 fixed 悬浮）：
                  出现时 flex 列让聊天区收缩让位，底部消息可滚动查看，
                  不遮挡任何内容。仅活跃会话有待处理项时占位。 */}
              <PermissionModal client={client} />
              <QuestionModal client={client} />
              <Composer
                onSend={(text) => {
                  if (!activeId || !activeStore) return;
                  const state = activeStore.getState();
                  if (state.isRunning) {
                    // A turn is active — queue the message and auto-send it
                    // once the current turn finishes cleanly. Mirrors the
                    // TUI pending_inputs / submit_input path.
                    state.enqueueInput(text);
                  } else {
                    void runSessionTurn(client, activeId, text);
                  }
                }}
                onStop={() => {
                  if (activeId) void stopSessionTurn(client, activeId);
                }}
                onCommand={handleCommand}
              />
            </div>
          </SessionStoreContext.Provider>
          <RightRail client={client} />
        </div>
        <StatusBar
          client={client}
          onSwitchModel={() =>
            setOpenCommand({ name: "/model", description: "Switch model profile" })
          }
        />
        {openCommand?.name === "/model" && (
          <CommandModal title="Switch model" onClose={closeCommand}>
            <ModelPanel client={client} />
          </CommandModal>
        )}
        <Toaster theme={theme} position="bottom-right" />
      </div>
    </ConfirmProvider>
  );
}
