import { DaemonClient } from "../api/client";
import { useChatStore } from "../state/chatStore";
import type { PermissionDecision } from "../api/types";

interface PermissionModalProps {
  client: DaemonClient;
}

/**
 * Modal for BOTH permission kinds:
 * - Root-tool synchronous (`pendingPermission`): resolved via the chat store's
 *   promise, which drives the approve→execute→unapprove dance in the agent loop.
 * - Subagent async (`pendingSubagent`): pushed via trace SSE; resolved here by
 *   calling `client.resolveSubagentPermission` (POST /tools/resolve-permission).
 *
 * Root takes precedence (it blocks the active turn); subagent shows when no root
 * prompt is pending.
 */
export function PermissionModal({ client }: PermissionModalProps) {
  const rootPending = useChatStore((s) => s.pendingPermission);
  const resolveRoot = useChatStore((s) => s.resolvePermission);
  const subagent = useChatStore((s) => s.pendingSubagent);
  const clearSubagent = useChatStore((s) => s.clearSubagentPermission);

  // Root permission wins when present.
  if (rootPending) {
    const { info } = rootPending;
    return <ModalShell tool={info.tool_name} reason={info.reason} rule={info.session_rule} onChoose={resolveRoot} />;
  }

  if (subagent) {
    const onChoose = async (d: PermissionDecision) => {
      // Map UI decision → resolve-permission call, then dismiss.
      const approved = d !== "deny";
      const always = d === "alwaysAllow";
      try {
        await client.resolveSubagentPermission(subagent.request_id, approved, always);
      } catch {
        // Best-effort; the bridge times out → deny on its own.
      }
      clearSubagent();
    };
    return (
      <ModalShell
        tool={subagent.tool}
        reason={subagent.policy_reason}
        rule={subagent.session_rule}
        onChoose={onChoose}
      />
    );
  }

  return null;
}

function ModalShell({
  tool,
  reason,
  rule,
  onChoose,
}: {
  tool: string;
  reason: string;
  rule: string;
  onChoose: (d: PermissionDecision) => void;
}) {
  return (
    <div className="modal-backdrop" role="dialog" aria-modal="true">
      <div className="modal">
        <div className="modal-title">Permission required</div>
        <div className="modal-tool">{tool}</div>
        <p className="modal-reason">{reason}</p>
        <p className="modal-rule">
          <code>{rule}</code>
        </p>
        <div className="modal-actions">
          <button type="button" className="btn btn-primary" onClick={() => onChoose("allowOnce")}>
            Allow once
          </button>
          <button type="button" className="btn" onClick={() => onChoose("alwaysAllow")}>
            Always allow
          </button>
          <button type="button" className="btn btn-danger" onClick={() => onChoose("deny")}>
            Deny
          </button>
        </div>
      </div>
    </div>
  );
}
