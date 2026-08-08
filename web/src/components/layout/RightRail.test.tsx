import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { RightRail } from "./RightRail";
import { useUiStore } from "../../state/uiStore";
import type { DaemonClient } from "../../api/client";

const fakeClient = {
  listSkills: vi.fn().mockResolvedValue([]),
  getTodos: vi.fn().mockResolvedValue({ items: [], has_open_items: false, display: "" }),
  listTasks: vi.fn().mockResolvedValue({ tasks: [] }),
} as unknown as DaemonClient;

describe("RightRail", () => {
  beforeEach(() => {
    useUiStore.setState({ rightPanel: null });
  });

  it("renders only the activity bar when no panel is open", () => {
    render(<RightRail client={fakeClient} />);
    expect(screen.getByTitle("Skills")).toBeInTheDocument();
    expect(screen.queryByTestId("right-panel-host")).not.toBeInTheDocument();
  });

  it("opens the skills panel via its activity icon", async () => {
    render(<RightRail client={fakeClient} />);
    await userEvent.click(screen.getByTitle("Skills"));
    expect(useUiStore.getState().rightPanel).toBe("skills");
    expect(screen.getByTestId("right-panel-host")).toBeInTheDocument();
  });
});
