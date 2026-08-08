/**
 * Shared agent-loop types that survive the server-side migration.
 *
 * The old client-side driver (loop.ts) was removed once the daemon owned the
 * agent loop (POST /sessions/:id/run); only the ToolExecution shape is still
 * referenced by the session store / load / tool-call rendering, so it lives here.
 */
import type { ExecuteToolResponse, PermissionDecision, ToolCall } from "../api/types";

/** A tool invocation paired with its daemon response and optional decision. */
export interface ToolExecution {
  /** The tool call the model made. */
  call: ToolCall;
  /** Raw daemon response (may carry permission_required if blocked). */
  response: ExecuteToolResponse;
  /** The final decision if a permission prompt was shown. */
  permissionDecision?: PermissionDecision;
}
