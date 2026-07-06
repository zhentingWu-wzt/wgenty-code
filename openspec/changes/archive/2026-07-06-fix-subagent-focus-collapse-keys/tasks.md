## 1. 提取 fold helper 到 FocusViewState

- [x] 1.1 在 `src/tui/components/subagent_focus_view.rs` 的 `FocusViewState` 新增 `toggle_fold_all(&mut self)`，从 event.rs 't' 逻辑提取（用 `chat_messages_to_ui_messages` 转换，操作 `collapsed_tool_ids`）。
- [x] 1.2 新增 `toggle_fold_latest(&mut self)`：逆序找最后一条 `MessageRole::Tool` 的 `tool_call_id`，翻转其在 `collapsed_tool_ids` 中的状态。

## 2. event.rs 接入

- [x] 2.1 't' 键处理改为调用 `focus.toggle_fold_all()`，删除内联逻辑。
- [x] 2.2 `AppEvent::ToggleCollapseAll` 开头检查 `self.subagent_focus`：激活时 `focus.toggle_fold_all()`；否则原主对话逻辑。
- [x] 2.3 `AppEvent::ToggleCollapseLatest` 开头检查 `self.subagent_focus`：激活时 `focus.toggle_fold_latest()`；否则原主对话逻辑。

## 3. delta spec

- [x] 3.1 创建 `openspec/changes/fix-subagent-focus-collapse-keys/specs/subagent-focus-view/spec.md`，MODIFIED "Focus view navigation and exit" requirement，增加 Ctrl+O/Ctrl+E scenario。

## 4. 构建与验证

- [x] 4.1 `cargo build` 通过。
- [x] 4.2 `cargo clippy --lib -- -D warnings` 零 warning。
- [x] 4.3 `cargo test --lib` 通过。
- [x] 4.4 根因消除检查：`ToggleCollapseLatest`/`ToggleCollapseAll` 在 `subagent_focus` 激活时不再操作 `committed_messages`。
