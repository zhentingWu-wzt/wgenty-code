import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { TodoPill } from "./TodoPill";
import { TodoPillShift } from "./TodoPillShift";
import { useUiStore } from "../../state/uiStore";
import type { DaemonClient } from "../../api/client";
import type { GetTodosResponse } from "../../api/types";

function todosResponse(items: GetTodosResponse["items"]): GetTodosResponse {
  return { items, has_open_items: true, display: "" };
}

const fakeClient = (items: GetTodosResponse["items"]) =>
  ({
    getTodos: vi.fn().mockResolvedValue(todosResponse(items)),
  }) as unknown as DaemonClient;

describe("TodoPill", () => {
  beforeEach(() => {
    useUiStore.setState({ todoPillExpanded: true, todoPillOverlay: false });
  });

  it("renders nothing when there are no todos", async () => {
    const client = fakeClient([]);
    render(<TodoPill client={client} />);
    await waitFor(() => expect(client.getTodos).toHaveBeenCalled());
    expect(screen.queryByTestId("todo-pill")).not.toBeInTheDocument();
    expect(useUiStore.getState().todoPillOverlay).toBe(false);
  });

  it("shows progress count and expands the plan list by default", async () => {
    render(
      <TodoPill
        client={fakeClient([
          { content: "Inspect layout", status: "completed", active_form: "" },
          { content: "Apply patch", status: "in_progress", active_form: "Patching layout" },
          { content: "Run tests", status: "pending", active_form: "" },
        ])}
      />,
    );
    expect(await screen.findByTestId("todo-pill")).toHaveTextContent("1/3");
    // 默认展开：计划清单无需点击即可见。
    expect(await screen.findByText("Patching layout")).toBeInTheDocument();
    expect(screen.getByText("Inspect layout")).toBeInTheDocument();
    expect(screen.getByText("Run tests")).toBeInTheDocument();
    // 展开且有内容 → overlay 置位（聊天内容左移避让）。
    await waitFor(() => expect(useUiStore.getState().todoPillOverlay).toBe(true));
    // 点击折叠后清单消失，overlay 复位。
    await userEvent.click(screen.getByTestId("todo-pill"));
    expect(screen.queryByTestId("todo-pill-popover")).not.toBeInTheDocument();
    expect(useUiStore.getState().todoPillOverlay).toBe(false);
  });

  it("unmount resets the overlay so the shift margin does not leak", async () => {
    const { unmount } = render(
      <TodoPill
        client={fakeClient([
          { content: "Step", status: "pending", active_form: "" },
        ])}
      />,
    );
    await waitFor(() => expect(useUiStore.getState().todoPillOverlay).toBe(true));
    unmount();
    expect(useUiStore.getState().todoPillOverlay).toBe(false);
  });
});

describe("TodoPillShift", () => {
  it("applies the shift margin only while the overlay is active", () => {
    useUiStore.setState({ todoPillOverlay: true });
    const { rerender } = render(
      <TodoPillShift>
        <div>chat</div>
      </TodoPillShift>,
    );
    expect(screen.getByTestId("todo-pill-shift").className).toContain("md:mr-[300px]");
    useUiStore.setState({ todoPillOverlay: false });
    rerender(
      <TodoPillShift>
        <div>chat</div>
      </TodoPillShift>,
    );
    expect(screen.getByTestId("todo-pill-shift").className).toContain("mr-0");
  });
});

