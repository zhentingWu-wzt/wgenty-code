import { beforeEach, describe, expect, it } from "vitest";
import { useUiStore, type PreviewTabMeta } from "./uiStore";

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

describe("uiStore preview tabs", () => {
  const previewMeta = (absPath: string, kind: "text" | "binary" = "text"): PreviewTabMeta => ({
    workspaceRoot: "/w/proj",
    absPath,
    relPath: absPath.replace("/w/proj/", ""),
    kind,
  });

  beforeEach(() => {
    useUiStore.setState({ openTabs: [], activeTabId: null, previewTabs: {} });
  });

  it("openPreviewTab is idempotent per path (no duplicate tab, just activates)", () => {
    useUiStore.getState().openPreviewTab(previewMeta("/w/proj/src/main.rs"));
    expect(useUiStore.getState().openTabs).toEqual(["preview:/w/proj/src/main.rs"]);
    expect(useUiStore.getState().activeTabId).toBe("preview:/w/proj/src/main.rs");
    expect(useUiStore.getState().previewTabs["preview:/w/proj/src/main.rs"]).toEqual(
      previewMeta("/w/proj/src/main.rs"),
    );

    // Re-open the same path: focus only — no second tab.
    useUiStore.getState().openPreviewTab(previewMeta("/w/proj/src/main.rs"));
    expect(useUiStore.getState().openTabs).toEqual(["preview:/w/proj/src/main.rs"]);

    // A different path opens (and activates) a second tab.
    useUiStore.getState().openPreviewTab(previewMeta("/w/proj/logo.png", "binary"));
    expect(useUiStore.getState().openTabs).toEqual([
      "preview:/w/proj/src/main.rs",
      "preview:/w/proj/logo.png",
    ]);
    expect(useUiStore.getState().activeTabId).toBe("preview:/w/proj/logo.png");
  });

  it("closeTab cleans the preview meta alongside the tab", () => {
    useUiStore.getState().openPreviewTab(previewMeta("/w/proj/src/main.rs"));
    useUiStore.getState().openPreviewTab(previewMeta("/w/proj/logo.png", "binary"));
    // Closing the first tab activates its right neighbor, as with any tab.
    expect(useUiStore.getState().closeTab("preview:/w/proj/src/main.rs")).toBe(
      "preview:/w/proj/logo.png",
    );
    expect(useUiStore.getState().openTabs).toEqual(["preview:/w/proj/logo.png"]);
    expect(useUiStore.getState().previewTabs).not.toHaveProperty("preview:/w/proj/src/main.rs");
    expect(useUiStore.getState().previewTabs).toHaveProperty("preview:/w/proj/logo.png");

    // Closing a non-preview tab leaves preview metas alone.
    useUiStore.getState().openTab("session-1");
    useUiStore.getState().closeTab("session-1");
    expect(useUiStore.getState().previewTabs).toHaveProperty("preview:/w/proj/logo.png");
  });

  it("pruneTabs drops openTabs only — preview meta survives (current semantics)", () => {
    useUiStore.getState().openPreviewTab(previewMeta("/w/proj/src/main.rs"));
    useUiStore.getState().pruneTabs(["preview:/w/proj/src/main.rs"]);
    expect(useUiStore.getState().openTabs).toEqual([]);
    expect(useUiStore.getState().previewTabs["preview:/w/proj/src/main.rs"]).toBeDefined();
  });
});
