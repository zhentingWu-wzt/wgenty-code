import { beforeEach, describe, expect, it } from "vitest";
import { sessionMessagesToDisplay } from "./sessionLoad";
import { useDisplayPrefs } from "../state/displayPrefs";
import type { SessionMessage } from "../api/types";

const CALL = {
  id: "c1",
  type: "function",
  function: { name: "file_read", arguments: "{}" },
} as const;

describe("sessionMessagesToDisplay", () => {
  beforeEach(() => useDisplayPrefs.setState({ mode: "single" }));

  it("single mode: folds tool results into the assistant bubble's toolExecs", () => {
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
    expect(out.map((m) => m.role)).toEqual(["user", "assistant"]);
    const assistant = out.find((m) => m.role === "assistant")!;
    expect(assistant.toolExecs).toHaveLength(1);
    expect(assistant.toolExecs?.[0].response.content).toBe("ok");
  });

  it("timeline mode: tool results become standalone entries in wire order", () => {
    useDisplayPrefs.setState({ mode: "timeline" });
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
    // user → assistant text → tool entry, in wire order, no toolExecs on the bubble.
    expect(out.map((m) => m.role)).toEqual(["user", "assistant", "tool"]);
    const assistant = out.find((m) => m.role === "assistant")!;
    expect(assistant.toolExecs).toBeUndefined();
    const tool = out.find((m) => m.role === "tool")!;
    expect(tool.toolExec?.call.function.name).toBe("file_read");
    expect(tool.toolExec?.response.content).toBe("ok");
  });

  it("timeline mode: multiple tool calls interleave in call order", () => {
    useDisplayPrefs.setState({ mode: "timeline" });
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
