import { Check, CircleSlash, X } from "lucide-react";
import type { ToolExecution } from "../../agent/loop";
import { cn } from "../../lib/utils";
import { DiffView } from "./DiffView";
import { hasDiff } from "./diffUtils";

/** Status → accent classes (left color bar + status text/icon color). */
const STATUS_STYLES = {
  ok: { bar: "border-l-success", text: "text-success" },
  error: { bar: "border-l-danger", text: "text-danger" },
  denied: { bar: "border-l-warning", text: "text-warning" },
} as const;

/**
 * Collapsed-by-default tool call (Codex-style: one summary line, click to
 * expand). The summary shows the tool name + a one-field arg preview + status,
 * so a run of tool calls reads as a compact log rather than a wall of boxes.
 */
export function ToolCallCard({ exec }: { exec: ToolExecution }) {
  const { call, response, permissionDecision } = exec;
  const ok = response.success;
  const denied = permissionDecision === "deny";
  const toolName = call.function.name;
  const showDiff = hasDiff(toolName, response);
  const summary = argSummary(call.function.arguments);

  let argsPreview = call.function.arguments;
  try {
    argsPreview = JSON.stringify(JSON.parse(call.function.arguments), null, 2);
  } catch {
    // leave raw
  }

  const status = denied ? "denied" : ok ? "ok" : "error";
  const styles = STATUS_STYLES[status];
  const statusIcon = denied ? (
    <CircleSlash size={12} />
  ) : ok ? (
    <Check size={12} />
  ) : (
    <X size={12} />
  );

  return (
    <details
      className={cn(
        "overflow-hidden rounded-md border border-border border-l-[3px] text-[12px] open:bg-card",
        styles.bar,
      )}
      data-status={status}
    >
      <summary className="flex cursor-pointer items-center gap-2 px-2.5 py-1.5 text-[12px] select-none">
        <span className={cn("inline-flex shrink-0", styles.text)}>{statusIcon}</span>
        <span className="shrink-0 font-mono font-medium text-foreground">{toolName}</span>
        {summary && (
          <span className="min-w-0 truncate font-mono text-[11px] text-muted-foreground">
            {summary}
          </span>
        )}
        <span className={cn("ml-auto shrink-0 text-[10px] tracking-wide uppercase", styles.text)}>
          {status}
        </span>
      </summary>
      <div className="border-t border-border px-2.5 pb-2">
        {argsPreview && !showDiff && (
          <pre className="mt-1.5 max-h-56 overflow-y-auto font-mono text-[11px] whitespace-pre-wrap text-muted-foreground">
            {argsPreview}
          </pre>
        )}
        {response.error && (
          <pre className="mt-1.5 max-h-56 overflow-y-auto font-mono text-[11px] whitespace-pre-wrap text-danger">
            {response.error}
          </pre>
        )}
        {showDiff ? (
          <DiffView toolName={toolName} response={response} />
        ) : (
          response.content && (
            <pre className="mt-1.5 max-h-56 overflow-y-auto font-mono text-[11px] whitespace-pre-wrap text-muted-foreground">
              {truncate(response.content, 2000)}
            </pre>
          )
        )}
      </div>
    </details>
  );
}

/** Extract a one-line arg preview (path / command / q / pattern) for the summary. */
function argSummary(argsJson: string): string {
  try {
    const args = JSON.parse(argsJson) as Record<string, unknown>;
    for (const key of ["path", "command", "q", "pattern", "query", "url"]) {
      const v = args[key];
      if (typeof v === "string") return shorten(v, 60);
    }
    return "";
  } catch {
    return "";
  }
}

function shorten(s: string, max: number): string {
  return s.length <= max ? s : `${s.slice(0, max)}…`;
}

function truncate(s: string, max: number): string {
  if (s.length <= max) return s;
  return `${s.slice(0, max)}\n… (${s.length - max} more chars truncated)`;
}
