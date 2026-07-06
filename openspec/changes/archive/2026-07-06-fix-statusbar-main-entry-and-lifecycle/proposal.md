# Fix: 状态栏 main 占位 + submit 清树生命周期

## Why

`fix-statusbar-height-mouse-scroll` 归档后，用户报告状态栏 selector 两个问题：

1. **主 agent 触发 subagent 后，selector 区域不显示 main agent 占位**——状态栏只列出活动 subagent，没有 "main" 条目，无法用方向键选中 main。而 focus view 的 selector 在 index 0 有 "main" 条目（选中 + Enter 退回主聊天）。两个 selector 不一致。
2. **触发 subagent 后，再在主 agent 提交提示词，selector 消失，再也进不去 subagent 窗口**——`Submit` 事件处理器无条件 `subagent_tree.clear()`，即使当前 turn 仍在运行（新提示词只是入队）。运行中的 subagent 被从 UI 抹掉，状态栏消失，无法 Enter 进 focus view。

## Root Cause

### 问题 1：状态栏缺少 "main" 占位

`src/tui/components/subagent_status_bar.rs::render` 只遍历 `active`（Running|Pending）subagent 生成行，没有 "main" 条目。而 `src/tui/components/subagent_focus_view.rs::build_selector_lines` 在 index 0 渲染 "main"（选中 + Enter 退回主聊天）。两个 selector 模型不一致。

`event.rs:306-355` 的状态栏导航用 `active.len()` 作 wrap 长度，`selected` 直接索引 `active` 列表——没有为 "main" 预留 index 0。

### 问题 2：Submit 无条件清树

`src/tui/app/event.rs:521-527`：

```rust
AppEvent::Submit(text) => {
    self.subagent_tree.clear();
    self.completed_at.clear();
    self.subagent_focus = None;
    self.subagent_status_bar_selected = 0;
    self.submit_input(text);
}
```

`submit_input`（`input.rs:255-258`）在 `current_turn_handle.is_some()` 时只把提示词入队 `pending_inputs`，不立即开新 turn。但 `Submit` 处理器在调用 `submit_input` **之前**就清了树——于是运行中 turn 的活动 subagent 被抹掉，状态栏消失。

正确时机：清树应发生在**新 turn 真正开始时**（`TurnStarted`），而非提示词提交时。`TurnStarted`（`event.rs:733-735`）当前只设 `turn_started_at`，不清树——所以新 turn 反而沿用旧树。

`/clear` 路径（`input.rs` → `cancel_current_turn` → `TurnAborted`）也需要清树：`cancel_current_turn`（`turn.rs:113-122`）只 `handle.abort()` + 发 `TurnAborted`，不给 subagent 发 Cancelled 更新，所以 /clear 后旧 subagent 会作为 Running 滞留状态栏。

## Fix Goals

1. **状态栏 selector 增加 "main" 占位**：index 0 为 "main"，1..N 为活动 subagent；↑↓ wrap 长度 = N+1；Enter on "main" 取消状态栏焦点（与 focus view "main" 退回主聊天语义一致），Enter on subagent 开 focus view。高度 +1 容纳 "main" 行。
2. **修正清树生命周期**：`Submit` 不再清树；改在 `TurnStarted`（新 turn 刷新树）和 `TurnAborted`（覆盖 /clear 与 turn 失败）清树 + 重置 focus/selected。运行中 turn 提交新提示词时树保留，subagent 仍可见可进。
3. **delta spec**：`subagent-status-display` 的 "main 占位" 与 "Enter on main" 修改既有验收场景，创建 MODIFIED Requirements delta。

## Impact

- **代码**：
  - `src/tui/components/subagent_status_bar.rs`（render "main" 条目 + 选中态）。
  - `src/tui/app/render.rs`（`status_bar_layout_height` +1 容纳 "main" 行 + 测试更新）。
  - `src/tui/app/event.rs`（状态栏导航索引 +1 / Enter on main / 清树从 Submit 移到 TurnStarted + TurnAborted）。
- **spec**：`openspec/changes/<name>/specs/subagent-status-display/spec.md` delta（MODIFIED Requirements：状态栏显示 main + subagent、导航 wrap 含 main、Enter on main 取消焦点）。focus view selector 已有 "main"，无需 delta。
- **依赖/API**：无外部 API、无依赖变更。
- **风险**：状态栏高度 +1（max 7 行）；"main" 选中 + Enter 取消焦点是行为变更（delta 记录）。清树时机改为 TurnStarted/TurnAborted，TurnComplete 的快照逻辑不变（快照在 start_next_turn 之前完成）。
