import { afterEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { SkillsPanel } from "./SkillsPanel";
import { DaemonClient } from "../../api/client";

function mockFetch(payload: unknown, status = 200) {
  // Fresh Response per call: DaemonClient.authedFetch probes /__daemon-info
  // first, and a single shared Response body cannot be read twice.
  const spy = vi
    .fn()
    .mockImplementation(async () => new Response(JSON.stringify(payload), { status }));
  vi.stubGlobal("fetch", spy);
  return spy;
}

describe("SkillsPanel", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("lists skills with descriptions", async () => {
    mockFetch([{ name: "brainstorming", description: "explore intent", source_path: "/x" }]);
    render(<SkillsPanel client={new DaemonClient()} />);

    expect(await screen.findByText("brainstorming")).toBeInTheDocument();
    expect(screen.getByText("explore intent")).toBeInTheDocument();
  });
});
