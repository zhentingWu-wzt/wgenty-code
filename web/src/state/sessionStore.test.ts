import { describe, expect, it } from "vitest";
import { createSessionStore } from "./sessionStore";

/** Wait one animation frame (rAF fallback: a macrotask). */
const nextFrame = () =>
  new Promise<void>((resolve) => {
    if (typeof requestAnimationFrame === "function") {
      requestAnimationFrame(() => resolve());
    } else {
      setTimeout(() => resolve(), 0);
    }
  });

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

  it("appendAssistant batches deltas per frame; finalizeAssistant flushes synchronously", async () => {
    const s = createSessionStore();
    const id = s.getState().beginAssistantRound(1);
    s.getState().appendAssistant(id, { type: "reasoningDelta", text: "think " });
    s.getState().appendAssistant(id, { type: "contentDelta", text: "Hel" });
    s.getState().appendAssistant(id, { type: "contentDelta", text: "lo" });
    // Not committed yet — no animation frame has passed.
    expect(s.getState().messages.find((m) => m.id === id)!.content).toBe("");
    await nextFrame();
    const streamed = s.getState().messages.find((m) => m.id === id)!;
    expect(streamed.content).toBe("Hello");
    expect(streamed.reasoning).toBe("think ");
    // Finalize flushes synchronously — no frame needed for the tail.
    s.getState().appendAssistant(id, { type: "contentDelta", text: "!" });
    s.getState().finalizeAssistant(id);
    expect(s.getState().messages.find((m) => m.id === id)!.content).toBe("Hello!");
  });

  it("clear discards uncommitted deltas", async () => {
    const s = createSessionStore();
    const id = s.getState().beginAssistantRound(1);
    s.getState().appendAssistant(id, { type: "contentDelta", text: "doomed" });
    s.getState().clear();
    await nextFrame();
    await nextFrame();
    expect(s.getState().messages).toHaveLength(0);
  });

  it("setAgentPhase skips identical values (per-delta callers)", () => {
    const s = createSessionStore();
    s.getState().setAgentPhase({ phase: "streaming" });
    const first = s.getState().agentPhase;
    s.getState().setAgentPhase({ phase: "streaming" });
    expect(s.getState().agentPhase).toBe(first); // same reference — set was skipped
    s.getState().setAgentPhase({ phase: "executing", toolName: "grep" });
    expect(s.getState().agentPhase).toEqual({ phase: "executing", toolName: "grep" });
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
