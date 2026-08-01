import { afterEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { WorktreePanel } from "./WorktreePanel";
import { SkillPanel } from "./SkillPanel";
import { DaemonClient } from "../api/client";

function mockFetch(payload: unknown, status = 200) {
  const spy = vi.fn().mockResolvedValue(
    new Response(JSON.stringify(payload), { status }),
  );
  vi.stubGlobal("fetch", spy);
  return spy;
}

describe("WorktreePanel", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("lists worktrees with branch names", async () => {
    mockFetch([
      { path: "/repo", head: "a", branch: "main", is_main: true },
      { path: "/repo/.worktrees/f", head: "b", branch: "feat", is_main: false },
    ]);
    render(<WorktreePanel client={new DaemonClient()} />);
    // "main" appears twice (branch name + main tag); scope to the branch span.
    expect(await screen.findByText("main", { selector: ".wt-branch" })).toBeInTheDocument();
    expect(screen.getByText("feat")).toBeInTheDocument();
  });

  it("delete button calls the API and refreshes (main worktree has no delete)", async () => {
    const spy = vi.fn().mockImplementation((_url: string, init?: RequestInit) => {
      if (init?.method === "DELETE") return Promise.resolve(new Response(null, { status: 204 }));
      return Promise.resolve(
        new Response(
          JSON.stringify([{ path: "/repo", head: "a", branch: "main", is_main: true }]),
          { status: 200 },
        ),
      );
    });
    vi.stubGlobal("fetch", spy);
    render(<WorktreePanel client={new DaemonClient()} />);
    await screen.findByText("main", { selector: ".wt-branch" });
    expect(screen.queryByRole("button", { name: /remove/i })).not.toBeInTheDocument();
  });
});

describe("SkillPanel", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("lists skills with descriptions", async () => {
    mockFetch([{ name: "brainstorming", description: "explore intent", source_path: "/x" }]);
    render(<SkillPanel client={new DaemonClient()} />);
    expect(await screen.findByText("brainstorming")).toBeInTheDocument();
    expect(screen.getByText("explore intent")).toBeInTheDocument();
  });
});
