import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { DirectoryPickerModal } from "./DirectoryPickerModal";
import { DaemonClient } from "../../api/client";

const client = new DaemonClient();

// Build a listing response. Entries let the picker show sub-directories.
function listing(current: string, parent: string | null, names: [string, boolean][]) {
  return {
    current,
    parent,
    entries: names.map(([name, is_hidden]) => ({
      name,
      path: current === "/" ? `/${name}` : `${current}/${name}`,
      is_hidden,
    })),
  };
}

function stubDirs(handler: (path: string | null) => ReturnType<typeof listing>) {
  return vi.fn(async (input: RequestInfo | URL) => {
    const url = new URL(String(input), "http://x");
    const path = url.searchParams.get("path");
    if (!url.pathname.startsWith("/api/v1/fs/dirs")) {
      return new Response("not found", { status: 404 });
    }
    return new Response(JSON.stringify(handler(path)), { status: 200 });
  });
}

describe("DirectoryPickerModal", () => {
  beforeEach(() => {
    localStorage.clear();
    vi.clearAllMocks();
  });
  afterEach(() => vi.unstubAllGlobals());

  it("opens at home and lists sub-directories", async () => {
    vi.stubGlobal(
      "fetch",
      stubDirs(() =>
        listing("/home", null, [
          ["projects", false],
          [".cache", true],
        ]),
      ),
    );
    render(
      <DirectoryPickerModal open client={client} onOpenChange={() => {}} onConfirm={() => {}} />,
    );
    expect(await screen.findByText("projects")).toBeInTheDocument();
    // Hidden dirs render too, just dimmed.
    expect(screen.getByText(".cache")).toBeInTheDocument();
  });

  it("double-clicking a folder navigates into it", async () => {
    let calls = 0;
    vi.stubGlobal(
      "fetch",
      stubDirs((path) => {
        calls++;
        if (path == null || path === "" || path === "/home") {
          return listing("/home", null, [["projects", false]]);
        }
        // nested listing under /home/projects
        return listing("/home/projects", "/home", [["myapp", false]]);
      }),
    );
    const user = userEvent.setup();
    render(
      <DirectoryPickerModal open client={client} onOpenChange={() => {}} onConfirm={() => {}} />,
    );
    await screen.findByText("projects");
    await user.dblClick(screen.getByText("projects"));
    expect(await screen.findByText("myapp")).toBeInTheDocument();
    expect(calls).toBe(2);
  });

  it("confirming fires onConfirm with the selected path and remembers it", async () => {
    vi.stubGlobal(
      "fetch",
      stubDirs(() => listing("/home", null, [["workspace", false]])),
    );
    const onConfirm = vi.fn();
    const user = userEvent.setup();
    render(
      <DirectoryPickerModal open client={client} onOpenChange={() => {}} onConfirm={onConfirm} />,
    );
    await screen.findByText("workspace");
    await user.click(screen.getByText("workspace"));
    await user.click(screen.getByRole("button", { name: "Select folder" }));

    expect(onConfirm).toHaveBeenCalledWith("/home/workspace");
    // lastDir persisted to localStorage for the next session.
    expect(localStorage.getItem("wgenty.lastDir")).toBe("/home");
  });

  it("filter narrows the visible entries", async () => {
    vi.stubGlobal(
      "fetch",
      stubDirs(() =>
        listing("/home", null, [
          ["alpha", false],
          ["beta", false],
          ["gamma", false],
        ]),
      ),
    );
    const user = userEvent.setup();
    render(
      <DirectoryPickerModal open client={client} onOpenChange={() => {}} onConfirm={() => {}} />,
    );
    await screen.findByText("alpha");
    await user.type(screen.getByPlaceholderText("Filter directories…"), "alp");
    await waitFor(() => {
      expect(screen.getByText("alpha")).toBeInTheDocument();
      expect(screen.queryByText("beta")).not.toBeInTheDocument();
      expect(screen.queryByText("gamma")).not.toBeInTheDocument();
    });
  });

  it("reopens at the last-used directory (localStorage memory)", async () => {
    localStorage.setItem("wgenty.lastDir", "/home/projects");
    const seen: (string | null)[] = [];
    vi.stubGlobal(
      "fetch",
      stubDirs((path) => {
        seen.push(path);
        return listing("/home/projects", "/home", [["app", false]]);
      }),
    );
    render(
      <DirectoryPickerModal open client={client} onOpenChange={() => {}} onConfirm={() => {}} />,
    );
    await screen.findByText("app");
    expect(seen[0]).toBe("/home/projects");
  });
});
