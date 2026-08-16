import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { SessionTabBar } from "./SessionTabBar";
import { useSessionManager } from "../../state/sessionManager";
import { useUiStore } from "../../state/uiStore";

function seed() {
  const mgr = useSessionManager.getState();
  const a = mgr.createLocalSession("Alpha");
  const b = mgr.createLocalSession("Beta");
  useSessionManager.getState().setActive(a);
  // Mirror what startUiSync does in the real app: activating a session also
  // sets the unified active tab.
  useUiStore.setState({ openTabs: [a, b], activeTabId: a, subagentTabs: {} });
  return { a, b };
}

describe("SessionTabBar", () => {
  beforeEach(() => {
    useSessionManager.setState({ entries: {}, order: [], activeId: null });
    useUiStore.setState({ openTabs: [], activeTabId: null, subagentTabs: {}, previewTabs: {} });
  });

  it("renders a tab per open session, marks the active one", () => {
    seed();
    render(<SessionTabBar />);
    expect(screen.getByText("Alpha")).toBeInTheDocument();
    expect(screen.getByText("Beta")).toBeInTheDocument();
    expect(screen.getByText("Alpha").closest("[data-active]")).toHaveAttribute("data-active", "true");
  });

  it("clicking a tab activates its session", async () => {
    const { b } = seed();
    render(<SessionTabBar />);
    await userEvent.click(screen.getByText("Beta"));
    expect(useSessionManager.getState().activeId).toBe(b);
  });

  it("closing the active tab activates its neighbor", async () => {
    const { a, b } = seed();
    render(<SessionTabBar />);
    const tab = screen.getByText("Alpha").closest("[data-active]")!;
    await userEvent.click(tab.querySelector("[data-close]") as HTMLElement);
    expect(useUiStore.getState().openTabs).toEqual([b]);
    expect(useSessionManager.getState().activeId).toBe(b);
    expect(a).not.toBe(useSessionManager.getState().activeId);
  });
});
