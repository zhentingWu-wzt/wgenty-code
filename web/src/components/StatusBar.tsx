import { useChatStore } from "../state/chatStore";

/**
 * Codex-style top control bar. Left: connection + branch context. Right:
 * model + run state. Dense, single row, no decoration — the "command center"
 * strip at the top of the window.
 */
export function StatusBar() {
  const connection = useChatStore((s) => s.connection);
  const modelName = useChatStore((s) => s.modelName);
  const isRunning = useChatStore((s) => s.isRunning);

  const statusText =
    connection === "connected"
      ? "online"
      : connection === "disconnected"
        ? "offline"
        : "connecting";

  return (
    <header className="topbar">
      <div className="topbar-left">
        <span className={`topbar-dot topbar-dot-${connection}`} title={statusText} />
        <span className="topbar-status">{statusText}</span>
      </div>
      <div className="topbar-right">
        {isRunning && (
          <span className="topbar-running">
            <span className="topbar-spinner" /> working
          </span>
        )}
        {modelName && (
          <>
            {isRunning && <span className="topbar-sep" />}
            <span className="topbar-model" title="active model">
              {modelName}
            </span>
          </>
        )}
      </div>
    </header>
  );
}
