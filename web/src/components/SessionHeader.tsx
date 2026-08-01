import { CircleAlert, CircleCheck, CircleX, LoaderCircle } from "lucide-react";
import { useSessionManager } from "../state/sessionManager";

const STATUS_LABEL: Record<string, string> = {
  running: "Running",
  awaiting_approval: "Needs approval",
  idle: "Idle",
  error: "Error",
};

const STATUS_ICON: Record<string, typeof CircleCheck> = {
  running: LoaderCircle,
  awaiting_approval: CircleAlert,
  idle: CircleCheck,
  error: CircleX,
};

/** Center-pane header: active session name + live status pill. */
export function SessionHeader() {
  const entry = useSessionManager((s) => (s.activeId ? s.entries[s.activeId] : null));
  if (!entry) return null;
  const StatusIcon = STATUS_ICON[entry.status] ?? CircleCheck;
  return (
    <div className="session-header">
      <span className="session-header-name">{entry.name}</span>
      <span className={`session-header-status session-status-${entry.status}`}>
        <StatusIcon size={11} />
        {STATUS_LABEL[entry.status]}
      </span>
    </div>
  );
}
