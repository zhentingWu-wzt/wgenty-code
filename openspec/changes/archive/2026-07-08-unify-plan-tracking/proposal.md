# Unify task tracking: remove TodoWrite, keep update_plan

## Why

The codebase has **two redundant task-tracking tools** that do essentially the same thing:

| | `TodoWrite` | `update_plan` |
|---|---|---|
| **State** | Real in-memory `TodoState` (`Arc<RwLock<...>>`) | `PlanPanelState` in the App (via `AppEvent::PlanUpdate`) |
| **Execution** | `TodoWriteTool::execute()` updates state, returns rendered list | Intercepted in `core.rs` before execute; sends event to App |
| **Item model** | `content` + `status` + `activeForm` + `subagent` meta | `step` + `status` |
| **Status model** | pending / in_progress / completed | pending / in_progress / completed |
| **Replace model** | Batch replace entire list | Batch replace entire list |
| **UI panel** | Task panel (`task_panel.rs`) | Plan panel (`plan_panel.rs`) |
| **Nag reminder** | Yes — `rounds_since_todo >= 3` injects reminder | **No** — `update_plan` does not reset `rounds_since_todo` |

**The redundancy is explicit**: `AGENTS.md` instructs the agent to call *both* tools and keep them in sync ("使用 TodoWrite 更新任务状态后，同步调用 update_plan 更新 UI 面板，保持两端状态一致"). This doubles tool calls and creates a sync burden.

**The auto-update bug**: When the agent uses `update_plan` (but not `TodoWrite`), the `rounds_since_todo` counter keeps incrementing because only `TodoWrite` sets `used_todo = true`. After 3 rounds, a stale nag reminder fires: `"<reminder>Update your todos with TodoWrite.</reminder>"` — even though the agent is already tracking progress via `update_plan`. The plan's status doesn't auto-update the todo-tracking state.

## What Changes

1. **Remove `TodoWriteTool`** from the tool registry and the codebase. The LLM no longer sees or calls `TodoWrite`. The `TodoState` / `TodoItem` / `SubagentTodoMeta` types stay (used by the daemon `/todos` endpoint and daemon models).
2. **Fix the nag reminder** to track `update_plan` instead of `TodoWrite`: `used_todo` → `used_plan`, `rounds_since_todo` → `rounds_since_plan`. The reminder text changes from "Update your todos with TodoWrite" to "Update your plan with update_plan".
3. **Remove `TodoWrite` from the `all_task` parallel-execution list** in `core.rs`.
4. **Update `AGENTS.md`** to remove the dual-tool sync instruction.

## Impact

- **Behavior**: The LLM has a single task-tracking tool (`update_plan`). The nag reminder correctly fires when the agent forgets to call `update_plan` for 3+ rounds.
- **Compatibility**: The daemon `/todos` endpoint still exists (returns empty list since no tool updates `TodoState`). The task panel in the TUI becomes inert (empty) — the plan panel is the single source of truth.
- **Surface area**: 5 code files + 1 docs file. No new dependencies, no schema changes.
