import { describe, expect, it } from "vitest";
import { createSessionStore } from "./sessionStore";

describe("createSessionStore", () => {
  it("two instances are fully isolated", () => {
    const a = createSessionStore();
    const b = createSessionStore();
    a.getState().pushUserMessage("hello a");
    expect(a.getState().messages).toHaveLength(1);
    expect(b.getState().messages).toHaveLength(0);
  });

  it("abort registration is per-instance (stopRunning only aborts its own)", () => {
    const a = createSessionStore();
    const b = createSessionStore();
    const ctrlA = new AbortController();
    const ctrlB = new AbortController();
    a.getState().registerAbort(ctrlA);
    b.getState().registerAbort(ctrlB);
    a.getState().stopRunning();
    expect(ctrlA.signal.aborted).toBe(true);
    expect(ctrlB.signal.aborted).toBe(false);
  });

  it("streaming round: begin → append → finalize", () => {
    const s = createSessionStore();
    const id = s.getState().beginAssistantRound(0);
    s.getState().appendAssistant(id, { type: "contentDelta", text: "hi" });
    s.getState().finalizeAssistant(id);
    const msg = s.getState().messages.find((m) => m.id === id)!;
    expect(msg.content).toBe("hi");
    expect(msg.streaming).toBe(false);
  });

  it("timeline tool entries: pushToolStart inserts a running placeholder, completeTool fills it", () => {
    const s = createSessionStore();
    const id = s.getState().pushToolStart("file_read", { path: "/a" });
    const running = s.getState().messages.find((m) => m.id === id)!;
    expect(running.role).toBe("tool");
    expect(running.streaming).toBe(true);
    expect(running.toolName).toBe("file_read");
    expect(running.toolArgs).toEqual({ path: "/a" });

    s.getState().completeTool(id, {
      call: { id: "c1", type: "function", function: { name: "file_read", arguments: "{}" } },
      response: { success: true, content: "ok" },
    });
    const done = s.getState().messages.find((m) => m.id === id)!;
    expect(done.streaming).toBe(false);
    expect(done.toolExec?.response.content).toBe("ok");
  });
});
