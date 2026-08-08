import { DaemonClient } from "../../api/client";
import { useSessionStore } from "../../state/sessionContext";
import type { PermissionDecision } from "../../api/types";
import { Button } from "../../components/ui/button";

interface PermissionModalProps {
  client: DaemonClient;
}

/**
 * Modal for BOTH permission kinds:
 * - Root-tool synchronous (`pendingPermission`): resolved via the session
 *   store's promise, which drives the approve→execute→unapprove dance in the
 *   agent loop.
 * - Subagent async (`pendingSubagent`): pushed via trace SSE; resolved here by
 *   calling `client.resolveSubagentPermission` (POST /tools/resolve-permission).
 *
 * Root takes precedence (it blocks the active turn); subagent shows when no root
 * prompt is pending.
 */
export function PermissionModal({ client }: PermissionModalProps) {
  const rootPending = useSessionStore((s) => s.pendingPermission);
  const resolveRoot = useSessionStore((s) => s.resolvePermission);
  const subagent = useSessionStore((s) => s.pendingSubagent);
  const clearSubagent = useSessionStore((s) => s.clearSubagentPermission);

  // Root permission wins when present.
  if (rootPending) {
    const { info } = rootPending;
    return (
      <ModalShell
        tool={info.tool_name}
        reason={info.reason}
        rule={info.session_rule}
        onChoose={resolveRoot}
      />
    );
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
    <div
      role="dialog"
      aria-modal="true"
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50"
    >
      <div className="w-[480px] max-w-[90%] rounded-lg border border-border bg-popover p-4">
        <div className="mb-2 text-[15px] font-semibold text-warning">Permission required</div>
        <div className="mb-1 font-mono text-primary">{tool}</div>
        <p className="mb-1.5 leading-relaxed">{reason}</p>
        <p className="mb-3">
          <code className="rounded-sm bg-background px-1.5 py-0.5 font-mono text-[12px] text-muted-foreground">
            {rule}
          </code>
        </p>
        <p className="mb-3 text-[12px] text-muted-foreground">
          Approvals are global — they apply to all sessions.
        </p>
        <div className="flex justify-end gap-2">
          <Button onClick={() => onChoose("allowOnce")}>Allow once</Button>
          <Button variant="outline" onClick={() => onChoose("alwaysAllow")}>
            Always allow
          </Button>
          <Button variant="destructive" onClick={() => onChoose("deny")}>
            Deny
          </Button>
        </div>
      </div>
    </div>
  );
}
