# Tasks: 状态栏 main 占位 + submit 清树生命周期

## 1. 状态栏 "main" 占位（问题 1）

- [ ] 1.1 `src/tui/components/subagent_status_bar.rs`：`render` 在活动 subagent 前插入 "main" 条目（index 0），统一列表 `["main", ...active]`，选中态 / wrap 用 `selected % (N+1)`。
- [ ] 1.2 `src/tui/app/render.rs`：`status_bar_layout_height` 改为 `active_count.min(5) + 2`（main + subagents + border），更新单元测试断言（`1→3`、`3→5`、`5→7`、`6→7`、`0→0`）。
- [ ] 1.3 `src/tui/app/event.rs`：状态栏导航 wrap 长度改 `active.len() + 1`；Enter 分支 `selected == 0` → 取消焦点，`selected >= 1` → `active.get(selected - 1)` 开 focus view。
- [ ] 1.4 `src/tui/components/subagent_status_bar.rs`：新增 render 测试验证 "main" 行存在、选中态、subagent 偏移。

## 2. 清树生命周期（问题 2）

- [ ] 2.1 `src/tui/app/event.rs`：`Submit` 处理器移除 `subagent_tree.clear()` / `completed_at.clear()` / `subagent_focus = None` / `subagent_status_bar_selected = 0`，只调 `submit_input(text)`。
- [ ] 2.2 `src/tui/app/event.rs`：`TurnStarted` 处理器加入清树 + 重置（新 turn 刷新）。
- [ ] 2.3 `src/tui/app/event.rs`：`TurnAborted` 处理器加入清树 + 重置（覆盖 /clear 与 turn 失败）。

## 3. delta spec + 构建

- [ ] 3.1 `openspec/changes/<name>/specs/subagent-status-display/spec.md`：MODIFIED Requirements（main 占位 + 导航 wrap 含 main + Enter on main 取消焦点）。
- [ ] 3.2 `cargo build` + `cargo test --lib` 全过、无回归。
- [ ] 3.3 根因消除检查：`Submit` 不再清树；`TurnStarted`/`TurnAborted` 清树；状态栏 render 含 "main"；高度 +2。
