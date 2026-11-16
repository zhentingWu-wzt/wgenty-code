import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { StatusBar } from "./StatusBar";
import type { DaemonClient } from "../api/client";
import { useSessionManager } from "../state/sessionManager";

vi.mock("../features/files/useWorkspaceRoot", () => ({
  useWorkspaceRoot: () => ({ root: "/repo/worktree-a" }),
}));

describe("StatusBar session information", () => {
  beforeEach(() => {
    useSessionManager.setState({
      entries: {},
      order: [],
      activeId: null,
      connection: "connected",
      contextWindow: null,
    });
  });

  it("removes model and mode controls while preserving existing status information", () => {
    const manager = useSessionManager.getState();
    const id = manager.createLocalSession("a", { id: "local-a", daemonId: "daemon-a" });
    manager.setSessionModel(id, { provider: "zhipu", model: "glm-5.3", isOverride: true });
    manager.setSessionMode(id, { mode: "yolo", isOverride: true });
    manager.setStatus(id, "awaiting_approval");
    manager.setContextWindow(1_000);
    const store = useSessionManager.getState().entries[id].store;
    store.getState().setContextTokens(500);
    store.getState().setCachedTokens(300);
    store
      .getState()
      .setSessionCacheTotals({ cached_tokens: 750, prompt_tokens: 1_000, cache_write_tokens: 0 });

    render(<StatusBar client={{} as DaemonClient} />);

    expect(screen.queryByRole("button", { name: /permission mode|switch model/i })).toBeNull();
    expect(screen.queryByText("glm-5.3")).not.toBeInTheDocument();
    expect(screen.queryByText("yolo")).not.toBeInTheDocument();
    expect(screen.getByText("online")).toBeInTheDocument();
    expect(screen.getByText("1 approval")).toBeInTheDocument();
    expect(screen.getByText("/repo/worktree-a")).toBeInTheDocument();
    expect(screen.getByTitle("context 500 / 1,000 tokens")).toBeInTheDocument();
    expect(screen.getByText("cache 75%")).toBeInTheDocument();
  });
});
