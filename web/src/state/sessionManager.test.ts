import { beforeEach, describe, expect, it } from "vitest";
import { useSessionManager } from "./sessionManager";

function resetManager() {
  useSessionManager.setState({
    entries: {},
    order: [],
    activeId: null,
    connection: "unknown",
    contextWindow: null,
  });
}

describe("sessionManager", () => {
  beforeEach(resetManager);

  it("createLocalSession registers an idle entry and makes it active", () => {
    const id = useSessionManager.getState().createLocalSession("test");
    const s = useSessionManager.getState();
    expect(s.entries[id].status).toBe("idle");
    expect(s.entries[id].store).toBeDefined();
    expect(s.entries[id].model).toBeNull();
    expect(s.entries[id].mode).toBeNull();
    expect(s.activeId).toBe(id);
    expect(s.order).toContain(id);
  });

  it("entries have independent stores", () => {
    const a = useSessionManager.getState().createLocalSession("a");
    const b = useSessionManager.getState().createLocalSession("b");
    const entries = useSessionManager.getState().entries;
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

describe("sessionManager model and mode state", () => {
  beforeEach(resetManager);

  it("routes session events by daemon id and resets only that entry", () => {
    const m = useSessionManager.getState();
    const a = m.createLocalSession("a", {
      id: "local-a",
      daemonId: "daemon-a",
      projectPath: "/repo",
      activate: false,
    });
    const b = m.createLocalSession("b", {
      id: "local-b",
      daemonId: "daemon-b",
      projectPath: "/repo",
      activate: false,
    });
    m.setSessionModel(b, { provider: "kimi", model: "k2", isOverride: false });
    m.setSessionMode(b, { mode: "normal", isOverride: false });

    m.applyModelChanged({
      seq: 1,
      kind: "model_changed",
      data: { session_id: "daemon-a", provider: "zhipu", model_name: "glm-5.3" },
    });
    m.applyModeChanged({
      seq: 1,
      kind: "mode_changed",
      data: { session_id: "daemon-a", mode: "yolo" },
    });

    let entries = useSessionManager.getState().entries;
    expect(entries[a].model).toEqual({ provider: "zhipu", model: "glm-5.3", isOverride: true });
    expect(entries[a].mode).toEqual({ mode: "yolo", isOverride: true });
    expect(entries[b].model).toEqual({ provider: "kimi", model: "k2", isOverride: false });
    expect(entries[b].mode).toEqual({ mode: "normal", isOverride: false });

    m.applyModelChanged({
      seq: 2,
      kind: "model_changed",
      data: { session_id: "daemon-a", provider: "kimi", model_name: "k2", reset: true },
    });
    m.applyModeChanged({
      seq: 2,
      kind: "mode_changed",
      data: { session_id: "daemon-a", mode: "accept_edits", reset: true },
    });

    entries = useSessionManager.getState().entries;
    expect(entries[a].model).toEqual({ provider: "kimi", model: "k2", isOverride: false });
    expect(entries[a].mode).toEqual({ mode: "accept_edits", isOverride: false });
    expect(entries[b].model).toEqual({ provider: "kimi", model: "k2", isOverride: false });
    expect(entries[b].mode).toEqual({ mode: "normal", isOverride: false });
  });

  it("clears an override and shows the legacy fallback when reset has a null provider", () => {
    const m = useSessionManager.getState();
    const id = m.createLocalSession("a", {
      id: "local-a",
      daemonId: "daemon-a",
      activate: false,
    });
    m.setSessionModel(id, { provider: "zhipu", model: "old-override", isOverride: true });

    m.applyModelChanged({
      seq: 1,
      kind: "model_changed",
      data: {
        session_id: "daemon-a",
        provider: null,
        model_name: "legacy-main",
        reset: true,
      },
    });

    expect(useSessionManager.getState().entries[id].model).toEqual({
      provider: null,
      model: "legacy-main",
      isOverride: false,
    });
  });

  it("routes events by local id before a daemon id is assigned", () => {
    const m = useSessionManager.getState();
    const id = m.createLocalSession("local", { id: "web-local", activate: false });
    m.applyModelChanged({
      seq: 1,
      kind: "model_changed",
      data: { session_id: "web-local", provider: "kimi", model_name: "k3" },
    });
    m.applyModeChanged({
      seq: 1,
      kind: "mode_changed",
      data: { session_id: "web-local", mode: "accept_edits" },
    });
    const entry = useSessionManager.getState().entries[id];
    expect(entry.model).toEqual({ provider: "kimi", model: "k3", isOverride: true });
    expect(entry.mode).toEqual({ mode: "accept_edits", isOverride: true });
  });

  it("rejects out-of-order and duplicate session model and mode envelopes", () => {
    const m = useSessionManager.getState();
    const id = m.createLocalSession("a", {
      id: "local-a",
      daemonId: "daemon-a",
      activate: false,
    });

    m.applyModelChanged({
      seq: 2,
      kind: "model_changed",
      data: { session_id: "daemon-a", provider: "kimi", model_name: "new-model" },
    });
    m.applyModelChanged({
      seq: 1,
      kind: "model_changed",
      data: { session_id: "daemon-a", provider: "zhipu", model_name: "old-model" },
    });
    m.applyModelChanged({
      seq: 2,
      kind: "model_changed",
      data: { session_id: "daemon-a", provider: "zhipu", model_name: "duplicate-model" },
    });
    m.applyModeChanged({
      seq: 4,
      kind: "mode_changed",
      data: { session_id: "daemon-a", mode: "yolo" },
    });
    m.applyModeChanged({
      seq: 3,
      kind: "mode_changed",
      data: { session_id: "daemon-a", mode: "normal" },
    });

    const entry = useSessionManager.getState().entries[id];
    expect(entry.model).toEqual({ provider: "kimi", model: "new-model", isOverride: true });
    expect(entry.mode).toEqual({ mode: "yolo", isOverride: true });
  });

  it("uses the same per-entry seq cursor for global defaults and targeted resets", () => {
    const m = useSessionManager.getState();
    const id = m.createLocalSession("a", {
      id: "local-a",
      daemonId: "daemon-a",
      activate: false,
    });
    m.setSessionModel(id, { provider: "kimi", model: "initial", isOverride: false });

    m.applyModelChanged({
      seq: 20,
      kind: "model_changed",
      data: { provider: "anthropic", model_name: "new-global" },
    });
    m.applyModelChanged({
      seq: 19,
      kind: "model_changed",
      data: {
        session_id: "daemon-a",
        provider: "zhipu",
        model_name: "stale-targeted-reset",
        reset: true,
      },
    });
    expect(useSessionManager.getState().entries[id].model).toEqual({
      provider: "anthropic",
      model: "new-global",
      isOverride: false,
    });

    m.applyModelChanged({
      seq: 21,
      kind: "model_changed",
      data: {
        session_id: "daemon-a",
        provider: null,
        model_name: "new-targeted-reset",
        reset: true,
      },
    });
    m.applyModelChanged({
      seq: 20,
      kind: "model_changed",
      data: { provider: "zhipu", model_name: "stale-global" },
    });
    expect(useSessionManager.getState().entries[id].model).toEqual({
      provider: null,
      model: "new-targeted-reset",
      isOverride: false,
    });
  });

  it("updates unoverridden entries on a global model change without clearing overrides", () => {
    const m = useSessionManager.getState();
    const overridden = m.createLocalSession("a", { activate: false });
    const following = m.createLocalSession("b", { activate: false });
    m.setSessionModel(overridden, { provider: "zhipu", model: "glm-5.2", isOverride: true });
    m.setSessionModel(following, { provider: "kimi", model: "k2", isOverride: false });

    m.applyModelChanged({
      seq: 1,
      kind: "model_changed",
      data: { provider: "anthropic", model_name: "claude-sonnet-4-5" },
    });

    const entries = useSessionManager.getState().entries;
    expect(entries[overridden].model).toEqual({
      provider: "zhipu",
      model: "glm-5.2",
      isOverride: true,
    });
    expect(entries[following].model).toEqual({
      provider: "anthropic",
      model: "claude-sonnet-4-5",
      isOverride: false,
    });
  });

  it("leaves unsynchronized entries unknown on a global model change", () => {
    const m = useSessionManager.getState();
    const id = m.createLocalSession("new", { activate: false });
    m.applyModelChanged({
      seq: 1,
      kind: "model_changed",
      data: { provider: "anthropic", model_name: "claude-sonnet-4-5" },
    });
    expect(useSessionManager.getState().entries[id].model).toBeNull();
  });
});

describe("sessionManager worktree binding", () => {
  beforeEach(resetManager);

  it("createLocalSession accepts explicit id and worktree", () => {
    const id = useSessionManager.getState().createLocalSession("bound", {
      id: "daemon-1",
      daemonId: "daemon-1",
      worktree: { path: ".worktrees/a", branch: "a" },
    });
    const e = useSessionManager.getState().entries[id];
    expect(id).toBe("daemon-1");
    expect(e.daemonId).toBe("daemon-1");
    expect(e.worktree?.branch).toBe("a");
  });

  it("setWorktree updates and clears the binding", () => {
    const id = useSessionManager.getState().createLocalSession("x");
    useSessionManager.getState().setWorktree(id, { path: ".worktrees/a", branch: "a" });
    expect(useSessionManager.getState().entries[id].worktree?.branch).toBe("a");
    useSessionManager.getState().setWorktree(id, null);
    expect(useSessionManager.getState().entries[id].worktree).toBeUndefined();
  });
});
