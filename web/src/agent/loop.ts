/**
 * Client-side agent loop — minimal port of `src/agent/runtime/loop_.rs`.
 *
 * WHY THIS EXISTS: the daemon's `/api/v1/chat/stream` is a pure passthrough
 * proxy. It forwards the upstream LLM's SSE but does NOT execute tools. The
 * client must: stream a round → if the model emitted tool_calls, execute each
 * via `/api/v1/tools/execute` → append tool results → stream the next round →
 * repeat until a round produces no tool calls. This is exactly what the TUI
 * does in `loop_.rs` (`run_agent_loop_inner`).
 *
 * DELIBERATELY OMITTED (out of MVP scope, all client-side optimizations from
 * the TUI): context compaction, stuck-detector, parallel `task` batches, token
 * calibration, Guardian local short-circuit (we rely on the daemon's
 * `permission_required` signal instead).
 */
import type { DaemonClient } from "../api/client";
import { StreamProcessor, type StreamEvent } from "../api/sseParser";
import type { ChatMessage, ExecuteToolResponse, PermissionDecision, ToolCall } from "../api/types";

/** Default round cap. Mirrors the TUI's `RuntimeConfig.max_rounds` spirit. */
const MAX_ROUNDS = 30;

/** Error thrown when a turn ends with neither content nor tool calls. */
export class EmptyResponseError extends Error {
  constructor() {
    super("the model returned an empty response with no tool calls");
    this.name = "EmptyResponseError";
  }
}

export class MaxRoundsExceededError extends Error {
  constructor(rounds: number) {
    super(`agent loop exceeded ${rounds} rounds`);
    this.name = "MaxRoundsExceededError";
  }
}

export interface ToolExecution {
  /** The tool call the model made. */
  call: ToolCall;
  /** Raw daemon response (may carry `permission_required` if blocked). */
  response: ExecuteToolResponse;
  /** The final decision if a permission prompt was shown. */
  permissionDecision?: PermissionDecision;
}

export interface RoundEvent {
  /** 1-based round index within this turn. */
  round: number;
}

export interface AgentLoopCallbacks {
  /** Fired for every SSE event (content delta, tool-call delta, done, …). */
  onStreamEvent: (round: number, ev: StreamEvent) => void;
  /** Fired when a tool call starts and again when it resolves. */
  onToolExecution: (exec: ToolExecution) => void;
  /** Ask the user to approve a blocked tool. Resolves with their decision. */
  onPermissionRequired: (
    info: NonNullable<ExecuteToolResponse["permission_required"]>,
  ) => Promise<PermissionDecision>;
}

export interface RunAgentLoopArgs {
  client: DaemonClient;
  /** Mutated in place: each round pushes assistant + tool messages here. */
  messages: ChatMessage[];
  /** Stable session id forwarded to `/tools/execute`. */
  sessionId: string;
  callbacks: AgentLoopCallbacks;
  maxRounds?: number;
  /** Optional signal to abort the loop between rounds (e.g. user cancel). */
  signal?: AbortSignal;
}

/** Read a `ReadableStream<Uint8Array>` chunk by chunk (async iterable). */
async function* readChunks(stream: ReadableStream<Uint8Array>): AsyncIterable<Uint8Array> {
  const reader = stream.getReader();
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) return;
      if (value) yield value;
    }
  } finally {
    reader.releaseLock();
  }
}

/**
 * Drive one full user turn to completion (multiple internal rounds possible).
 * Returns the assistant's final text content. Throws on stream errors, empty
 * responses, and max-rounds exhaustion.
 *
 * Ported from `run_agent_loop_inner` (src/agent/runtime/loop_.rs:191-959).
 */
export async function runAgentLoop(args: RunAgentLoopArgs): Promise<string> {
  const { client, messages, sessionId, callbacks } = args;
  const maxRounds = args.maxRounds ?? MAX_ROUNDS;

  for (let round = 1; round <= maxRounds; round++) {
    if (args.signal?.aborted) throw new Error("aborted");

    // ── 1. Stream one round ──────────────────────────────────────────────────
    const processor = new StreamProcessor();
    const { body } = await client.chatStream(messages, {}, args.signal);
    try {
      for await (const chunk of readChunks(body)) {
        for (const ev of processor.feedBytes(chunk)) {
          callbacks.onStreamEvent(round, ev);
          if (ev.type === "streamError") {
            throw new Error(`stream error: ${ev.message}`);
          }
        }
      }
    } catch (err) {
      // An AbortError (user hit stop mid-stream) is expected — surface it as a
      // clean "aborted" so the caller can finalize without showing an error.
      if (err instanceof Error && err.name === "AbortError") throw new Error("aborted");
      throw err;
    }
    const result = processor.finish();

    // ── 2. Guard: empty turn with no tool calls (loop_.rs:904-909) ────────────
    if (!result.hasToolCalls && result.content.length === 0) {
      throw new EmptyResponseError();
    }

    // ── 3. Push the assistant message into history (loop_.rs:829-831) ────────
    // Preserve tool_calls on the assistant message so the next round's request
    // stays well-formed (assistant tool_call ↔ tool result pairing).
    messages.push({
      role: "assistant",
      content: result.content || undefined,
      ...(result.toolCalls.length > 0 ? { tool_calls: result.toolCalls } : {}),
    });

    // ── 4. No tool calls → turn is complete (loop_.rs:951-959) ───────────────
    if (!result.hasToolCalls || result.toolCalls.length === 0) {
      return result.content;
    }

    // ── 5. Execute every tool call, then loop back for another round ─────────
    for (const call of result.toolCalls) {
      await executeOneTool({ client, call, sessionId, callbacks });
    }
    // loop continues → next chat/stream round carries the tool results.
  }

  throw new MaxRoundsExceededError(maxRounds);
}

/**
 * Execute a single tool call, handling the `permission_required` approval dance
 * that the daemon signals inline (a successful response carrying
 * `permission_required`, NOT an error status).
 *
 * Ported from `DaemonToolPort::execute` (src/tui/agent/adapters.rs:240-437):
 *   - AllowOnce:  approve → execute → unapprove (revoke after one use)
 *   - AlwaysAllow: approve → execute (no revoke)
 *   - Deny:       push a denial as the tool result (no execute)
 */
async function executeOneTool(args: {
  client: DaemonClient;
  call: ToolCall;
  sessionId: string;
  callbacks: AgentLoopCallbacks;
}): Promise<void> {
  const { client, call, sessionId, callbacks } = args;

  // Parse the JSON-encoded argument string into an object for the daemon.
  let parsedArgs: Record<string, unknown> = {};
  try {
    parsedArgs = call.function.arguments ? JSON.parse(call.function.arguments) : {};
  } catch {
    // Malformed args — surface as a failed tool result so the model can react.
    callbacks.onToolExecution({
      call,
      response: {
        success: false,
        error: `failed to parse tool arguments: ${call.function.arguments}`,
      },
    });
    return;
  }

  const initial = await client.executeTool({
    tool_name: call.function.name,
    arguments: parsedArgs,
    session_id: sessionId,
  });

  // No permission needed — done.
  if (!initial.permission_required) {
    callbacks.onToolExecution({ call, response: initial });
    return;
  }

  // Permission required → ask the user.
  const decision = await callbacks.onPermissionRequired(initial.permission_required);
  if (decision === "deny") {
    callbacks.onToolExecution({
      call,
      response: initial,
      permissionDecision: "deny",
    });
    return;
  }

  // Approve, then re-execute. For `allowOnce`, revoke the rule right after.
  await client.approveTool(initial.permission_required.session_rule);
  try {
    const retried = await client.executeTool({
      tool_name: call.function.name,
      arguments: parsedArgs,
      session_id: sessionId,
    });
    callbacks.onToolExecution({
      call,
      response: retried,
      permissionDecision: decision,
    });
  } finally {
    if (decision === "allowOnce") {
      // Best-effort revoke; a failure here shouldn't mask the tool result.
      await client.unapproveTool(initial.permission_required.session_rule).catch(() => {});
    }
  }
}
