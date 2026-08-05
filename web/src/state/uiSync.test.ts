import { afterEach, describe, expect, it } from "vitest";
import { useSessionManager } from "./sessionManager";
import { useUiStore } from "./uiStore";
import { startUiSync } from "./uiSync";

describe("uiSync", () => {
  afterEach(() => {
    useSessionManager.setState({ entries: {}, order: [], activeId: null });
    useUiStore.setState({ openTabs: [] });
  });

  it("auto-opens a tab for the newly activated session", () => {
    const stop = startUiSync();
    const id = useSessionManager.getState().createLocalSession("S1");
    expect(useUiStore.getState().openTabs).toEqual([id]);
    stop();
  });

  it("prunes tabs of removed sessions", () => {
    const stop = startUiSync();
    const id = useSessionManager.getState().createLocalSession("S1");
    useSessionManager.getState().removeSession(id);
    expect(useUiStore.getState().openTabs).toEqual([]);
    stop();
  });
});
