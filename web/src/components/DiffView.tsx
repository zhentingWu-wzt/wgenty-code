/**
 * Diff view for file-mutating tool results.
 *
 * Renders the before/after pairs extracted by `diffUtils.extractDiffs` as a
 * line-level diff computed client-side with the `diff` package.
 */
import { diffLines } from "diff";
import type { ExecuteToolResponse } from "../api/types";
import { extractDiffs, type FileDiff } from "./diffUtils";

export function DiffView({
  toolName,
  response,
}: {
  toolName: string;
  response: ExecuteToolResponse;
}) {
  const diffs = extractDiffs(toolName, response.metadata);
  if (diffs.length === 0) return null;

  return (
    <div className="diff-list">
      {diffs.map((d, i) => (
        <SingleFileDiff key={i} diff={d} />
      ))}
    </div>
  );
}

function SingleFileDiff({ diff }: { diff: FileDiff }) {
  // `diffLines` returns parts with .added/.removed/.value; unchanged parts are
  // context. We render every line with its line classification.
  const parts = diffLines(diff.oldContent, diff.newContent);
  const lines: Array<{ type: "add" | "del" | "ctx"; text: string }> = [];
  for (const part of parts) {
    const type = part.added ? "add" : part.removed ? "del" : "ctx";
    // Each part.value may span multiple lines; drop the trailing newline so
    // we render one row per line.
    for (const text of part.value.replace(/\n$/, "").split("\n")) {
      lines.push({ type, text });
    }
  }

  return (
    <div className="diff-file">
      {diff.path && <div className="diff-path">{diff.path}</div>}
      <pre className="diff-body">
        {lines.map((line, i) => (
          <div key={i} className={`diff-line diff-${line.type}`}>
            <span className="diff-gutter">
              {line.type === "add" ? "+" : line.type === "del" ? "-" : " "}
            </span>
            <span className="diff-text">{line.text}</span>
          </div>
        ))}
      </pre>
    </div>
  );
}
