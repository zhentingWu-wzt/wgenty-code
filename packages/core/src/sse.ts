import type { StreamChunk } from "./types.ts";

/**
 * Parse a single SSE `data:` line into a StreamChunk.
 * Returns null for `data: [DONE]` or unparseable lines.
 */
export function parseSseLine(line: string): StreamChunk | null {
  const trimmed = line.trim();
  if (!trimmed.startsWith("data: ")) return null;

  const data = trimmed.slice(6); // remove "data: " prefix
  if (data === "[DONE]") return null;

  try {
    return JSON.parse(data) as StreamChunk;
  } catch {
    return null;
  }
}
