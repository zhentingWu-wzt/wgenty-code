import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Composer } from "./Composer";
import { SessionStoreContext } from "../../state/sessionContext";
import { createSessionStore, type SessionStore } from "../../state/sessionStore";
import type { SlashCommand } from "../../components/slashCommands";

function renderComposer(
  onSend: (t: string) => void,
  store: SessionStore = createSessionStore(),
  onCommand: (cmd: SlashCommand) => void = () => {},
  onStop: () => void = () => {},
) {
  return render(
    <SessionStoreContext.Provider value={store}>
      <Composer onSend={onSend} onCommand={onCommand} onStop={onStop} />
    </SessionStoreContext.Provider>,
  );
}

describe("Composer", () => {
  it("sends the trimmed message on Enter and clears the input", async () => {
    const onSend = vi.fn();
    const user = userEvent.setup();
    renderComposer(onSend);

    const input = screen.getByRole("textbox");
    await user.type(input, "  hello agent  ");
    await user.keyboard("{Enter}");

    expect(onSend).toHaveBeenCalledWith("hello agent");
    expect(input).toHaveValue("");
  });

  it("inserts a newline on Shift+Enter instead of sending", async () => {
    const onSend = vi.fn();
    const user = userEvent.setup();
    renderComposer(onSend);

    const input = screen.getByRole("textbox");
    await user.type(input, "line one{Shift>}{Enter}{/Shift}line two");

    expect(onSend).not.toHaveBeenCalled();
    expect(input).toHaveValue("line one\nline two");
  });

  it("does not send on Enter while an IME composition is active", async () => {
    const onSend = vi.fn();
    const user = userEvent.setup();
    renderComposer(onSend);

    const input = screen.getByRole("textbox");
    await user.type(input, "你好");
    // Simulate Enter used to confirm an IME candidate (Chinese/Japanese/Korean).
    fireEvent.keyDown(input, { key: "Enter", isComposing: true });

    expect(onSend).not.toHaveBeenCalled();
    expect(input).toHaveValue("你好");
  });

  it("disables Send when the input is empty or whitespace", async () => {
    const user = userEvent.setup();
    renderComposer(vi.fn());

    const send = screen.getByRole("button", { name: "Send" });
    expect(send).toBeDisabled();

    await user.type(screen.getByRole("textbox"), "   ");
    expect(send).toBeDisabled();
  });

  it("shows Stop instead of Send while a turn is running", () => {
    const store = createSessionStore();
    store.getState().setRunning(true);
    renderComposer(vi.fn(), store);

    expect(screen.getByRole("button", { name: "Stop" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Send" })).not.toBeInTheDocument();
    // The textarea stays enabled so the user can queue a follow-up message;
    // the running placeholder hints at this.
    expect(screen.getByRole("textbox")).not.toBeDisabled();
    expect(screen.getByRole("textbox")).toHaveAttribute(
      "placeholder",
      "Agent is working… type to queue your next message",
    );
  });

  it("calls onSend via Enter while running so the parent can queue", async () => {
    const store = createSessionStore();
    store.getState().setRunning(true);
    const onSend = vi.fn();
    const user = userEvent.setup();
    renderComposer(onSend, store);

    const input = screen.getByRole("textbox");
    // Input is enabled while running — typing + Enter queues.
    expect(input).not.toBeDisabled();
    await user.type(input, "follow up");
    await user.keyboard("{Enter}");

    expect(onSend).toHaveBeenCalledWith("follow up");
    expect(input).toHaveValue("");
  });
});

describe("Composer pending queue", () => {
  it("renders each queued message as an editable field", () => {
    const store = createSessionStore();
    store.getState().enqueueInput("first");
    store.getState().enqueueInput("second");
    renderComposer(vi.fn(), store);

    expect(screen.getByLabelText("Queued message 1")).toHaveValue("first");
    expect(screen.getByLabelText("Queued message 2")).toHaveValue("second");
  });

  it("edits a queued message in place and keeps it queued", async () => {
    const store = createSessionStore();
    store.getState().enqueueInput("draft");
    const user = userEvent.setup();
    renderComposer(vi.fn(), store);

    const field = screen.getByLabelText("Queued message 1");
    await user.clear(field);
    await user.type(field, "revised");
    expect(field).toHaveValue("revised");
    // Still queued (not sent): store reflects the edit.
    expect(store.getState().pendingInputs).toEqual(["revised"]);
  });

  it("removes a queued message via its remove button", async () => {
    const store = createSessionStore();
    store.getState().enqueueInput("keep");
    store.getState().enqueueInput("drop");
    const user = userEvent.setup();
    renderComposer(vi.fn(), store);

    await user.click(screen.getAllByLabelText("Remove queued message")[1]);
    expect(store.getState().pendingInputs).toEqual(["keep"]);
    expect(screen.queryByLabelText("Queued message 2")).not.toBeInTheDocument();
  });
});

describe("Composer slash commands", () => {
  it("shows the command menu while typing a slash prefix", async () => {
    const user = userEvent.setup();
    renderComposer(vi.fn());

    expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
    await user.type(screen.getByRole("textbox"), "/m");
    expect(screen.getByRole("listbox")).toBeInTheDocument();
    expect(screen.getByRole("option", { name: /\/model/ })).toBeInTheDocument();
    expect(screen.getByRole("option", { name: /\/memory/ })).toBeInTheDocument();
  });

  it("Enter on an exact command fires onCommand, not onSend", async () => {
    const onSend = vi.fn();
    const onCommand = vi.fn();
    const user = userEvent.setup();
    renderComposer(onSend, undefined, onCommand);

    const input = screen.getByRole("textbox");
    await user.type(input, "/model");
    await user.keyboard("{Enter}");

    expect(onCommand).toHaveBeenCalledWith(expect.objectContaining({ name: "/model" }));
    expect(onSend).not.toHaveBeenCalled();
    expect(input).toHaveValue("");
  });

  it("clicking a menu item fires onCommand and clears the input", async () => {
    const onCommand = vi.fn();
    const user = userEvent.setup();
    renderComposer(vi.fn(), undefined, onCommand);

    const input = screen.getByRole("textbox");
    await user.type(input, "/se");
    await user.click(screen.getByRole("option", { name: /\/sessions/ }));

    expect(onCommand).toHaveBeenCalledWith(expect.objectContaining({ name: "/sessions" }));
    expect(input).toHaveValue("");
  });
});
