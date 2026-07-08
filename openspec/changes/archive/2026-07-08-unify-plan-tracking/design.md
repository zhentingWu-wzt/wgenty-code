## Context

Two tools track agent task progress with the same status model:

- `TodoWrite` (`src/tasks/todo_write.rs`) — a real `Tool` with `TodoState`, registered in the daemon tool registry (`src/daemon/state.rs:88`). The TUI agent loop intercepts it to set `used_todo = true` (`core.rs:161,326`), then lets it execute normally. After each round, `rounds_since_todo` is reset to 0 if `used_todo`, else incremented; at `>= 3` a nag reminder is appended to the last tool result (`core.rs:417-433`).

- `update_plan` (`src/tools/meta/update_plan.rs`) — a stub `Tool` whose `execute()` returns an error. The TUI agent loop intercepts it *before* execution (`core.rs:170,288`), sends `AppEvent::PlanUpdate(args)` to update `PlanPanelState`, and pushes a success tool result. It does **not** set `used_todo`, so the nag counter keeps incrementing.

`AGENTS.md` explicitly tells the agent to use both and keep them in sync — a redundancy that doubles tool calls.

## Goals / Non-Goals

**Goals:**
- Remove `TodoWrite` as an LLM-callable tool so the agent has one tracking mechanism.
- Make the nag reminder fire based on `update_plan` usage, not `TodoWrite`.
- Keep `TodoState`/`TodoItem`/`SubagentTodoMeta` types (daemon endpoint/models depend on them).

**Non-Goals:**
- Remove the daemon `/todos` endpoint or `TodoState` type (kept for API compatibility; returns empty).
- Remove the TUI task panel (`task_panel.rs`) or `tui::client::TodoItem` (inert but harmless; future cleanup).
- Merge `PlanPanelState` with `TodoState` (separate concerns: plan panel is UI display, todo state is daemon API).

## Decisions

### D1: Remove `TodoWriteTool` struct entirely, keep `TodoState`/`TodoItem` types
**Choice:** Delete the `TodoWriteTool` struct, its `impl Default`, `impl TodoWriteTool`, and `impl Tool` from `todo_write.rs`. Keep `TodoState`, `TodoItem`, `SubagentTodoMeta` (used by daemon). Remove `TodoWriteTool` from `pub use` in `tasks/mod.rs` and from the daemon tool registry.
**Rationale:** Keeping the struct but not registering it would trigger `dead_code` warnings under `-D warnings`. The types stay because the daemon `/todos` endpoint and `daemon/models.rs` depend on them.

### D2: Replace `used_todo`/`rounds_since_todo` with `used_plan`/`rounds_since_plan`
**Choice:** In `core.rs`, set `used_plan = true` when `update_plan` is called (both parallel and sequential paths). Rename the field in `mod.rs` and the nag logic. Change the reminder text to reference `update_plan`.
**Rationale:** This is the root-cause fix for the auto-update bug. The nag mechanism now correctly tracks the single remaining tool.

### D3: Create `todo_state` independently in daemon
**Choice:** In `daemon/state.rs`, replace `let todo_write = TodoWriteTool::new(); let todo_state = todo_write.todo_state();` with `let todo_state = Arc::new(RwLock::new(TodoState::default()));`. Remove `registry.register(Box::new(todo_write))`.
**Rationale:** The daemon `/todos` endpoint reads `todo_state` — keeping it (empty) avoids breaking the API. Creating it directly from `TodoState::default()` is cleaner than keeping a dead tool instance.

## Risks / Trade-offs

- [Task panel becomes empty] → **Accepted.** The plan panel (`PlanPanelState`) is the single source of truth. The task panel was already redundant when both tools were used. Future cleanup can remove it.
- [Daemon `/todos` returns empty] → **Accepted.** No tool updates `TodoState` anymore. The endpoint remains for API compatibility.
- [`TodoWrite` in conversation history] → If a resumed session has a `TodoWrite` tool call in history, the LLM might try to call it again. Since it's not in the tool list, the API will reject it. The TUI interception code for `TodoWrite` is removed, so the call would fall through to normal execution → daemon returns "tool not found". This is acceptable degradation.
