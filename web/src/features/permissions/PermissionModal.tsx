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
        await client.resolveSubagentPermission(
          subagent.request_id,
          approved,
          always,
          subagent.session_rule,
        );
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
  // 布局内嵌横幅（App.tsx 中置于聊天区与输入框之间，非 fixed 悬浮）：
  // 出现时聊天区收缩让位而非被遮挡，底部消息保持可滚动可读。
  // 超高内容（超长 reason/rule）在卡片内部滚动，max-h 上限防止过度压缩聊天。
  return (
    <div
      role="dialog"
      aria-label="Permission required"
      className="flex max-h-[45dvh] shrink-0 flex-col overflow-y-auto border-t border-warning/40 bg-popover px-3 py-2.5"
    >
      <div className="mb-1.5 flex items-center gap-1.5 text-[13px] font-semibold text-warning">
        <span className="h-2 w-2 shrink-0 animate-pulse rounded-full bg-warning" />
        Permission required
      </div>
      <div className="mb-1 font-mono text-primary [overflow-wrap:anywhere]">{tool}</div>
      <p className="mb-1.5 leading-relaxed">{reason}</p>
      <p className="mb-2">
        <code className="rounded-sm bg-background px-1.5 py-0.5 font-mono text-[12px] text-muted-foreground [overflow-wrap:anywhere]">
          {rule}
        </code>
      </p>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <span className="text-[11px] text-muted-foreground">
          Approvals are global — they apply to all sessions.
        </span>
        <div className="flex flex-wrap gap-2">
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
