import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { FileTree, guessFileKind, isMutedDir, joinPath, relativeTo } from "./FileTree";
import { DaemonClient } from "../../api/client";
import { useUiStore } from "../../state/uiStore";

const client = new DaemonClient();

const ROOT = "/repo/wt";

const ROOT_LIST = {
  current: ROOT,
  truncated: false,
  entries: [
    { name: "src", is_dir: true, size: 0 },
    { name: "target", is_dir: true, size: 0 },
    { name: "logo.png", is_dir: false, size: 2048 },
    { name: "main.rs", is_dir: false, size: 96 },
  ],
};

const SRC_LIST = {
  current: `${ROOT}/src`,
  truncated: false,
  entries: [{ name: "lib.rs", is_dir: false, size: 12 }],
};

/** Fetch stub keyed by the `path` query param of /api/v1/fs/entries.
 *  `gitStatus` (default none) backs /api/v1/fs/git-status responses. */
function stubFetch(listings: Record<string, unknown>, gitStatus: unknown[] = []) {
  return vi.fn(async (input: RequestInfo | URL) => {
    const url = new URL(String(input), "http://localhost");
    if (url.pathname === "/api/v1/fs/entries") {
      const dir = url.searchParams.get("path") ?? "";
      const body = listings[dir];
      if (body === undefined) return new Response("not found", { status: 404 });
      return new Response(JSON.stringify(body), { status: 200 });
    }
    if (url.pathname === "/api/v1/fs/git-status") {
      return new Response(JSON.stringify(gitStatus), { status: 200 });
    }
    return new Response("not found", { status: 404 });
  });
}

function resetUi() {
  useUiStore.setState({ openTabs: [], activeTabId: null, previewTabs: {} });
}

/** Count only /fs/entries calls on a stubbed fetch spy (git-status excluded). */
function expectEntriesCalls(spy: ReturnType<typeof stubFetch>, n: number) {
  expect(spy.mock.calls.filter((c) => String(c[0]).includes("/fs/entries"))).toHaveLength(n);
}

describe("FileTree helpers", () => {
  it("guessFileKind maps text extensions to text, others/unknown to binary", () => {
    expect(guessFileKind("main.rs")).toBe("text");
    expect(guessFileKind("App.tsx")).toBe("text");
    expect(guessFileKind("Cargo.toml")).toBe("text");
    expect(guessFileKind("Makefile")).toBe("text"); // extensionless text filename
    expect(guessFileKind("logo.png")).toBe("binary");
    expect(guessFileKind("app.exe")).toBe("binary");
    expect(guessFileKind("unknownblob")).toBe("binary");
  });

  it("isMutedDir matches exactly target/node_modules/dist", () => {
    expect(isMutedDir("target")).toBe(true);
    expect(isMutedDir("node_modules")).toBe(true);
    expect(isMutedDir("dist")).toBe(true);
    expect(isMutedDir("src")).toBe(false);
    expect(isMutedDir("distribute")).toBe(false); // prefix must not match
  });

  it("joinPath avoids doubling the slash at filesystem roots", () => {
    expect(joinPath("/repo/wt", "main.rs")).toBe("/repo/wt/main.rs");
    expect(joinPath("/", "etc")).toBe("/etc");
  });

  it("relativeTo strips the workspace root prefix", () => {
    expect(relativeTo(ROOT, `${ROOT}/src/main.rs`)).toBe("src/main.rs");
    expect(relativeTo(ROOT, ROOT)).toBe("");
    expect(relativeTo("/", "/etc/hosts")).toBe("etc/hosts");
    expect(relativeTo(ROOT, "/elsewhere/x.txt")).toBe("/elsewhere/x.txt"); // defensive
  });
});

describe("FileTree", () => {
  beforeEach(resetUi);
  afterEach(() => vi.unstubAllGlobals());

  it("fetches the root listing on mount and renders dirs and files", async () => {
    const spy = stubFetch({ [ROOT]: ROOT_LIST });
    vi.stubGlobal("fetch", spy);
    render(<FileTree workspaceRoot={ROOT} client={client} />);

    expect(await screen.findByText("main.rs")).toBeInTheDocument();
    expect(screen.getByText("logo.png")).toBeInTheDocument();
    expect(screen.getByText("src")).toBeInTheDocument();
    // Only the root was fetched — sub-dirs wait for their first expand.
    // (The mount also fires one /fs/git-status; filter it out.)
    const entriesCalls = spy.mock.calls.filter((c) => String(c[0]).includes("/fs/entries"));
    expect(entriesCalls).toHaveLength(1);
    expect(String(entriesCalls[0][0])).toContain(
      `/api/v1/fs/entries?path=${encodeURIComponent(ROOT)}`,
    );
  });

  it("lazy-loads a sub-directory on first expand, then serves from cache", async () => {
    const spy = stubFetch({ [ROOT]: ROOT_LIST, [`${ROOT}/src`]: SRC_LIST });
    vi.stubGlobal("fetch", spy);
    const user = userEvent.setup();
    render(<FileTree workspaceRoot={ROOT} client={client} />);

    await user.click(await screen.findByText("src"));
    expect(await screen.findByText("lib.rs")).toBeInTheDocument();
    expectEntriesCalls(spy, 2);

    // Collapse and re-expand: served from the per-dir cache, no refetch.
    await user.click(screen.getByText("src"));
    expect(screen.queryByText("lib.rs")).toBeNull();
    await user.click(screen.getByText("src"));
    expect(await screen.findByText("lib.rs")).toBeInTheDocument();
    expectEntriesCalls(spy, 2);
  });

  it("greys out target/node_modules/dist and keeps them collapsed", async () => {
    const spy = stubFetch({ [ROOT]: ROOT_LIST });
    vi.stubGlobal("fetch", spy);
    render(<FileTree workspaceRoot={ROOT} client={client} />);

    const muted = await screen.findByText("target");
    expect(muted.className).toContain("text-muted-foreground");
    expect(screen.getByText("src").className).not.toContain("text-muted-foreground");
    // Muted dirs are collapsed by default — nothing was fetched for them.
    for (const call of spy.mock.calls) expect(String(call[0])).not.toContain("/target");
  });

  it("shows the truncation hint when the listing is truncated", async () => {
    vi.stubGlobal(
      "fetch",
      stubFetch({
        [ROOT]: {
          current: ROOT,
          truncated: true,
          entries: [{ name: "a.txt", is_dir: false, size: 1 }],
        },
      }),
    );
    render(<FileTree workspaceRoot={ROOT} client={client} />);

    expect(await screen.findByText("已截断，仅显示前 2000 项")).toBeInTheDocument();
  });

  it("opens a preview tab (idempotent id) with abs/rel paths and kind on file click", async () => {
    vi.stubGlobal("fetch", stubFetch({ [ROOT]: ROOT_LIST }));
    const user = userEvent.setup();
    render(<FileTree workspaceRoot={ROOT} client={client} />);

    await user.click(await screen.findByText("main.rs"));
    const tabId = `preview:${ROOT}/main.rs`;
    const s = useUiStore.getState();
    expect(s.openTabs).toContain(tabId);
    expect(s.activeTabId).toBe(tabId);
    expect(s.previewTabs[tabId]).toEqual({
      workspaceRoot: ROOT,
      absPath: `${ROOT}/main.rs`,
      relPath: "main.rs",
      kind: "text",
    });

    // Non-text extension guesses binary.
    await user.click(screen.getByText("logo.png"));
    expect(useUiStore.getState().previewTabs[`preview:${ROOT}/logo.png`]).toMatchObject({
      relPath: "logo.png",
      kind: "binary",
    });
  });

  it("derives preview meta paths from the daemon-canonical root", async () => {
    // e.g. macOS: requested /repo/wt resolves to /private/repo/wt.
    vi.stubGlobal(
      "fetch",
      stubFetch({
        [ROOT]: { current: "/private/repo/wt", truncated: false, entries: ROOT_LIST.entries },
      }),
    );
    const user = userEvent.setup();
    render(<FileTree workspaceRoot={ROOT} client={client} />);

    await user.click(await screen.findByText("main.rs"));
    expect(useUiStore.getState().previewTabs["preview:/private/repo/wt/main.rs"]).toEqual({
      workspaceRoot: "/private/repo/wt",
      absPath: "/private/repo/wt/main.rs",
      relPath: "main.rs",
      kind: "text",
    });
  });

  it("colors changed files by git status and shows deleted files at their parent", async () => {
    vi.stubGlobal(
      "fetch",
      stubFetch({ [ROOT]: ROOT_LIST, [`${ROOT}/src`]: SRC_LIST }, [
        { path: "main.rs", status: "modified" },
        { path: "logo.png", status: "added" },
        { path: "old.txt", status: "deleted" },
        { path: "src/lib.rs", status: "modified" },
      ]),
    );
    const user = userEvent.setup();
    render(<FileTree workspaceRoot={ROOT} client={client} />);

    expect((await screen.findByText("main.rs")).className).toContain("text-warning");
    expect(screen.getByText("logo.png").className).toContain("text-success");

    // Deleted file: strike-through red row even though it is absent from the
    // on-disk listing (ROOT_LIST has no old.txt).
    const old = screen.getByText("old.txt");
    expect(old.className).toContain("line-through");
    expect(old.parentElement?.className).toContain("text-danger");

    // Directory aggregates the strongest change beneath it.
    const srcDir = screen.getByText("src");
    expect(srcDir.className).toContain("text-warning");
    await user.click(srcDir);
    expect(screen.getByText("lib.rs").className).toContain("text-warning");
  });

  it("offers a retry row when the listing fails", async () => {
    const spy = stubFetch({}); // root 404s
    vi.stubGlobal("fetch", spy);
    const user = userEvent.setup();
    render(<FileTree workspaceRoot={ROOT} client={client} />);

    await user.click(await screen.findByText(/加载失败/));
    // Retry hits the same path again. (The mount also fires one
    // /fs/git-status — filter the spy down to /fs/entries calls.)
    await waitFor(() => expectEntriesCalls(spy, 2));
  });
});
