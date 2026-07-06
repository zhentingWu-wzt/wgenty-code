## Why

进入 subagent focus view（全屏 subagent 窗口）后，主对话的折叠快捷键 Ctrl+O（展开最后一条）和 Ctrl+E（全部展开）"失效"：按键后无可见效果，且退出 focus view 后发现主对话的消息折叠状态被悄悄改掉。

## 根因分析

`src/tui/input_reader.rs` 将 Ctrl+O/Ctrl+E 提前转换为独立事件 `AppEvent::ToggleCollapseLatest`/`ToggleCollapseAll`（line 57-68），而非作为 `AppEvent::KeyEvent` 转发。

`src/tui/app/event.rs` 的 `KeyEvent` 分支（line 31-124）在 `self.subagent_focus` 激活时吞没所有按键（仅 Esc/↑↓/Enter/t/Ctrl+P/Ctrl+L 例外）。但 Ctrl+O/Ctrl+E 已被转成独立事件，**绕过**此吞没逻辑，直接进入 `ToggleCollapseLatest`（line 519）和 `ToggleCollapseAll`（line 501）处理。

这两个处理直接操作 `self.committed_messages`（主对话消息列表），**不检查 `self.subagent_focus`**。因此 focus view 打开时：
- Ctrl+O/Ctrl+E 操作被 focus view 遮住的主对话，用户看不到效果 → "失效"
- 退出 focus view 后，主对话折叠状态被悄悄修改 → 副作用

这违反 `subagent-focus-view` spec "selector SHALL be the sole keyboard-interactive area" 的设计意图：focus view 打开时按键不应影响被遮住的主对话。

## 修复目标

- focus view 打开时，Ctrl+O/Ctrl+E 操作 focus view 内的 tool call 折叠（复用 `collapsed_tool_ids` 机制），而非主对话
- Ctrl+E = 切换全部 tool call 折叠（与 `t` 键一致）
- Ctrl+O = 切换最后一条 tool call 折叠
- focus view 关闭时，Ctrl+O/Ctrl/E 保持原主对话行为
- 消除"悄悄改主对话状态"的副作用

## Impact

- **Code**:
  - `src/tui/components/subagent_focus_view.rs`：`FocusViewState` 新增 `toggle_fold_all`/`toggle_fold_latest` 方法（从 event.rs 的 't' 逻辑提取）
  - `src/tui/app/event.rs`：'t' 键复用 `toggle_fold_all`；`ToggleCollapseAll`/`ToggleCollapseLatest` 开头检查 `subagent_focus`
- **Spec**: `subagent-focus-view` delta spec（MODIFIED "Focus view navigation and exit"，增加 Ctrl+O/Ctrl+E scenario）
- **User-visible behavior**: focus view 里 Ctrl+O/Ctrl/E 不再失效，操作 timeline tool call 折叠；不再副作用主对话
