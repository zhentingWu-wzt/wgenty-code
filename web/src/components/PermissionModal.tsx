import { useChatStore } from "../state/chatStore";
import type { PermissionDecision } from "../api/types";

/**
 * Modal shown when the daemon signals `permission_required` on a tool call.
 * Resolves the pending promise in the chat store via `resolvePermission`.
 */
export function PermissionModal() {
  const pending = useChatStore((s) => s.pendingPermission);
  const resolve = useChatStore((s) => s.resolvePermission);

  if (!pending) return null;
  const { info } = pending;

  const choose = (d: PermissionDecision) => resolve(d);

  return (
    <div className="modal-backdrop" role="dialog" aria-modal="true">
      <div className="modal">
        <div className="modal-title">Permission required</div>
        <div className="modal-tool">{info.tool_name}</div>
        <p className="modal-reason">{info.reason}</p>
        <p className="modal-rule">
          <code>{info.session_rule}</code>
        </p>
        <div className="modal-actions">
          <button type="button" className="btn btn-primary" onClick={() => choose("allowOnce")}>
            Allow once
          </button>
          <button type="button" className="btn" onClick={() => choose("alwaysAllow")}>
            Always allow
          </button>
          <button type="button" className="btn btn-danger" onClick={() => choose("deny")}>
            Deny
          </button>
        </div>
      </div>
    </div>
  );
}
