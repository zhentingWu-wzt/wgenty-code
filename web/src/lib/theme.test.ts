import { beforeEach, describe, expect, it } from "vitest";
import { applyTheme, readStoredTheme } from "./theme";

describe("theme", () => {
  beforeEach(() => {
    localStorage.clear();
    document.documentElement.classList.remove("dark");
  });

  it("defaults to system when nothing stored", () => {
    expect(readStoredTheme()).toBe("system");
  });

  it("applyTheme(dark) adds .dark to documentElement and persists", () => {
    applyTheme("dark");
    expect(document.documentElement.classList.contains("dark")).toBe(true);
    expect(localStorage.getItem("wgenty-theme")).toBe("dark");
  });

  it("applyTheme(light) removes .dark", () => {
    document.documentElement.classList.add("dark");
    applyTheme("light");
    expect(document.documentElement.classList.contains("dark")).toBe(false);
  });
});
