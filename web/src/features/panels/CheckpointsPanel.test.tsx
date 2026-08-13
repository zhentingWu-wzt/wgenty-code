import { afterEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { CheckpointsPanel } from "./CheckpointsPanel";
import { DaemonClient } from "../../api/client";

describe("CheckpointsPanel", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("lists checkpoints and undoes the selected turn", async () => {
    const spy = vi.fn().mockImplementation((_url: string, init?: RequestInit) => {
      if (init?.method === "POST") {
        return Promise.resolve(
          new Response(JSON.stringify({ restored: 2, skipped: 0, failed: 0, rewound_turns: 1 }), {
            status: 200,
          }),
        );
      }
      return Promise.resolve(
        new Response(
          JSON.stringify([
            { turn_id: "t2", created_at: 200, file_count: 1 },
            { turn_id: "t1", created_at: 100, file_count: 3 },
          ]),
          { status: 200 },
        ),
      );
    });
    vi.stubGlobal("fetch", spy);
    vi.stubGlobal("confirm", vi.fn().mockReturnValue(true));

    render(<CheckpointsPanel client={new DaemonClient()} />);
    const btn = await screen.findByRole("button", { name: /undo t2/i });
    await userEvent.setup().click(btn);

    const post = spy.mock.calls.find(([, init]) => init?.method === "POST");
    expect(JSON.parse(post![1].body)).toEqual({ turn_ids: ["t2"] });
    expect(await screen.findByText(/restored 2/i)).toBeInTheDocument();
  });
});
