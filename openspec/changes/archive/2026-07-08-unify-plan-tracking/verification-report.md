# Verification Report — unify-plan-tracking

## Checks

| Check | Command | Result |
|-------|---------|--------|
| Format | `cargo fmt -- --check` | ✅ clean (no diff) |
| Lint | `cargo clippy --all-targets -- -D warnings` | ✅ zero warnings |
| Tests | `cargo test --all` | ✅ all passed (30 passed, 0 failed) |

## Changes

- **`src/tasks/todo_write.rs`** — Removed `TodoWriteTool` struct + impls. Kept `TodoState`, `TodoItem`, `SubagentTodoMeta` (used by daemon endpoint/models).
- **`src/tasks/mod.rs`** — Removed `TodoWriteTool` from `pub use`. Updated module doc.
- **`src/daemon/state.rs`** — Removed `TodoWriteTool` import, construction, and registration. `todo_state` created directly via `TodoState::default()`.
- **`src/tui/agent/core.rs`** — `used_todo`→`used_plan`, set when `update_plan` is called (both paths). Removed `TodoWrite` from `all_task` list and interception. Nag reminder now references `update_plan`.
- **`src/tui/agent/mod.rs`** — Renamed `rounds_since_todo`→`rounds_since_plan`.
- **`AGENTS.md`** — Updated "计划同步" instruction to reference only `update_plan`.

## Behavior

- The LLM's tool list no longer includes `TodoWrite`. The single task-tracking tool is `update_plan`.
- The nag reminder correctly fires after 3 rounds without an `update_plan` call (previously only `TodoWrite` reset the counter — the auto-update bug).
- Daemon `/todos` endpoint remains (returns empty list). Task panel is inert. Plan panel is the single source of truth.
