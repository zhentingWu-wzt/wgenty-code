import { afterEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { SkillPanel } from "./SkillPanel";
import { DaemonClient } from "../api/client";

function mockFetch(payload: unknown, status = 200) {
  const spy = vi.fn().mockResolvedValue(new Response(JSON.stringify(payload), { status }));
  vi.stubGlobal("fetch", spy);
  return spy;
}

describe("SkillPanel", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("is collapsed by default; expanding lists skills with descriptions", async () => {
    mockFetch([{ name: "brainstorming", description: "explore intent", source_path: "/x" }]);
    const user = userEvent.setup();
    render(<SkillPanel client={new DaemonClient()} />);

    // Collapsed by default — the skill row must not be visible yet.
    expect(screen.queryByText("brainstorming")).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /skills/i }));
    expect(await screen.findByText("brainstorming")).toBeInTheDocument();
    expect(screen.getByText("explore intent")).toBeInTheDocument();
  });
});
