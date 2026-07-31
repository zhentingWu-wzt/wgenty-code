/**
 * Diff view for file-mutating tool results.
 *
 * Both `file_edit` and `apply_patch` return before/after content in their
 * response metadata (not as a diff string), so we compute the line-level diff
 * client-side with the `diff` package. This avoids any backend change and
 * keeps the rendering faithful to what the tool actually changed.
 *
 * Metadata shapes (see src/tools/filesystem/{file_edit,apply_patch}.rs):
 *   file_edit:  metadata.old_content, metadata.new_content
 *   apply_patch: metadata.diffs = { "<path>": { old_content, new_content }, ... }
 */
import { diffLines } from "diff";
import type { ExecuteToolResponse } from "../api/types";

/** One file's before/after, with a computed line diff. */
interface FileDiff {
  path?: string;
  oldContent: string;
  newContent: string;
}

const FILE_EDIT_TOOLS = new Set(["file_edit", "file_write", "file_read"]);
const PATCH_TOOL = "apply_patch";

/** Extract before/after pairs from a tool response's metadata. */
function extractDiffs(
  toolName: string,
  metadata: ExecuteToolResponse["metadata"],
): FileDiff[] {
  if (!metadata) return [];

  if (toolName === PATCH_TOOL) {
    const diffs = metadata.diffs as Record<string, { old_content: string; new_content: string }> | undefined;
    if (!diffs) return [];
    return Object.entries(diffs).map(([path, v]) => ({
      path,
      oldContent: v.old_content,
      newContent: v.new_content,
    }));
  }

  if (FILE_EDIT_TOOLS.has(toolName)) {
    const oldContent = metadata.old_content;
    const newContent = metadata.new_content;
    if (typeof oldContent !== "string" || typeof newContent !== "string") return [];
    return [{ oldContent, newContent }];
  }

  return [];
}

/** Whether a tool result is diff-renderable (used by ToolCallCard to switch). */
export function hasDiff(toolName: string, response: ExecuteToolResponse): boolean {
  return extractDiffs(toolName, response.metadata).length > 0;
}

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
            <span className="diff-gutter">{line.type === "add" ? "+" : line.type === "del" ? "-" : " "}</span>
            <span className="diff-text">{line.text}</span>
          </div>
        ))}
      </pre>
    </div>
  );
}
