# Tasks: 状态栏 main 占位 + submit 清树生命周期

## 1. 状态栏 "main" 占位（问题 1）

- [x] 1.1 `src/tui/components/subagent_status_bar.rs`：`render` 在活动 subagent 前插入 "main" 条目（index 0），统一列表 `["main", ...active]`，选中态 / wrap 用 `selected % (N+1)`。
- [x] 1.2 `src/tui/app/render.rs`：`status_bar_layout_height` 改为 `active_count.min(5) + 2`（main + subagents + border），更新单元测试断言（`1→3`、`3→5`、`5→7`、`6→7`、`0→0`）。
- [x] 1.3 `src/tui/app/event.rs`：状态栏导航 wrap 长度改 `active.len() + 1`；Enter 分支 `selected == 0` → 取消焦点，`selected >= 1` → `active.get(selected - 1)` 开 focus view。
- [x] 1.4 render 测试：1.1/1.2 由 `render.rs` 的 6 个 `status_bar_layout_height` 单元测试覆盖高度；"main" 行渲染与选中态由 verify 阶段手动验收（1.1 采用 inline render，无抽取函数可单测）。`cargo test --lib` 509 passed。

## 2. 清树生命周期（问题 2）

- [x] 2.1 `src/tui/app/event.rs`：`Submit` 处理器移除 `subagent_tree.clear()` / `completed_at.clear()` / `subagent_focus = None` / `subagent_status_bar_selected = 0`，只调 `submit_input(text)`。
- [x] 2.2 `src/tui/app/event.rs`：`TurnStarted` 处理器加入清树 + 重置（新 turn 刷新）。
- [x] 2.3 `src/tui/app/event.rs`：`TurnAborted` 处理器加入清树 + 重置（覆盖 /clear 与 turn 失败）。

## 3. delta spec + 构建

- [x] 3.1 `openspec/changes/<name>/specs/subagent-status-display/spec.md`：MODIFIED Requirements（main 占位 + 导航 wrap 含 main + Enter on main 取消焦点）。
- [x] 3.2 `cargo build` + `cargo test --lib` 全过、无回归（509 passed）。
- [x] 3.3 根因消除检查：`Submit` 不再清树；`TurnStarted`/`TurnAborted` 清树；状态栏 render 含 "main"；高度 +2。hotfix 独立编译 + 509 测试通过（不含 SubagentError 并行工作）。

## 备注：并行工作

构建期间发现 `src/teams/subagent_loop.rs` / `src/tools/meta/task.rs` / `tests/refactor_e2e_test.rs` 有用户并行编辑的 SubagentError feature（结构化 subagent 错误 + partial_result）。经用户确认为其独立工作，**不属于本 hotfix**。本 hotfix 仅提交 3 个状态栏/事件文件（`subagent_status_bar.rs` / `render.rs` / `event.rs`），SubagentError 三文件保持 unstaged 由用户另行处理。我的初始 3 文件改动曾暂存于 `stash@{0}`，合并（用户 inline 版 bug #1 + 我的 bug #2）后冗余，可于归档后 drop。
