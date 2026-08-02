import { describe, expect, it, vi } from "vitest";
import { runAgentLoop } from "./loop";
import type { DaemonClient } from "../api/client";
import type { ExecuteToolRequest } from "../api/types";

/** Build a chatStream response body from raw SSE text. */
function sseBody(text: string): ReadableStream<Uint8Array> {
  const body = new Response(text).body;
  if (!body) throw new Error("Response.body unavailable in this test environment");
  return body;
}

const TOOL_CALL_ROUND = [
  `data: ${JSON.stringify({
    id: "c1",
    object: "chat.completion.chunk",
    created: 0,
    model: "m",
    choices: [
      {
        index: 0,
        delta: {
          tool_calls: [
            { index: 0, id: "call_1", function: { name: "file_read", arguments: "{}" } },
          ],
        },
      },
    ],
  })}`,
  `data: ${JSON.stringify({
    id: "c1",
    object: "chat.completion.chunk",
    created: 0,
    model: "m",
    choices: [{ index: 0, delta: {}, finish_reason: "tool_calls" }],
  })}`,
  "data: [DONE]",
  "",
].join("\n");

const FINAL_ROUND = [
  `data: ${JSON.stringify({
    id: "c2",
    object: "chat.completion.chunk",
    created: 0,
    model: "m",
    choices: [{ index: 0, delta: { content: "done" } }],
  })}`,
  `data: ${JSON.stringify({
    id: "c2",
    object: "chat.completion.chunk",
    created: 0,
    model: "m",
    choices: [{ index: 0, delta: {}, finish_reason: "stop" }],
  })}`,
  "data: [DONE]",
  "",
].join("\n");

function mockClient() {
  const executeTool = vi.fn().mockResolvedValue({ success: true, content: "file contents" });
  const chatStream = vi
    .fn()
    .mockResolvedValueOnce({ body: sseBody(TOOL_CALL_ROUND) })
    .mockResolvedValueOnce({ body: sseBody(FINAL_ROUND) });
  const client = { chatStream, executeTool } as unknown as DaemonClient;
  return { client, executeTool };
}

const noopCallbacks = {
  onStreamEvent: () => {},
  onToolExecution: () => {},
  onPermissionRequired: () => Promise.resolve("deny" as const),
};

describe("runAgentLoop turn_id threading", () => {
  it("forwards the turn's turnId on every /tools/execute request", async () => {
    const { client, executeTool } = mockClient();
    const final = await runAgentLoop({
      client,
      messages: [{ role: "user", content: "read it" }],
      sessionId: "sess-1",
      turnId: "sess-1-turn-123",
      callbacks: noopCallbacks,
    });
    expect(final).toBe("done");
    expect(executeTool).toHaveBeenCalledTimes(1);
    const req = executeTool.mock.calls[0][0] as ExecuteToolRequest;
    expect(req.turn_id).toBe("sess-1-turn-123");
    expect(req.session_id).toBe("sess-1");
    expect(req.tool_name).toBe("file_read");
  });

  it("omits turn_id when the caller did not mint one", async () => {
    const { client, executeTool } = mockClient();
    await runAgentLoop({
      client,
      messages: [{ role: "user", content: "read it" }],
      sessionId: "sess-1",
      callbacks: noopCallbacks,
    });
    const req = executeTool.mock.calls[0][0] as ExecuteToolRequest;
    expect(req.turn_id).toBeUndefined();
    // …and JSON.stringify drops the key entirely (daemon sees no turn_id).
    expect(JSON.stringify(req)).not.toContain("turn_id");
  });
});

describe("runAgentLoop tool-result history", () => {
  it("round 2 request pairs every tool_call with a tool message", async () => {
    const { client } = mockClient();
    const chatStream = client.chatStream as ReturnType<typeof vi.fn>;
    await runAgentLoop({
      client,
      messages: [{ role: "user", content: "read it" }],
      sessionId: "sess-1",
      callbacks: noopCallbacks,
    });

    expect(chatStream).toHaveBeenCalledTimes(2);
    const round2Messages = chatStream.mock.calls[1][0] as Array<{
      role: string;
      tool_call_id?: string;
      tool_calls?: Array<{ id: string }>;
    }>;

    // The assistant message carrying the tool call…
    const assistantIdx = round2Messages.findIndex((m) => m.role === "assistant");
    expect(assistantIdx).toBeGreaterThan(-1);
    expect(round2Messages[assistantIdx].tool_calls?.map((t) => t.id)).toEqual(["call_1"]);

    // …must be immediately followed by a tool message per tool_call_id —
    // otherwise the upstream rejects with invalid_request_error.
    const toolMsg = round2Messages[assistantIdx + 1];
    expect(toolMsg?.role).toBe("tool");
    expect(toolMsg?.tool_call_id).toBe("call_1");
  });
});
