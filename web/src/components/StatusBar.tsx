import { useState } from "react";
import { selectPendingApprovalCount, useSessionManager } from "../state/sessionManager";
import type { DaemonClient } from "../api/client";
import type { PermissionMode } from "../api/types";

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
  const pendingApprovals = useSessionManager(selectPendingApprovalCount);
  const activeStatus = useSessionManager((s) =>
    s.activeId ? s.entries[s.activeId]?.status : undefined,
  );
  const isRunning = activeStatus === "running" || activeStatus === "awaiting_approval";
  const [modeOpen, setModeOpen] = useState(false);

  const statusText =
    connection === "connected" ? "online" : connection === "disconnected" ? "offline" : "connecting";

  const chooseMode = async (m: PermissionMode) => {
    setModeOpen(false);
    try {
      const res = await client.setPermissionMode(m);
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
      <div className="flex-1" />
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
