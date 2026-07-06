# Fix: 状态栏高度裁切 + 鼠标滚轮失效

## Why

`fix-subagent-focus-nav` 归档后，主 agent 窗口的 subagent 状态栏与聊天区出现三个回归 / 未完成的交互问题：

1. **主 agent 窗口看不到 subagent selector 选择项**——状态栏高度不足，活动 subagent 行被裁切。
2. **似乎仍需按 Tab 才能焦点到 selector**——但代码与 spec 都规定 ↑↓ 自动激活、Tab 为 no-op；实际是问题 1 让 ↑↓ 的效果不可见，用户误判为「需要 Tab」。
3. **聊天区鼠标滚动失效**（主 agent 窗口与 subagent focus view 均失效）——终端从未启用鼠标捕获，crossterm 不投递 `Event::Mouse`，`AppEvent::MouseScrolled` 永不触发。

## Root Cause

### 问题 1 + 2：状态栏高度未计入顶边框

`src/tui/app/render.rs:48`:

```rust
let status_bar_height = self.subagent_tree.active_count().min(5) as u16;
```

`SubagentStatusBar::render` 使用 `Block::default().borders(Borders::TOP)`，顶边框占用 1 行，`block.inner(area)` 的高度为 `area.height - 1`。布局只分配 `active_count` 行（封顶 5），因此：

| 活动 subagent | 分配高度 | 边框 | 可见内容行 | 结果 |
|---|---|---|---|---|
| 1 | 1 | 1 | 0 | 整行被边框吃掉，**0 项可见** |
| 2 | 2 | 1 | 1 | 仅 1/2 可见 |
| N (≤5) | N | 1 | N-1 | **最后一项被裁切** |

spec `subagent-status-display` 要求「occupying the minimum height needed to display all active subagents」——当前实现违反该要求。

问题 2 是问题 1 的表象：`event.rs:306-355` 的 ↑↓ 自动激活逻辑正确（按下即 `subagent_status_bar_focused = true` 并 `wrap_prev/wrap_next` 导航），但因状态栏内容被裁切到 0 行（1 个活动 subagent 时），用户看不到 ▶ 光标移动，误以为 ↑↓ 无效、需要 Tab。Tab 在代码中是 no-op（`event.rs:341-346`），spec 也明确规定「Tab SHALL have no effect」。

### 问题 3：鼠标捕获从未启用

`src/cli/args.rs` 的终端初始化只执行 `EnterAlternateScreen` + `enable_raw_mode` + `EnableBracketedPaste`，**从未调用 `EnableMouseCapture`**。全仓库搜索 `EnableMouseCapture` / `DisableMouseCapture` 均无结果。因此 crossterm 不投递 `Event::Mouse`，`input_reader.rs:20-31` 的 `MouseEventKind::ScrollUp/ScrollDown` 分支永不命中，`AppEvent::MouseScrolled` 永不发送。

`event.rs:450-487` 的 `MouseScrolled` 处理逻辑本身正确（focus view timeline 与主聊天都覆盖），只是没有事件可处理。

spec `subagent-focus-view` 第 27 / 41-43 行明确要求 timeline「scrollable only via mouse wheel」——当前实现违反该要求。

## Fix Goals

1. 状态栏高度计入顶边框：`active_count` 行内容 + 1 行边框，使所有活动 subagent 可见（≤5 项时全部可见，>5 项封顶 5 项 + 边框）。
2. 随问题 1 修复，↑↓ 自动激活与导航的视觉效果恢复，用户不再误判需要 Tab。
3. 在终端初始化启用 `EnableMouseCapture`，在正常退出与 panic hook 中配对 `DisableMouseCapture`，恢复主聊天与 focus view timeline 的鼠标滚轮滚动。

## Impact

- **代码**：
  - `src/tui/app/render.rs`（状态栏高度计算 +1 边框）。
  - `src/cli/args.rs`（启用 / 关闭鼠标捕获，含 panic hook）。
- **spec**：无需 delta——三处修复均使代码回归既有 spec 要求（`subagent-status-display` 的「display all active subagents」「↑↓ auto-activate」「Tab no-op」；`subagent-focus-view` 的「timeline scrollable only via mouse wheel」）。
- **依赖/API**：无外部 API、无依赖变更。crossterm 0.28.1 已提供 `crossterm::event::{EnableMouseCapture, DisableMouseCapture}`。
- **风险**：启用鼠标捕获后，终端模拟器原生的滚轮回滚（buffer scrollback）会被 TUI 拦截为滚轮事件——这是 TUI 应用的预期行为（TUI 自管理滚动），与 ratatui/crossterm 应用惯例一致。
