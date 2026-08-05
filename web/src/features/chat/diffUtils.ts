/**
 * Diff extraction for file-mutating tool results.
 *
 * Both `file_edit` and `apply_patch` return before/after content in their
 * response metadata (not as a diff string), so we compute the line-level diff
 * client-side with the `diff` package (see DiffView). This avoids any backend
 * change and keeps the rendering faithful to what the tool actually changed.
 *
 * Metadata shapes (see src/tools/filesystem/{file_edit,apply_patch}.rs):
 *   file_edit:  metadata.old_content, metadata.new_content
 *   apply_patch: metadata.diffs = { "<path>": { old_content, new_content }, ... }
 */
import type { ExecuteToolResponse } from "../../api/types";

/** One file's before/after, with a computed line diff. */
export interface FileDiff {
  path?: string;
  oldContent: string;
  newContent: string;
}

const FILE_EDIT_TOOLS = new Set(["file_edit", "file_write", "file_read"]);
const PATCH_TOOL = "apply_patch";

/** Extract before/after pairs from a tool response's metadata. */
export function extractDiffs(
  toolName: string,
  metadata: ExecuteToolResponse["metadata"],
): FileDiff[] {
  if (!metadata) return [];

  if (toolName === PATCH_TOOL) {
    const diffs = metadata.diffs as
      Record<string, { old_content: string; new_content: string }> | undefined;
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
