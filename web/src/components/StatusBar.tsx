import { useSessionManager } from "../state/sessionManager";
import { useSessionStore } from "../state/sessionContext";

/**
 * Codex-style top control bar. Left: connection + branch context. Right:
 * model + run state. Dense, single row, no decoration — the "command center"
 * strip at the top of the window.
 *
 * connection/modelName are app-level facts (sessionManager); isRunning is the
 * active session's run state (StatusBar renders inside the session Provider).
 */
export function StatusBar() {
  const connection = useSessionManager((s) => s.connection);
  const modelName = useSessionManager((s) => s.modelName);
  const isRunning = useSessionStore((s) => s.isRunning);

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
