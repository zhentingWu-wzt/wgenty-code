import { selectPendingApprovalCount, useSessionManager } from "../state/sessionManager";

/** 底部状态栏：daemon 连接 · 运行状态 · 待审批数 · 模型。 */
export function StatusBar() {
  const connection = useSessionManager((s) => s.connection);
  const modelName = useSessionManager((s) => s.modelName);
  const pendingApprovals = useSessionManager(selectPendingApprovalCount);
  const activeStatus = useSessionManager((s) =>
    s.activeId ? s.entries[s.activeId]?.status : undefined,
  );
  const isRunning = activeStatus === "running" || activeStatus === "awaiting_approval";

  const statusText =
    connection === "connected" ? "online" : connection === "disconnected" ? "offline" : "connecting";

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
        <span className="rounded-sm bg-warning/20 px-1 text-warning">{pendingApprovals} approval</span>
      )}
      <div className="flex-1" />
      {modelName && <span title="active model">{modelName}</span>}
    </footer>
  );
}
