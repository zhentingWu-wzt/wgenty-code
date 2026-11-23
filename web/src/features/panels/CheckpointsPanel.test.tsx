import { afterEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { CheckpointsPanel } from "./CheckpointsPanel";
import { DaemonClient } from "../../api/client";

describe("CheckpointsPanel", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("previews the diff, then undoes only after confirmation", async () => {
    const spy = vi.fn().mockImplementation((url: string, init?: RequestInit) => {
      if (init?.method === "POST") {
        return Promise.resolve(
          new Response(JSON.stringify({ restored: 2, skipped: 0, failed: 0, rewound_turns: 1 }), {
            status: 200,
          }),
        );
      }
      if (url.endsWith("/diff")) {
        return Promise.resolve(
          new Response(
            JSON.stringify({
              turn_id: "t2",
              files: [
                { path: "a.ts", state: "saved", diff: "--- a/a.ts\n+++ b/a.ts\n@@ -1 +1 @@\n-old\n+new\n", added: 1, removed: 1, will_delete: false, skipped: false },
                { path: "b.ts", state: "tombstone", diff: "", added: 0, removed: 0, will_delete: true, skipped: false },
                { path: "img.png", state: "skipped", diff: "", added: 0, removed: 0, will_delete: false, skipped: true },
              ],
            }),
            { status: 200 },
          ),
        );
      }
      return Promise.resolve(
        new Response(
          JSON.stringify([
            {
              turn_id: "t2",
              created_at: 200,
              file_count: 3,
              user_summary: "Fix the parser",
              files: [{ path: "a.ts", state: "saved" }],
            },
            { turn_id: "t1", created_at: 100, file_count: 3 },
          ]),
          { status: 200 },
        ),
      );
    });
    vi.stubGlobal("fetch", spy);

    render(<CheckpointsPanel client={new DaemonClient()} />);

    // Listing shows the summary + file names, not bare UUIDs.
    expect(await screen.findByText("Fix the parser")).toBeInTheDocument();
    expect(screen.getByText("a.ts")).toBeInTheDocument();

    // Step 1: click Undo → fetches diff preview, no POST yet.
    const btn = (await screen.findAllByRole("button", { name: "Undo" }))[0];
    await userEvent.setup().click(btn);
    expect(
      await screen.findByText(/undo preview/i),
    ).toBeInTheDocument();
    expect(screen.getByText(/Will delete/)).toBeInTheDocument();
    expect(screen.getByText(/Not restored/)).toBeInTheDocument();

    // Step 2: confirm → the POST goes out with the right turn id.
    await userEvent.setup().click(screen.getByRole("button", { name: /undo 3 file changes/i }));
    const post = spy.mock.calls.find(([, init]) => init?.method === "POST");
    expect(JSON.parse(post![1].body)).toEqual({ turn_ids: ["t2"] });
    expect(await screen.findByText(/restored 2/i)).toBeInTheDocument();
  });
});
