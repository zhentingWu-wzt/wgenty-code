import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { PreviewPanel } from "./PreviewPanel";
import type { DaemonClient } from "../../api/client";
import type { FileContent } from "../../api/types";
import type { PreviewTabMeta } from "../../state/uiStore";

function previewMeta(relPath: string, kind: "text" | "binary" = "text"): PreviewTabMeta {
  return { workspaceRoot: "/w/proj", absPath: `/w/proj/${relPath}`, relPath, kind };
}

function clientWith(impl: (path: string) => Promise<FileContent>): DaemonClient {
  return { fetchFile: impl } as unknown as DaemonClient;
}

const text = (lines: string[], size: number): FileContent => ({
  kind: "text",
  lines,
  version: { mtime_ms: 1, size },
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((res) => {
    resolve = res;
  });
  return { promise, resolve };
}

describe("PreviewPanel", () => {
  it("shows loading on path changes and ignores superseded responses", async () => {
    const first = deferred<FileContent>();
    const superseded = deferred<FileContent>();
    const current = deferred<FileContent>();
    const requests = new Map([
      ["/w/proj/first.txt", first.promise],
      ["/w/proj/superseded.txt", superseded.promise],
      ["/w/proj/current.txt", current.promise],
    ]);
    const client = clientWith((path) => requests.get(path)!);
    const { rerender } = render(<PreviewPanel client={client} meta={previewMeta("first.txt")} />);

    await act(async () => first.resolve(text(["old content"], 11)));
    expect(await screen.findByText("old content")).toBeInTheDocument();

    rerender(<PreviewPanel client={client} meta={previewMeta("superseded.txt")} />);
    expect(screen.queryByText("old content")).not.toBeInTheDocument();

    rerender(<PreviewPanel client={client} meta={previewMeta("current.txt")} />);
    await act(async () => current.resolve(text(["current content"], 15)));
    expect(await screen.findByText("current content")).toBeInTheDocument();

    await act(async () => superseded.resolve(text(["superseded content"], 18)));
    expect(screen.queryByText("superseded content")).not.toBeInTheDocument();
    expect(screen.getByText("current content")).toBeInTheDocument();
  });

  it("renders code with a line-number gutter and shiki highlighting", async () => {
    const { container } = render(
      <PreviewPanel
        client={clientWith(async () => text(["fn main() {}", "let x = 1;"], 26))}
        meta={previewMeta("src/main.rs")}
      />,
    );
    // Header: relPath + size from the version stamp.
    expect(await screen.findByText("src/main.rs")).toBeInTheDocument();
    expect(screen.getByText("26 B")).toBeInTheDocument();
    // Gutter column carries exactly one row per line ("1" and "2").
    const gutter = container.querySelector('pre[aria-hidden="true"]');
    expect(gutter?.textContent).toBe("12");
    // Highlighted once the shared highlighter singleton resolves.
    await waitFor(() => expect(container.querySelector(".shiki")).not.toBeNull());
  });

  it("renders markdown by default and toggles to source", async () => {
    const { container } = render(
      <PreviewPanel
        client={clientWith(async () => text(["# Title", "", "body"], 14))}
        meta={previewMeta("README.md")}
      />,
    );
    expect(await screen.findByText("Title")).toBeInTheDocument();
    expect(container.querySelector("h1")).not.toBeNull();

    await userEvent.click(screen.getByRole("button", { name: /源码/ }));
    expect(container.querySelector("h1")).toBeNull();
    expect(container.textContent).toContain("# Title");

    await userEvent.click(screen.getByRole("button", { name: /渲染/ }));
    await waitFor(() => expect(container.querySelector("h1")).not.toBeNull());
  });

  it("falls back to a notice for non-whitelisted binaries", async () => {
    render(
      <PreviewPanel
        client={clientWith(async () => ({
          kind: "binary-unsupported",
          version: { mtime_ms: 1, size: 4096 },
        }))}
        meta={previewMeta("bin/app.exe", "binary")}
      />,
    );
    expect(await screen.findByText("二进制文件，暂不支持预览")).toBeInTheDocument();
    // Size appears twice by design: header badge + fallback detail.
    expect(screen.getAllByText("4 KB")).toHaveLength(2);
  });

  it("shows the oversize (413) message with sizes and retries", async () => {
    const fetchFile = vi
      .fn<() => Promise<FileContent>>()
      .mockRejectedValueOnce(
        new Error("file too large to preview: 2.1 MB exceeds the 1.5 MB limit"),
      )
      .mockResolvedValueOnce(text(["ok"], 2));
    render(<PreviewPanel client={clientWith(fetchFile)} meta={previewMeta("big.txt")} />);
    expect(
      await screen.findByText(/file too large to preview: 2\.1 MB exceeds the 1\.5 MB limit/),
    ).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: /重试/ }));
    expect(await screen.findByText("ok")).toBeInTheDocument();
    expect(fetchFile).toHaveBeenCalledTimes(2);
  });

  describe("blob previews", () => {
    // jsdom implements neither — stub both and assert the revoke lifecycle.
    const createObjectURL = vi.fn(() => "blob:fake-url");
    const revokeObjectURL = vi.fn();
    let originalCreate: unknown;
    let originalRevoke: unknown;

    beforeEach(() => {
      originalCreate = URL.createObjectURL;
      originalRevoke = URL.revokeObjectURL;
      URL.createObjectURL = createObjectURL;
      URL.revokeObjectURL = revokeObjectURL;
    });
    afterEach(() => {
      // Unmount BEFORE restoring the stubs (this hook runs before the
      // setup file's global cleanup, where jsdom's missing originals would
      // make the panel's revokeObjectURL cleanup throw).
      cleanup();
      URL.createObjectURL = originalCreate as typeof URL.createObjectURL;
      URL.revokeObjectURL = originalRevoke as typeof URL.revokeObjectURL;
      vi.unstubAllGlobals();
      vi.clearAllMocks();
    });

    it("renders images from an object URL and revokes it on unmount", async () => {
      const blob = new Blob(["png"], { type: "image/png" });
      const { unmount } = render(
        <PreviewPanel
          client={clientWith(async () => ({ kind: "blob", mime: "image/png", blob }))}
          meta={previewMeta("logo.png", "binary")}
        />,
      );
      const img = await screen.findByRole("img");
      expect(img).toHaveAttribute("src", "blob:fake-url");
      expect(img).toHaveAttribute("alt", "logo.png");
      unmount();
      expect(revokeObjectURL).toHaveBeenCalledWith("blob:fake-url");
    });

    it("never shows the previous blob URL while the next source is pending", async () => {
      const microtasks: Array<() => void> = [];
      vi.stubGlobal("queueMicrotask", (callback: () => void) => microtasks.push(callback));
      createObjectURL.mockReturnValueOnce("blob:a").mockReturnValueOnce("blob:b");
      const blobA = new Blob(["a"], { type: "image/png" });
      const blobB = new Blob(["b"], { type: "image/png" });
      const client = clientWith(async (path) => ({
        kind: "blob",
        mime: "image/png",
        blob: path.endsWith("a.png") ? blobA : blobB,
      }));
      const { rerender } = render(
        <PreviewPanel client={client} meta={previewMeta("a.png", "binary")} />,
      );
      await waitFor(() => expect(createObjectURL).toHaveBeenCalledTimes(1));
      act(() => microtasks.shift()!());
      expect(screen.getByRole("img")).toHaveAttribute("src", "blob:a");

      rerender(<PreviewPanel client={client} meta={previewMeta("b.png", "binary")} />);
      await waitFor(() => expect(createObjectURL).toHaveBeenCalledTimes(2));
      expect(screen.queryByRole("img")).not.toBeInTheDocument();

      act(() => microtasks.shift()!());
      expect(screen.getByRole("img")).toHaveAttribute("src", "blob:b");
      expect(screen.getByRole("img")).toHaveAttribute("alt", "b.png");
    });

    it("ignores a queued blob URL update after unmount", async () => {
      const microtasks: Array<() => void> = [];
      vi.stubGlobal("queueMicrotask", (callback: () => void) => microtasks.push(callback));
      const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
      const blob = new Blob(["png"], { type: "image/png" });
      const { unmount } = render(
        <PreviewPanel
          client={clientWith(async () => ({ kind: "blob", mime: "image/png", blob }))}
          meta={previewMeta("logo.png", "binary")}
        />,
      );
      await waitFor(() => expect(createObjectURL).toHaveBeenCalledTimes(1));
      expect(microtasks).toHaveLength(1);

      unmount();
      expect(revokeObjectURL).toHaveBeenCalledWith("blob:fake-url");
      consoleError.mockClear();
      act(() => microtasks.shift()!());
      expect(consoleError).not.toHaveBeenCalled();
      consoleError.mockRestore();
    });

    it("renders PDFs in an iframe filling the panel", async () => {
      const blob = new Blob(["pdf"], { type: "application/pdf" });
      const { container } = render(
        <PreviewPanel
          client={clientWith(async () => ({ kind: "blob", mime: "application/pdf", blob }))}
          meta={previewMeta("doc.pdf", "binary")}
        />,
      );
      const frame = await waitFor(() => {
        const f = container.querySelector("iframe");
        expect(f).not.toBeNull();
        return f as HTMLIFrameElement;
      });
      expect(frame.getAttribute("src")).toBe("blob:fake-url");
    });
  });
});
