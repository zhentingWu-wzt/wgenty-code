import { useSessionManager } from "../state/sessionManager";

const STATUS_LABEL: Record<string, string> = {
  running: "Running",
  awaiting_approval: "Needs approval",
  idle: "Idle",
  error: "Error",
};

/** Center-pane header: active session name + live status pill. */
export function SessionHeader() {
  const entry = useSessionManager((s) => (s.activeId ? s.entries[s.activeId] : null));
  if (!entry) return null;
  return (
    <div className="session-header">
      <span className="session-header-name">{entry.name}</span>
      <span className={`session-header-status session-status-${entry.status}`}>
        {STATUS_LABEL[entry.status]}
      </span>
    </div>
  );
}
