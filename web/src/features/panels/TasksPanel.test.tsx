import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { TasksPanel } from "./TasksPanel";
import type { DaemonClient } from "../../api/client";

const fakeClient = {
  getTodos: vi.fn().mockResolvedValue({
    items: [{ content: "Write tests", status: "in_progress" }],
    has_open_items: true,
    display: "",
  }),
  listTasks: vi.fn().mockResolvedValue({
    tasks: [{ id: "t1", subject: "Ship redesign", status: "pending", priority: "high",
      description: "", created_at: "", updated_at: "", tags: [] }],
  }),
} as unknown as DaemonClient;

describe("TasksPanel", () => {
  it("renders todos and tasks", async () => {
    render(<TasksPanel client={fakeClient} />);
    expect(await screen.findByText("Write tests")).toBeInTheDocument();
    expect(await screen.findByText("Ship redesign")).toBeInTheDocument();
  });
});
