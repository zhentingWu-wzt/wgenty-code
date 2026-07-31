/**
 * SSE stream parser — ported 1:1 from `src/agent/core.rs` (`StreamProcessor`).
 *
 * The daemon's `/api/v1/chat/stream` is a pure passthrough proxy: it forwards
 * the upstream LLM's OpenAI-compatible SSE verbatim (stripping the `data: `
 * prefix so it isn't double-wrapped). This class reassembles those fragments
 * into typed events and accumulates the full turn result.
 *
 * Critical behaviors preserved from the Rust source:
 *   - Error detection happens BEFORE chunk parsing (a `{"error":"..."}` payload
 *     is a daemon/stream error, not an OpenAI chunk). (core.rs:72-81)
 *   - Deltas are accumulated BEFORE checking finish_reason, because some
 *     providers ship content + finish_reason in the same final chunk. (core.rs:92-94)
 *   - Tool-call fragments are accumulated by `index`, with `arguments` string-
 *     concatenated across fragments. (core.rs:103-131)
 *
 * This module is pure (no React, no fetch) so it unit-tests trivially.
 */
import type {
  AssembledToolCall,
  StreamChunk,
  StreamToolCall,
} from "./types";

/** Discriminated union of events emitted while consuming the stream. */
export type StreamEvent =
  | { type: "contentDelta"; text: string }
  | { type: "reasoningDelta"; text: string }
  | { type: "toolCallDelta"; index: number; id?: string; name?: string; arguments?: string }
  | { type: "streamDone"; finishReason: string }
  | { type: "streamError"; message: string };

/** Final accumulated state after the stream closes. */
export interface StreamResult {
  content: string;
  reasoningContent: string;
  toolCalls: AssembledToolCall[];
  finishReason: string;
  hasToolCalls: boolean;
  usage?: StreamChunk["usage"];
}

/** Sentinel terminal marker from the upstream provider. */
const DONE = "[DONE]";
const DATA_PREFIX = "data: ";

/**
 * Mirrors `crate::api::parse_sse_line` (src/api/types.rs:341). Strips the
 * `data: ` prefix, returns `null` for `[DONE]`, and parses the rest as JSON.
 * Unparseable lines return `null` (logged at debug in Rust; silently dropped
 * here — they're typically SSE comments or keepalives).
 */
function parseSseLine(line: string): StreamChunk | null {
  if (!line.startsWith(DATA_PREFIX)) return null;
  const payload = line.slice(DATA_PREFIX.length);
  if (payload === DONE) return null;
  try {
    return JSON.parse(payload) as StreamChunk;
  } catch {
    return null;
  }
}

/**
 * Ported from `StreamProcessor` (src/agent/core.rs).
 *
 * Feed raw SSE bytes as they arrive from `fetch`'s `response.body`; call
 * `finish()` once the stream ends to retrieve the reassembled turn result.
 */
export class StreamProcessor {
  private buffer = "";
  private fullContent = "";
  private reasoningContent = "";
  private hasToolCalls = false;
  private toolCallsAccum: AssembledToolCall[] = [];
  private finishReason = "";
  private usage?: StreamChunk["usage"];

  /** Feed a UTF-8 chunk (from a `ReadableStream<Uint8Array>` reader). */
  feedBytes(bytes: Uint8Array): StreamEvent[] {
    this.buffer += new TextDecoder().decode(bytes);
    return this.drainBuffer();
  }

  /** Also accept a string chunk (handy for tests and pre-decoded streams). */
  feedString(text: string): StreamEvent[] {
    this.buffer += text;
    return this.drainBuffer();
  }

  private drainBuffer(): StreamEvent[] {
    const events: StreamEvent[] = [];
    let nlIdx: number;
    while ((nlIdx = this.buffer.indexOf("\n")) !== -1) {
      // Extract the complete line up to (but excluding) the newline.
      const line = this.buffer.slice(0, nlIdx).trim();
      this.buffer = this.buffer.slice(nlIdx + 1);
      const ev = this.processLine(line);
      if (ev) events.push(ev);
    }
    return events;
  }

  /** Process one SSE text line. Ported from `StreamProcessor::process_line`. */
  private processLine(line: string): StreamEvent | null {
    // 1. Detect daemon error events BEFORE chunk parsing. (core.rs:72-81)
    const payload = line.startsWith(DATA_PREFIX) ? line.slice(DATA_PREFIX.length) : line;
    if (payload !== DONE) {
      try {
        const parsed = JSON.parse(payload) as { error?: unknown };
        if (typeof parsed.error === "string") {
          return { type: "streamError", message: parsed.error };
        }
      } catch {
        // Not JSON (or not an error object) — fall through to chunk parsing.
      }
    }

    const chunk = parseSseLine(line);
    if (!chunk) return null;

    if (chunk.usage) this.usage = chunk.usage;
    const choice = chunk.choices[0];
    if (!choice) return null;

    // 2. Accumulate content/reasoning BEFORE finish_reason. (core.rs:92-101)
    if (choice.delta.content) this.fullContent += choice.delta.content;
    if (choice.delta.reasoning_content) this.reasoningContent += choice.delta.reasoning_content;

    // 3. Accumulate ALL tool-call fragments by index. (core.rs:103-131)
    if (choice.delta.tool_calls) {
      this.hasToolCalls = true;
      for (const tc of choice.delta.tool_calls) this.accumulateToolCall(tc);
    }

    // 4. Check finish_reason AFTER accumulating deltas. (core.rs:134-139)
    if (choice.finish_reason) {
      this.finishReason = choice.finish_reason;
      return { type: "streamDone", finishReason: choice.finish_reason };
    }

    // 5. Emit the most significant delta as the event. (core.rs:142-157)
    if (choice.delta.content) {
      return { type: "contentDelta", text: choice.delta.content };
    }
    if (choice.delta.reasoning_content) {
      return { type: "reasoningDelta", text: choice.delta.reasoning_content };
    }
    if (choice.delta.tool_calls) {
      const tc = choice.delta.tool_calls[0];
      return {
        type: "toolCallDelta",
        index: tc.index,
        id: tc.id,
        name: tc.function?.name,
        arguments: tc.function?.arguments,
      };
    }

    return null;
  }

  /** Ported from the tool-call accumulation loop (core.rs:105-130). */
  private accumulateToolCall(tc: StreamToolCall): void {
    const idx = tc.index;
    while (this.toolCallsAccum.length <= idx) {
      this.toolCallsAccum.push({
        id: "",
        type: "function",
        function: { name: "", arguments: "" },
      });
    }
    const entry = this.toolCallsAccum[idx];
    if (tc.id) entry.id = tc.id;
    if (tc.function?.name) entry.function.name = tc.function.name;
    if (tc.function?.arguments) {
      // String concatenation across fragments — the load-bearing detail.
      entry.function.arguments += tc.function.arguments;
    }
  }

  /**
   * Flush any trailing partial line and return the reassembled result.
   * Called once the HTTP stream ends.
   */
  finish(): StreamResult {
    // The Rust `flush()` drains a final partial line if present; do the same.
    if (this.buffer.length > 0) {
      const remaining = this.buffer.trim();
      this.buffer = "";
      if (remaining) void this.processLine(remaining);
    }
    return {
      content: this.fullContent,
      reasoningContent: this.reasoningContent,
      toolCalls: this.toolCallsAccum,
      finishReason: this.finishReason,
      hasToolCalls: this.hasToolCalls,
      usage: this.usage,
    };
  }
}
