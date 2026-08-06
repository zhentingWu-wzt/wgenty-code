import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { toast } from "sonner";
import { LeftSidebar } from "./LeftSidebar";
import { useSessionManager } from "../../state/sessionManager";
import { useUiStore } from "../../state/uiStore";
import { DaemonClient } from "../../api/client";

vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }));

const client = new DaemonClient();

const MAIN_PROJECT = {
  path: "/repo/wgenty-code",
  name: "wgenty-code",
  is_main: true,
  is_git_repo: true,
  added_at: "2026-08-01",
};

const ADDED_PROJECT = {
  path: "/repo/docs",
  name: "docs",
  is_main: false,
  is_git_repo: false,
  added_at: "2026-08-02",
};

function stubFetch() {
  return vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    const method = init?.method ?? "GET";
    const json = (p: unknown, status = 200) => new Response(JSON.stringify(p), { status });
    if (url === "/api/v1/projects" && method === "GET") return json([MAIN_PROJECT]);
    if (url === "/api/v1/projects" && method === "POST") return json(ADDED_PROJECT, 201);
    if (url.startsWith("/api/v1/worktrees")) {
      return json([{ path: "/repo/wgenty-code", head: "a", branch: "main", is_main: true }]);
    }
    return new Response("not found", { status: 404 });
  });
}

describe("LeftSidebar", () => {
  beforeEach(() => {
    useSessionManager.setState({ entries: {}, order: [], activeId: null });
    useUiStore.setState({ leftCollapsed: false });
    vi.stubGlobal("fetch", stubFetch());
    vi.clearAllMocks();
  });
  afterEach(() => vi.unstubAllGlobals());

  it("renders the projects header with an add-project button", async () => {
    render(<LeftSidebar client={client} />);
    expect(screen.getByText("Projects")).toBeInTheDocument();
    expect(await screen.findByRole("button", { name: "Add project" })).toBeInTheDocument();
  });

  it("add-project prompts for a path and registers it via the API", async () => {
    const spy = vi.mocked(fetch);
    vi.stubGlobal("prompt", vi.fn().mockReturnValue("/repo/docs"));
    const user = userEvent.setup();
    render(<LeftSidebar client={client} />);

    await user.click(await screen.findByRole("button", { name: "Add project" }));

    await waitFor(() => {
      const mk = spy.mock.calls.find(
        ([u, i]) => String(u) === "/api/v1/projects" && i?.method === "POST",
      );
      expect(mk).toBeDefined();
      expect(JSON.parse(mk![1]!.body as string)).toEqual({ path: "/repo/docs" });
      expect(toast.success).toHaveBeenCalledWith("Project docs added");
    });
  });

  it("add-project surfaces the daemon's error text on failure", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = String(input);
        const method = init?.method ?? "GET";
        const json = (p: unknown, status = 200) => new Response(JSON.stringify(p), { status });
        if (url === "/api/v1/projects" && method === "GET") return json([MAIN_PROJECT]);
        if (url === "/api/v1/projects" && method === "POST") {
          return new Response("directory does not exist", { status: 400 });
        }
        if (url.startsWith("/api/v1/worktrees")) return json([]);
        return new Response("not found", { status: 404 });
      }),
    );
    vi.stubGlobal("prompt", vi.fn().mockReturnValue("/nope"));
    const user = userEvent.setup();
    render(<LeftSidebar client={client} />);

    await user.click(await screen.findByRole("button", { name: "Add project" }));

    await waitFor(() => {
      expect(toast.error).toHaveBeenCalledWith(
        expect.stringMatching(/directory does not exist/),
      );
    });
  });
});
