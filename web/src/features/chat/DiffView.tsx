/**
 * Diff view for file-mutating tool results.
 *
 * Renders the before/after pairs extracted by `diffUtils.extractDiffs` as a
 * line-level diff computed client-side with the `diff` package.
 */
import { diffLines } from "diff";
import type { ExecuteToolResponse } from "../../api/types";
import { cn } from "../../lib/utils";
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
    <div className="mt-1 flex flex-col gap-2">
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
    <div className="overflow-hidden rounded-md border border-border">
      {diff.path && (
        <div className="border-b border-border bg-muted px-2.5 py-1 font-mono text-[12px] text-foreground">
          {diff.path}
        </div>
      )}
      <pre className="max-h-[360px] overflow-auto font-mono text-[12px] leading-normal">
        {lines.map((line, i) => (
          <div
            key={i}
            className={cn(
              "flex whitespace-pre",
              line.type === "add" && "bg-success/10",
              line.type === "del" && "bg-danger/10",
            )}
          >
            <span className="w-6 shrink-0 px-0.5 text-center text-muted-foreground select-none">
              {line.type === "add" ? "+" : line.type === "del" ? "-" : " "}
            </span>
            <span
              className={cn(
                "flex-1",
                line.type === "add" && "text-success",
                line.type === "del" && "text-danger",
                line.type === "ctx" && "text-muted-foreground",
              )}
            >
              {line.text}
            </span>
          </div>
        ))}
      </pre>
    </div>
  );
}
