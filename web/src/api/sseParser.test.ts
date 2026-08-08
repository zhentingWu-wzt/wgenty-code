import { describe, expect, it } from "vitest";
import { StreamProcessor } from "./sseParser";

/** Helper: build an OpenAI-style `data:` line for a chunk. */
function dataLine(payload: Record<string, unknown>): string {
  return `data: ${JSON.stringify(payload)}\n`;
}

function contentChunk(text: string, finishReason?: string): string {
  return dataLine({
    id: "x",
    object: "chat.completion.chunk",
    created: 0,
    model: "m",
    choices: [{ index: 0, delta: { content: text }, finish_reason: finishReason ?? null }],
  });
}

/** Build a tool-call delta chunk. Arguments are plain strings (concatenated). */
function toolChunk(
  calls: Array<{ index: number; id?: string; name?: string; arguments?: string }>,
  finishReason?: string,
): string {
  return dataLine({
    id: "x",
    object: "chat.completion.chunk",
    created: 0,
    model: "m",
    choices: [
      {
        index: 0,
        delta: {
          tool_calls: calls.map((c) => ({
            index: c.index,
            ...(c.id ? { id: c.id } : {}),
            function: {
              ...(c.name ? { name: c.name } : {}),
              ...(c.arguments ? { arguments: c.arguments } : {}),
            },
          })),
        },
        finish_reason: finishReason ?? null,
      },
    ],
  });
}

describe("StreamProcessor", () => {
  it("accumulates content deltas and emits streamDone on finish", () => {
    const p = new StreamProcessor();
    const events = [
      ...p.feedString(contentChunk("Hello")),
      ...p.feedString(contentChunk(", ")),
      ...p.feedString(contentChunk("world!")),
      ...p.feedString(contentChunk("", "stop")),
    ];

    const contentEvents = events.filter((e) => e.type === "contentDelta");
    expect(contentEvents.map((e) => (e as { text: string }).text).join("")).toBe("Hello, world!");
    expect(events.some((e) => e.type === "streamDone")).toBe(true);

    const result = p.finish();
    expect(result.content).toBe("Hello, world!");
    expect(result.finishReason).toBe("stop");
    expect(result.hasToolCalls).toBe(false);
  });

  it("treats [DONE] as the terminal sentinel (no event, no error)", () => {
    const p = new StreamProcessor();
    const events = [...p.feedString(contentChunk("hi", "stop")), ...p.feedString("data: [DONE]\n")];
    expect(events.some((e) => e.type === "streamError")).toBe(false);
    expect(p.finish().content).toBe("hi");
  });

  it("detects a daemon/stream error payload before chunk parsing", () => {
    const p = new StreamProcessor();
    const events = p.feedString(dataLine({ error: "upstream rate limited" }));
    expect(events).toHaveLength(1);
    expect(events[0]).toEqual({ type: "streamError", message: "upstream rate limited" });
  });

  it("reassembles a tool call split across multiple fragments", () => {
    // The most error-prone case: arguments arrive as string fragments that must
    // be concatenated, and id/name may arrive in a different fragment than args.
    const pathFirstHalf = '{"path":"/tmp';
    const pathSecondHalf = '/a.txt"}';
    const p = new StreamProcessor();
    const events = [
      ...p.feedString(toolChunk([{ index: 0, id: "call_1", name: "file_write" }])),
      ...p.feedString(toolChunk([{ index: 0, arguments: pathFirstHalf }])),
      ...p.feedString(toolChunk([{ index: 0, arguments: pathSecondHalf }])),
      ...p.feedString(contentChunk("", "tool_calls")),
    ];

    expect(events.some((e) => e.type === "streamDone")).toBe(true);
    const result = p.finish();
    expect(result.hasToolCalls).toBe(true);
    expect(result.toolCalls).toHaveLength(1);
    expect(result.toolCalls[0].id).toBe("call_1");
    expect(result.toolCalls[0].function.name).toBe("file_write");
    expect(result.toolCalls[0].function.arguments).toBe('{"path":"/tmp/a.txt"}');
    expect(result.finishReason).toBe("tool_calls");
  });

  it("accumulates multiple distinct tool calls by index", () => {
    const p = new StreamProcessor();
    p.feedString(toolChunk([{ index: 0, id: "a", name: "grep", arguments: '{"q":"foo' }]));
    p.feedString(toolChunk([{ index: 1, id: "b", name: "glob", arguments: '{"p":"x' }]));
    p.feedString(toolChunk([{ index: 0, arguments: '"}' }]));
    p.feedString(toolChunk([{ index: 1, arguments: '"}' }]));
    p.feedString(contentChunk("", "tool_calls"));

    const result = p.finish();
    expect(result.toolCalls).toHaveLength(2);
    expect(result.toolCalls[0].function.name).toBe("grep");
    expect(result.toolCalls[0].function.arguments).toBe('{"q":"foo"}');
    expect(result.toolCalls[1].function.name).toBe("glob");
    expect(result.toolCalls[1].function.arguments).toBe('{"p":"x"}');
  });

  it("preserves content that ships in the same chunk as finish_reason", () => {
    // Regression guard for core.rs:92-94: returning early on finish_reason
    // would lose trailing content. The result must contain the final token.
    const p = new StreamProcessor();
    p.feedString(contentChunk("final", "stop"));
    expect(p.finish().content).toBe("final");
  });

  it("handles bytes split mid-line across feedBytes calls", () => {
    const p = new StreamProcessor();
    const full = contentChunk("abc", "stop");
    const mid = Math.floor(full.length / 2);
    const encoder = new TextEncoder();
    const a = p.feedBytes(encoder.encode(full.slice(0, mid)));
    const b = p.feedBytes(encoder.encode(full.slice(mid)));
    // No complete line until the second half arrives.
    expect(a).toHaveLength(0);
    expect(b.some((e) => e.type === "streamDone")).toBe(true);
    expect(p.finish().content).toBe("abc");
  });
});
