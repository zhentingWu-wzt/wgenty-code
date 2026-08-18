import { useState, useSyncExternalStore } from "react";
import { selectPendingApprovalCount, useSessionManager } from "../state/sessionManager";
import type { DaemonClient } from "../api/client";
import type { PermissionMode } from "../api/types";
import { useWorkspaceRoot } from "../features/files/useWorkspaceRoot";

const MODE_LABELS: Record<PermissionMode, string> = {
  normal: "normal",
  accept_edits: "accept edits",
  yolo: "yolo",
};
const MODE_ORDER: PermissionMode[] = ["normal", "accept_edits", "yolo"];

interface StatusBarProps {
  client: DaemonClient;
  /** Open the /model switcher modal. */
  onSwitchModel: () => void;
}

/** 底部状态栏：daemon 连接 · 运行状态 · 待审批数 · 权限模式 · 模型。 */
export function StatusBar({ client, onSwitchModel }: StatusBarProps) {
  const connection = useSessionManager((s) => s.connection);
  const modelName = useSessionManager((s) => s.modelName);
  const permissionMode = useSessionManager((s) => s.permissionMode);
  const setPermissionMode = useSessionManager((s) => s.setPermissionMode);
  const contextWindow = useSessionManager((s) => s.contextWindow);
  const activeStore = useSessionManager((s) =>
    s.activeId ? (s.entries[s.activeId]?.store ?? null) : null,
  );
  const pendingApprovals = useSessionManager(selectPendingApprovalCount);
  const activeStatus = useSessionManager((s) =>
    s.activeId ? s.entries[s.activeId]?.status : undefined,
  );
  // Workspace root of the ACTIVE session (worktree, else project path, else
  // the main project) — shows which checkout a turn will land in.
  const { root: workRoot } = useWorkspaceRoot(client);
  const isRunning = activeStatus === "running" || activeStatus === "awaiting_approval";
  const [modeOpen, setModeOpen] = useState(false);

  // Context-window occupancy of the active session, updated LIVE by
  // usage_update events (after every LLM call) and re-synced by the turn-end
  // turn_context snapshot. The per-session store lives outside React context
  // here (StatusBar sits above the provider), so subscribe directly — the
  // primitive snapshot re-renders only on real usage changes, and switching
  // sessions re-subscribes via the new store's subscribe identity.
  const contextTokens = useSyncExternalStore(
    activeStore?.subscribe ?? noopSubscribe,
    () => activeStore?.getState().contextTokens ?? null,
  );

  const statusText =
    connection === "connected"
      ? "online"
      : connection === "disconnected"
        ? "offline"
        : "connecting";

  const chooseMode = async (m: PermissionMode) => {
    setModeOpen(false);
    // 按活跃会话路由：已落地 daemon 的用 daemonId，否则用本地 id（daemon
    // 对未知 id 回退 main working root，等同改动前的 "default"）。
    const sm = useSessionManager.getState();
    const sid = sm.activeId ? (sm.entries[sm.activeId]?.daemonId ?? sm.activeId) : null;
    if (!sid) return;
    try {
      const res = await client.setPermissionMode(sid, m);
      setPermissionMode(res.mode);
    } catch {
      // Non-fatal: keep the previous mode.
    }
  };

  return (
    <footer className="flex h-6 shrink-0 items-center gap-3 border-t border-border bg-background px-3 text-[11px] text-muted-foreground">
      <span className="flex items-center gap-1.5">
        <span
          className={
            connection === "connected"
              ? "h-1.5 w-1.5 rounded-full bg-success"
              : connection === "disconnected"
                ? "h-1.5 w-1.5 rounded-full bg-danger"
                : "h-1.5 w-1.5 rounded-full bg-warning"
          }
        />
        {statusText}
      </span>
      {isRunning && <span className="text-warning">working</span>}
      {pendingApprovals > 0 && (
        <span className="rounded-sm bg-warning/20 px-1 text-warning">
          {pendingApprovals} approval{pendingApprovals > 1 ? "s" : ""}
        </span>
      )}
      {workRoot && (
        // Full worktree paths overflow the narrow phone status bar (<md);
        // desktop keeps the full path with a hover tooltip.
        <span className="shrink-0 font-mono max-md:hidden" title={workRoot}>
          {workRoot}
        </span>
      )}
      <div className="flex-1" />
      {/* Context-usage bar — a real CSS track/fill bar (▓/░ shade glyphs read
          as a solid block in web fonts, hiding the fill level). Fill color
          mirrors the TUI context_bar: green < 50%, yellow 50–80%, red ≥ 80%.
          Hidden until the first turn reports usage or the window is unknown. */}
      {contextWindow !== null && contextWindow > 0 && contextTokens !== null && (
        <span
          className="flex items-center gap-1.5"
          title={`context ${contextTokens.toLocaleString()} / ${contextWindow.toLocaleString()} tokens`}
        >
          <span className="h-1.5 w-14 overflow-hidden rounded-full bg-muted">
            <span
              className={
                contextTokens / contextWindow >= 0.8
                  ? "block h-full bg-danger"
                  : contextTokens / contextWindow >= 0.5
                    ? "block h-full bg-warning"
                    : "block h-full bg-success"
              }
              style={{
                width: `${Math.round(Math.min(contextTokens / contextWindow, 1) * 100)}%`,
              }}
            />
          </span>
          <span className="tabular-nums">
            {Math.round(Math.min(contextTokens / contextWindow, 1) * 100)}%
          </span>
        </span>
      )}
      {/* Permission mode picker (normal / accept edits / yolo) */}
      <div className="relative">
        <button
          type="button"
          onClick={() => setModeOpen((v) => !v)}
          className="rounded-sm px-1 hover:bg-accent"
          title="Permission mode"
        >
          {permissionMode ? MODE_LABELS[permissionMode] : "-"}
        </button>
        {modeOpen && (
          <>
            <button
              type="button"
              className="fixed inset-0 z-40 cursor-default"
              onClick={() => setModeOpen(false)}
              aria-label="Close mode menu"
            />
            <div className="absolute bottom-full right-0 z-50 mb-1 w-28 rounded-md border border-border bg-popover py-0.5 shadow-lg">
              {MODE_ORDER.map((m) => (
                <button
                  key={m}
                  type="button"
                  onClick={() => chooseMode(m)}
                  className="flex w-full items-center justify-between px-2 py-1 text-left hover:bg-accent"
                >
                  <span>{MODE_LABELS[m]}</span>
                  {permissionMode === m && <span className="text-success">✓</span>}
                </button>
              ))}
            </div>
          </>
        )}
      </div>
      {/* Model switcher (opens the /model modal) */}
      {modelName && (
        <button
          type="button"
          onClick={onSwitchModel}
          className="rounded-sm px-1 hover:bg-accent"
          title="Switch model"
        >
          {modelName}
        </button>
      )}
    </footer>
  );
}

const noopSubscribe = () => () => {};
