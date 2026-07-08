## 1. Remove TodoWriteTool from the codebase

- [x] 1.1 In `src/tasks/todo_write.rs`, removed `TodoWriteTool` struct, `impl Default`, `impl TodoWriteTool` (new/from_arc/todo_state), and `impl Tool for TodoWriteTool`. Removed now-unused imports (`Tool`, `ToolError`, `ToolOutput`, `async_trait`, `Arc`, `RwLock`). Kept `TodoState`, `TodoItem`, `SubagentTodoMeta` and their impls.
- [x] 1.2 In `src/tasks/mod.rs`, removed `TodoWriteTool` from the `pub use` line. Updated module doc comment.
- [x] 1.3 In `src/daemon/state.rs`, removed `TodoWriteTool` from the import, replaced construction with `Arc::new(RwLock::new(TodoState::default()))`, and removed `registry.register(Box::new(todo_write))`.

## 2. Fix nag reminder to track update_plan

- [x] 2.1 In `src/tui/agent/core.rs`, renamed `used_todo` → `used_plan`. Set `used_plan = true` in the `update_plan` interception (both parallel path and sequential path). Removed the `TodoWrite` interception from both paths.
- [x] 2.2 In `src/tui/agent/core.rs`, removed `"TodoWrite"` from the `all_task` parallel-execution match list.
- [x] 2.3 In `src/tui/agent/core.rs`, updated the nag logic: `rounds_since_todo` → `rounds_since_plan`, changed reminder text to "Update your plan with update_plan".
- [x] 2.4 In `src/tui/agent/mod.rs`, renamed the field `rounds_since_todo` → `rounds_since_plan` (declaration + init).

## 3. Update docs

- [x] 3.1 In `AGENTS.md`, updated the "计划同步" instruction to reference only `update_plan`.

## 4. Verify

- [x] 4.1 `cargo fmt` — no diff.
- [x] 4.2 `cargo clippy --all-targets -- -D warnings` — zero warnings.
- [x] 4.3 `cargo test --all` — all green (30 passed, 0 failed).
