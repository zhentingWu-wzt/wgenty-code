import { beforeEach, describe, expect, it } from "vitest";
import { useUiStore } from "./uiStore";

describe("uiStore", () => {
  beforeEach(() => {
    useUiStore.setState({ leftCollapsed: false, rightPanel: null });
  });

  it("toggleLeft flips leftCollapsed", () => {
    useUiStore.getState().toggleLeft();
    expect(useUiStore.getState().leftCollapsed).toBe(true);
  });

  it("toggleRightPanel opens then closes the same panel", () => {
    useUiStore.getState().toggleRightPanel("skills");
    expect(useUiStore.getState().rightPanel).toBe("skills");
    useUiStore.getState().toggleRightPanel("skills");
    expect(useUiStore.getState().rightPanel).toBeNull();
  });

  it("toggleRightPanel switches to a different panel", () => {
    useUiStore.getState().toggleRightPanel("skills");
    useUiStore.getState().toggleRightPanel("memory");
    expect(useUiStore.getState().rightPanel).toBe("memory");
  });
});
