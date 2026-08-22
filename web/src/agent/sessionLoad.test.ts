import { describe, expect, it } from "vitest";
import { sessionMessagesToDisplay } from "./sessionLoad";
import type { SessionMessage } from "../api/types";

const CALL = {
  id: "c1",
  type: "function",
  function: { name: "file_read", arguments: "{}" },
} as const;

describe("sessionMessagesToDisplay", () => {
  it("tool results become standalone entries in wire order", () => {
    const messages: SessionMessage[] = [
      { role: "user", content: "read it" },
      { role: "assistant", content: "checking…", tool_calls: [CALL] },
      {
        role: "tool",
        tool_call_id: "c1",
        content: JSON.stringify({ success: true, content: "ok" }),
      },
    ];
    const out = sessionMessagesToDisplay(messages);
    // user → assistant text → tool entry, in wire order.
    expect(out.map((m) => m.role)).toEqual(["user", "assistant", "tool"]);
    const tool = out.find((m) => m.role === "tool")!;
    expect(tool.toolExec?.call.function.name).toBe("file_read");
    expect(tool.toolExec?.response.content).toBe("ok");
  });

  it("multiple tool calls interleave in call order", () => {
    const messages: SessionMessage[] = [
      { role: "user", content: "go" },
      {
        role: "assistant",
        content: "checking…",
        tool_calls: [
          { ...CALL, id: "c1", function: { name: "grep", arguments: "{}" } },
          { ...CALL, id: "c2", function: { name: "file_read", arguments: "{}" } },
        ],
      },
      {
        role: "tool",
        tool_call_id: "c1",
        content: JSON.stringify({ success: true, content: "hit" }),
      },
      {
        role: "tool",
        tool_call_id: "c2",
        content: JSON.stringify({ success: true, content: "body" }),
      },
    ];
    const out = sessionMessagesToDisplay(messages);
    const tools = out.filter((m) => m.role === "tool");
    expect(tools.map((t) => t.toolExec?.call.function.name)).toEqual(["grep", "file_read"]);
    expect(tools.map((t) => t.toolExec?.response.content)).toEqual(["hit", "body"]);
  });
});
