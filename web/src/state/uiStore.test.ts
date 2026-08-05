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

  it("setLeftWidth clamps to [180, 400]", () => {
    useUiStore.getState().setLeftWidth(100);
    expect(useUiStore.getState().leftWidth).toBe(180);
    useUiStore.getState().setLeftWidth(500);
    expect(useUiStore.getState().leftWidth).toBe(400);
    useUiStore.getState().setLeftWidth(300);
    expect(useUiStore.getState().leftWidth).toBe(300);
  });
});

describe("uiStore tabs", () => {
  beforeEach(() => {
    useUiStore.setState({ openTabs: [] });
  });

  it("openTab appends once", () => {
    useUiStore.getState().openTab("a");
    useUiStore.getState().openTab("a");
    useUiStore.getState().openTab("b");
    expect(useUiStore.getState().openTabs).toEqual(["a", "b"]);
  });

  it("closeTab returns the neighbor to activate", () => {
    useUiStore.setState({ openTabs: ["a", "b", "c"] });
    expect(useUiStore.getState().closeTab("b")).toBe("c");
    expect(useUiStore.getState().openTabs).toEqual(["a", "c"]);
    expect(useUiStore.getState().closeTab("c")).toBe("a");
    expect(useUiStore.getState().closeTab("a")).toBeNull();
  });

  it("moveTab reorders to the target's position", () => {
    useUiStore.setState({ openTabs: ["a", "b", "c"] });
    useUiStore.getState().moveTab("a", "c");
    expect(useUiStore.getState().openTabs).toEqual(["b", "c", "a"]);
  });

  it("pruneTabs removes gone sessions", () => {
    useUiStore.setState({ openTabs: ["a", "b", "c"] });
    useUiStore.getState().pruneTabs(["b"]);
    expect(useUiStore.getState().openTabs).toEqual(["a", "c"]);
  });
});
