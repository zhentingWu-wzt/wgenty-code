# Tasks: 新 turn 清树抹掉 background subagent

## 1. clear_if_idle + TurnStarted

- [x] 1.1 `src/tui/components/subagent_tree.rs`：新增 `clear_if_idle() -> bool`，仅当 `active_count() == 0` 时 `clear()` 并返回 true。
- [x] 1.2 `src/tui/components/subagent_tree.rs`：新增单元测试——有 Running 保留 / 只 Completed 清空 / 空树清空 / 混合保留 Running。
- [x] 1.3 `src/tui/app/event.rs`：`TurnStarted` 改用 `clear_if_idle()`，仅在其返回 true 时重置 `completed_at`/`subagent_focus`/`subagent_status_bar_selected`。

## 2. delta spec + 构建

- [x] 2.1 `openspec/changes/<name>/specs/subagent-status-display/spec.md`：MODIFIED "Subagent tree lifecycle across submitted prompts" 的 "New turn start clears the tree" scenario。
- [x] 2.2 `cargo build` + `cargo test --lib` 全过、无回归（513 passed，含 4 个新 clear_if_idle 测试）。
- [x] 2.3 根因消除检查：`TurnStarted` 不再无条件清树（用 `clear_if_idle`）；`clear_if_idle` 在有活动 subagent 时返回 false 保留。fix 独立编译通过（不含用户并行工作）。
