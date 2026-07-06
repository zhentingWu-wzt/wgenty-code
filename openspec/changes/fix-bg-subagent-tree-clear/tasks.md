# Tasks: 新 turn 清树抹掉 background subagent

## 1. clear_if_idle + TurnStarted

- [ ] 1.1 `src/tui/components/subagent_tree.rs`：新增 `clear_if_idle() -> bool`，仅当 `active_count() == 0` 时 `clear()` 并返回 true。
- [ ] 1.2 `src/tui/components/subagent_tree.rs`：新增单元测试——有 Running 保留 / 只 Completed 清空 / 空树清空。
- [ ] 1.3 `src/tui/app/event.rs`：`TurnStarted` 改用 `clear_if_idle()`，仅在其返回 true 时重置 `completed_at`/`subagent_focus`/`subagent_status_bar_selected`。

## 2. delta spec + 构建

- [ ] 2.1 `openspec/changes/<name>/specs/subagent-status-display/spec.md`：MODIFIED "Subagent tree lifecycle across submitted prompts" 的 "New turn start clears the tree" scenario。
- [ ] 2.2 `cargo build` + `cargo test --lib` 全过、无回归。
- [ ] 2.3 根因消除检查：`TurnStarted` 不再无条件清树；`clear_if_idle` 在有活动 subagent 时保留。
