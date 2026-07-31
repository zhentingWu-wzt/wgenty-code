import { useChatStore } from "../state/chatStore";

/** Top bar: connection status + active model. Mirrors the TUI status line. */
export function StatusBar() {
  const connection = useChatStore((s) => s.connection);
  const modelName = useChatStore((s) => s.modelName);
  const isRunning = useChatStore((s) => s.isRunning);

  const dotClass =
    connection === "connected" ? "dot dot-ok" : connection === "disconnected" ? "dot dot-bad" : "dot";

  return (
    <header className="status-bar">
      <span className={dotClass} />
      <span className="status-label">
        {connection === "connected"
          ? "connected"
          : connection === "disconnected"
            ? "disconnected"
            : "connecting…"}
      </span>
      {modelName && <span className="status-model">{modelName}</span>}
      {isRunning && <span className="status-running">agent working…</span>}
    </header>
  );
}
