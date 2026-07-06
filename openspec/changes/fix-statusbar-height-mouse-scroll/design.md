# Design: 状态栏高度裁切 + 鼠标滚轮失效

## 方案

三个问题两个根因，分两处修复。

### 修复 1：状态栏高度计入顶边框（解决问题 1 + 2）

`SubagentStatusBar::render` 保留 `Borders::TOP` 作为焦点指示（聚焦时高亮、失焦时暗淡），布局侧补足边框占用的高度。

`src/tui/app/render.rs`：

```rust
// before
let status_bar_height = self.subagent_tree.active_count().min(5) as u16;

// after
let visible_items = self.subagent_tree.active_count().min(5) as u16;
let status_bar_height = visible_items + 1; // +1 for the TOP border
```

语义：可见 subagent 行封顶 5（保持 spec「capped at 5 lines」对 subagent 行数的解读），高度 = 可见行 + 1 行顶边框。

| 活动 subagent | visible_items | status_bar_height | 可见内容行 |
|---|---|---|---|
| 1 | 1 | 2 | 1 ✓ |
| 3 | 3 | 4 | 3 ✓ |
| 5 | 5 | 6 | 5 ✓ |
| 6 | 5 | 6 | 5（第 6 项裁切，封顶行为） |

`has_status_bar = status_bar_height > 0` 改为基于 `visible_items > 0`（等价于 `active_count > 0`，保持原显隐逻辑）。实际 `status_bar_height` 在 `active_count > 0` 时 ≥ 2，所以 `> 0` 判断不变；只需确保 `has_status_bar` 仍由活动数驱动，避免 `0 + 1 = 1` 时误显空边框——因此 `has_status_bar` 显式取 `active_count > 0`。

问题 2 无需单独修复：状态栏内容可见后，`event.rs:306-355` 既有的 ↑↓ 自动激活 + `wrap_prev/wrap_next` 导航即可看到 ▶ 光标移动，用户不再误判需要 Tab。Tab 仍是 no-op，符合 spec。

### 修复 2：启用鼠标捕获（解决问题 3）

`src/cli/args.rs` `run_tui`：

- 在 `EnterAlternateScreen` 之后执行 `execute!(stdout, EnableMouseCapture)`。
- 正常退出路径：在 `LeaveAlternateScreen` 之前执行 `execute!(io::stdout(), DisableMouseCapture)`。
- panic hook：在 `LeaveAlternateScreen` 之前执行 `DisableMouseCapture`，保证崩溃也恢复终端。

`input_reader.rs:20-31` 已正确把 `ScrollUp/ScrollDown` 转为 `AppEvent::MouseScrolled(±5)`，`event.rs:450-487` 已正确处理 focus view timeline 与主聊天滚动——启用捕获后即端到端打通，无需改动这两处。

`EnableMouseCapture` / `DisableMouseCapture` 位于 `crossterm::event`（crossterm 0.28.1），与既有 `EnableBracketedPaste` 同模块。

### 不做的事

- **不改 focus view selector 高度**：`Constraint::Length(8)` + `Borders::ALL`（inner 6 行）已有滑动跟随，用户问题 1 明确指向「主 agent 窗口」即状态栏，不在范围。
- **不加 delta spec**：三处修复均回归既有 spec 要求，无验收场景变更。
- **不实现状态栏滚动跟随**：状态栏是紧凑条，封顶 5 项可接受；超过时进 focus view 查看全部，与既有设计一致。

## 验证策略

- **单元测试**：抽取高度计算为纯函数 `status_bar_height(active_count: usize) -> u16` 并断言：`1→2`、`3→4`、`5→6`、`6→6`、`0→0`（`has_status_bar` 由 `active_count > 0` 判定）。
- **编译**：`cargo build` 确认 crossterm 导入与调用无误。
- **既有测试**：`cargo test` 确认 `subagent_status_bar` / `subagent_focus_view` / `subagent_tree` 既有测试不回归。
- **手动验证**（verify 阶段）：运行 TUI，触发 subagent，确认状态栏所有活动项可见、↑↓ 即时移动 ▶、鼠标滚轮在主聊天与 focus view 均可滚动。
