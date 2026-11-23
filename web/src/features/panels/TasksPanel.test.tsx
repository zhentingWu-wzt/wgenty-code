import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { TasksPanel } from "./TasksPanel";
import type { DaemonClient } from "../../api/client";

const fakeClient = {
  getTodos: vi.fn().mockResolvedValue({
    items: [{ content: "Write tests", status: "in_progress", active_form: "" }],
    has_open_items: true,
    display: "",
  }),
  listTasks: vi.fn().mockResolvedValue({
    tasks: [{ id: "t1", subject: "Ship redesign", status: "pending", priority: "high",
      description: "", created_at: "", updated_at: "", tags: [] }],
  }),
} as unknown as DaemonClient;

describe("TasksPanel", () => {
  it("renders the plan (todos) always, task board collapsed by default", async () => {
    render(<TasksPanel client={fakeClient} />);
    expect(await screen.findByText("Write tests")).toBeInTheDocument();
    expect(screen.queryByText("Ship redesign")).not.toBeInTheDocument();
  });

  it("expands the task board on demand", async () => {
    render(<TasksPanel client={fakeClient} />);
    await userEvent.click(await screen.findByRole("button", { name: /task board/i }));
    expect(await screen.findByText("Ship redesign")).toBeInTheDocument();
  });
});
