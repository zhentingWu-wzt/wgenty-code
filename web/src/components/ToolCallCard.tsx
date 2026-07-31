import type { ToolExecution } from "../agent/loop";
import { DiffView, hasDiff } from "./DiffView";

/** Compact card showing one tool call + its result + any permission decision. */
export function ToolCallCard({ exec }: { exec: ToolExecution }) {
  const { call, response, permissionDecision } = exec;
  const ok = response.success;
  const denied = permissionDecision === "deny";
  const toolName = call.function.name;
  const showDiff = hasDiff(toolName, response);

  let argsPreview = call.function.arguments;
  try {
    argsPreview = JSON.stringify(JSON.parse(call.function.arguments), null, 2);
  } catch {
    // leave raw
  }

  return (
    <div className={`tool-card ${ok ? "tool-ok" : denied ? "tool-denied" : "tool-err"}`}>
      <div className="tool-head">
        <span className="tool-name">{toolName}</span>
        <span className={`tool-badge ${ok ? "ok" : denied ? "denied" : "err"}`}>
          {denied ? "denied" : ok ? "ok" : "error"}
        </span>
      </div>
      {argsPreview && !showDiff && <pre className="tool-args">{argsPreview}</pre>}
      {response.error && <pre className="tool-output tool-output-err">{response.error}</pre>}
      {showDiff ? (
        <DiffView toolName={toolName} response={response} />
      ) : (
        response.content && <pre className="tool-output">{truncate(response.content, 2000)}</pre>
      )}
    </div>
  );
}

function truncate(s: string, max: number): string {
  if (s.length <= max) return s;
  return `${s.slice(0, max)}\n… (${s.length - max} more chars truncated)`;
}
