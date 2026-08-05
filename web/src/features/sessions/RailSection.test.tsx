import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { RailSection } from "./RailSection";

describe("RailSection", () => {
  it("shows children by default and hides them on toggle", async () => {
    const user = userEvent.setup();
    render(
      <RailSection title="Sessions">
        <div>content here</div>
      </RailSection>,
    );

    expect(screen.getByText("content here")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /sessions/i }));
    expect(screen.queryByText("content here")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /sessions/i }));
    expect(screen.getByText("content here")).toBeInTheDocument();
  });

  it("starts collapsed when defaultCollapsed is set", async () => {
    const user = userEvent.setup();
    render(
      <RailSection title="Skills" defaultCollapsed>
        <div>secret content</div>
      </RailSection>,
    );

    expect(screen.queryByText("secret content")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /skills/i }));
    expect(screen.getByText("secret content")).toBeInTheDocument();
  });

  it("hides header actions while collapsed", async () => {
    const user = userEvent.setup();
    render(
      <RailSection title="Worktrees" actions={<button>+ New</button>} defaultCollapsed>
        <div>wt</div>
      </RailSection>,
    );

    expect(screen.queryByRole("button", { name: "+ New" })).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /worktrees/i }));
    expect(screen.getByRole("button", { name: "+ New" })).toBeInTheDocument();
  });
});
