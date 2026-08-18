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
  // 底部停靠卡片而非全屏遮罩弹窗：审批需要的上下文（正在执行的工具、
  // 对话上文）保持可见可滚动，卡片停靠在输入框上方（bottom-8 避开 h-6
  // 状态栏），超高内容在卡片内部滚动。
  return (
    <div
      role="dialog"
      aria-label="Permission required"
      className="fixed bottom-8 left-1/2 z-50 flex max-h-[60dvh] w-[480px] max-w-[calc(100%-16px)] -translate-x-1/2 flex-col overflow-y-auto rounded-lg border border-warning/50 bg-popover p-4 shadow-2xl"
    >
      <div className="mb-2 flex items-center gap-1.5 text-[15px] font-semibold text-warning">
        <span className="h-2 w-2 shrink-0 animate-pulse rounded-full bg-warning" />
        Permission required
      </div>
      <div className="mb-1 font-mono text-primary [overflow-wrap:anywhere]">{tool}</div>
      <p className="mb-1.5 leading-relaxed">{reason}</p>
      <p className="mb-3">
        <code className="rounded-sm bg-background px-1.5 py-0.5 font-mono text-[12px] text-muted-foreground [overflow-wrap:anywhere]">
          {rule}
        </code>
      </p>
      <p className="mb-3 text-[12px] text-muted-foreground">
        Approvals are global — they apply to all sessions.
      </p>
      <div className="flex flex-wrap justify-end gap-2">
        <Button onClick={() => onChoose("allowOnce")}>Allow once</Button>
        <Button variant="outline" onClick={() => onChoose("alwaysAllow")}>
          Always allow
        </Button>
        <Button variant="destructive" onClick={() => onChoose("deny")}>
          Deny
        </Button>
      </div>
    </div>
  );
}
