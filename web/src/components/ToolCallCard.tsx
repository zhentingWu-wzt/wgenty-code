import type { ToolExecution } from "../agent/loop";
import { DiffView, hasDiff } from "./DiffView";

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
  const statusIcon = denied ? "⊘" : ok ? "✓" : "✕";

  return (
    <details className={`tool-card ${ok ? "tool-ok" : denied ? "tool-denied" : "tool-err"}`}>
      <summary className="tool-summary">
        <span className="tool-status-icon" data-status={status}>
          {statusIcon}
        </span>
        <span className="tool-name">{toolName}</span>
        {summary && <span className="tool-arg-summary">{summary}</span>}
        <span className={`tool-status-text tool-status-${status}`}>{status}</span>
      </summary>
      <div className="tool-body">
        {argsPreview && !showDiff && <pre className="tool-args">{argsPreview}</pre>}
        {response.error && <pre className="tool-output tool-output-err">{response.error}</pre>}
        {showDiff ? (
          <DiffView toolName={toolName} response={response} />
        ) : (
          response.content && <pre className="tool-output">{truncate(response.content, 2000)}</pre>
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
