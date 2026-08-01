import { beforeEach, describe, expect, it } from "vitest";
import { useSessionManager } from "./sessionManager";

describe("sessionManager", () => {
  beforeEach(() => {
    useSessionManager.setState({
      entries: {},
      order: [],
      activeId: null,
      connection: "unknown",
      modelName: null,
    });
  });

  it("createLocalSession registers an idle entry and makes it active", () => {
    const id = useSessionManager.getState().createLocalSession("test");
    const s = useSessionManager.getState();
    expect(s.entries[id].status).toBe("idle");
    expect(s.entries[id].store).toBeDefined();
    expect(s.activeId).toBe(id);
    expect(s.order).toContain(id);
  });

  it("entries have independent stores", () => {
    const a = useSessionManager.getState().createLocalSession("a");
    const b = useSessionManager.getState().createLocalSession("b");
    const entries = useSessionManager.getState().entries; // 重新取最新 state
    entries[a].store.getState().pushUserMessage("hi");
    expect(entries[b].store.getState().messages).toHaveLength(0);
  });

  it("setStatus / setPreview update only the target entry", () => {
    const m = useSessionManager.getState();
    const a = m.createLocalSession("a");
    const b = m.createLocalSession("b");
    m.setStatus(a, "running");
    m.setPreview(a, "working…");
    const s = useSessionManager.getState();
    expect(s.entries[a].status).toBe("running");
    expect(s.entries[a].lastPreview).toBe("working…");
    expect(s.entries[b].status).toBe("idle");
  });

  it("removeSession drops the entry and fixes activeId", () => {
    const m = useSessionManager.getState();
    const a = m.createLocalSession("a");
    const b = m.createLocalSession("b");
    m.setActive(b);
    m.removeSession(b);
    const s = useSessionManager.getState();
    expect(s.entries[b]).toBeUndefined();
    expect(s.activeId).toBe(a);
  });
});
