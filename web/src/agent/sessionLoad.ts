/**
 * Lossless conversion of a saved session's wire messages into display
 * messages. Lives outside SessionList.tsx so the component file only exports
 * components (react-refresh) and the mapping stays unit-testable in isolation.
 */
import type { ExecuteToolResponse, SessionMessage } from "../api/types";
import type { ToolExecution } from "./loop";
import type { DisplayMessage } from "../state/sessionStore";

// Module-level counter for loaded-history message ids (same pattern as
// sessionStore's genId; avoids impure Math.random in component scope).
let loadedCounter = 0;
const loadedId = (): string => `loaded-${++loadedCounter}`;

/** Sentinel for a tool call whose result never arrived in the saved history. */
const MISSING_RESULT = "tool result missing from saved history";

/**
 * Best-effort reconstruction of an `ExecuteToolResponse` from a stored tool
 * message's content. Autosave writes `JSON.stringify(exec.response)` (see
 * sessionRunner.toWireMessages), so JSON.parse usually recovers the original
 * shape; anything else is wrapped as plain-text content. Never throws.
 */
function parseToolResponse(raw: unknown): ExecuteToolResponse {
  if (typeof raw === "string" && raw.length > 0) {
    try {
      const parsed = JSON.parse(raw) as unknown;
      if (parsed !== null && typeof parsed === "object") return parsed as ExecuteToolResponse;
      // A JSON scalar (e.g. `"ok"`, `42`) — keep it as text content.
      return { success: true, content: raw };
    } catch {
      return { success: true, content: raw };
    }
  }
  // Missing/empty result — mark it so a re-save doesn't fabricate a success.
  return { success: false, error: MISSING_RESULT };
}

/**
 * Convert a saved session's wire messages into display messages WITHOUT
 * dropping tool-call structure (the previous mapping collapsed every non-user
 * message to an empty assistant bubble, and the next autosave then overwrote
 * the daemon's copy with that stripped history — silent data loss).
 *
 * Pairing: an assistant message's `tool_calls[i]` is matched to the following
 * tool message with the same `tool_call_id` (falling back to the next
 * unmatched call of the same assistant message). Tool messages with no
 * matching call stay as standalone `role: "tool"` display messages so they
 * still round-trip through `toWireMessages`.
 */
export function sessionMessagesToDisplay(messages: SessionMessage[]): DisplayMessage[] {
  const out: DisplayMessage[] = [];
  // Tool calls of the most recent assistant message, awaiting their results.
  let pending: ToolExecution[] = [];

  for (const msg of messages) {
    if (msg.role === "user") {
      pending = [];
      out.push({ id: loadedId(), role: "user", content: msg.content ?? "" });
    } else if (msg.role === "assistant") {
      pending = (msg.tool_calls ?? []).map((call) => ({
        call,
        // Placeholder until the matching tool message fills in the result.
        response: { success: false, error: MISSING_RESULT },
      }));
      out.push({
        id: loadedId(),
        role: "assistant",
        content: typeof msg.content === "string" ? msg.content : "",
        ...(pending.length > 0 ? { toolExecs: pending } : {}),
      });
    } else if (msg.role === "tool") {
      const response = parseToolResponse(msg.content);
      const exec =
        pending.find(
          (e) => e.call.id === msg.tool_call_id && e.response.error === MISSING_RESULT,
        ) ?? pending.find((e) => e.response.error === MISSING_RESULT);
      if (exec) {
        exec.response = response;
      } else {
        // No assistant tool_call to fold into — keep it as a tool message
        // rather than an empty assistant bubble.
        out.push({
          id: loadedId(),
          role: "tool",
          content: typeof msg.content === "string" ? msg.content : "",
          toolCallId: msg.tool_call_id,
        });
      }
    }
    // system/other roles: not displayable, skip.
  }
  return out;
}
