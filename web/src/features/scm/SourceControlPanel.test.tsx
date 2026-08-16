import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { SourceControlPanel } from "./SourceControlPanel";
import { DiffView } from "./DiffView";
import { DaemonClient } from "../../api/client";
import { useSessionManager } from "../../state/sessionManager";
import { useUiStore } from "../../state/uiStore";

const client = new DaemonClient();
const ROOT = "/repo/wt";

function stubApi(gitStatus: unknown[], diff: unknown) {
  return vi.fn(async (input: RequestInfo | URL) => {
    const url = new URL(String(input), "http://localhost");
    if (url.pathname === "/api/v1/projects") {
      return new Response(
        JSON.stringify([{ path: ROOT, name: "wt", is_main: true, is_git_repo: true }]),
        { status: 200 },
      );
    }
    if (url.pathname === "/api/v1/fs/git-status") {
      return new Response(JSON.stringify(gitStatus), { status: 200 });
    }
    if (url.pathname === "/api/v1/fs/git-diff") {
      return new Response(JSON.stringify(diff), { status: 200 });
    }
    return new Response("nope", { status: 404 });
  });
}

const STATUS = [
  { path: "src/a.ts", status: "modified" },
  { path: "new.txt", status: "added" },
  { path: "old.rs", status: "deleted" },
];

const DIFF = {
  status: "modified",
  truncated: false,
  lines: [
    { kind: "context", old_no: 1, new_no: 1, text: "fn main() {" },
    { kind: "delete", old_no: 2, text: '    println!("old");' },
    { kind: "add", new_no: 2, text: '    println!("new");' },
    { kind: "context", old_no: 3, new_no: 3, text: "}" },
  ],
};

describe("SourceControlPanel", () => {
  beforeEach(() => {
    useUiStore.setState({ openTabs: [], activeTabId: null, diffTabs: {} });
    const m = useSessionManager.getState();
    // No sessions: the hook falls back to the (stubbed) main project.
    for (const e of Object.values(m.entries)) m.removeSession(e.id);
  });
  afterEach(() => vi.unstubAllGlobals());

  it("lists changed files with M/A/D badges", async () => {
    vi.stubGlobal("fetch", stubApi(STATUS, DIFF));
    render(<SourceControlPanel client={client} />);

    expect(await screen.findByText("a.ts")).toBeInTheDocument();
    expect(screen.getByText("3 项变更")).toBeInTheDocument();
    expect(screen.getByTitle("src/a.ts").textContent).toContain("M");
    expect(screen.getByTitle("new.txt").textContent).toContain("A");
    expect(screen.getByTitle("old.rs").textContent).toContain("D");
    // Dir hints next to names.
    expect(screen.getByTitle("src/a.ts").textContent).toContain("src");
  });

  it("opens a diff tab with abs/rel paths on file click", async () => {
    vi.stubGlobal("fetch", stubApi(STATUS, DIFF));
    const user = userEvent.setup();
    render(<SourceControlPanel client={client} />);

    await user.click(await screen.findByText("a.ts"));
    const tabId = "diff:/repo/wt/src/a.ts";
    const s = useUiStore.getState();
    expect(s.openTabs).toContain(tabId);
    expect(s.activeTabId).toBe(tabId);
    expect(s.diffTabs[tabId]).toEqual({
      workspaceRoot: ROOT,
      absPath: `${ROOT}/src/a.ts`,
      relPath: "src/a.ts",
      status: "modified",
    });
  });

  it("shows the empty state when nothing changed", async () => {
    vi.stubGlobal("fetch", stubApi([], DIFF));
    render(<SourceControlPanel client={client} />);
    expect(await screen.findByText("没有变更")).toBeInTheDocument();
  });
});

describe("DiffView", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("renders context/add/delete rows with colors, gutters, and signs", async () => {
    vi.stubGlobal("fetch", stubApi(STATUS, DIFF));
    render(
      <DiffView
        client={client}
        meta={{
          workspaceRoot: ROOT,
          absPath: `${ROOT}/src/a.ts`,
          relPath: "src/a.ts",
          status: "modified",
        }}
      />,
    );

    // Header counts: +1 / −1.
    expect(await screen.findByText("+1")).toBeInTheDocument();
    expect(screen.getByText("−1")).toBeInTheDocument();

    const old = await screen.findByText('println!("old");');
    expect(old.parentElement?.className).toContain("bg-danger/15");
    const added = screen.getByText('println!("new");');
    expect(added.parentElement?.className).toContain("bg-success/15");

    // Context rows carry both line numbers and no background tint.
    const ctx = screen.getByText("fn main() {");
    expect(ctx.parentElement?.className).not.toContain("bg-danger/15");
    expect(ctx.parentElement?.className).not.toContain("bg-success/15");

    // Gutter numbers: old_no on the delete row, new_no on the add row.
    expect(old.parentElement?.textContent).toContain("2");
  });
});
