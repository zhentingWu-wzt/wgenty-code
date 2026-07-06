## 修复方案

### 方案：focus view 打开时，Ctrl+O/Ctrl/E 操作 focus view 的 tool call 折叠

#### 提取 fold helper 到 FocusViewState

`src/tui/components/subagent_focus_view.rs` 的 `FocusViewState` 新增两个方法，复用现有 `collapsed_tool_ids` 机制（in set = expanded，not in set = collapsed）：

- `toggle_fold_all(&mut self)`：从 event.rs 't' 逻辑提取。若 `collapsed_tool_ids` 为空（全部折叠）→ 加入所有 tool_call_id（全部展开）；否则 clear（全部折叠）。
- `toggle_fold_latest(&mut self)`：新增。用 `chat_messages_to_ui_messages` 转换后逆序找最后一条 `MessageRole::Tool` 的 `tool_call_id`，在 set 中则移除（折叠），否则加入（展开）。

#### event.rs 接入

1. 't' 键（KeyEvent 分支）：改为调用 `focus.toggle_fold_all()`，消除重复逻辑。
2. `AppEvent::ToggleCollapseAll` 开头：`if let Some(ref mut focus) = self.subagent_focus { focus.toggle_fold_all(); } else { 原主对话逻辑 }`
3. `AppEvent::ToggleCollapseLatest` 开头：`if let Some(ref mut focus) = self.subagent_focus { focus.toggle_fold_latest(); } else { 原主对话逻辑 }`

### 取舍

- **为何复用 collapsed_tool_ids**：focus view 已有 per-tool-call 折叠机制（'t'），Ctrl+O/Ctrl/E 复用同一机制，行为一致，无新状态。
- **为何 Ctrl+E = 't'（全部）**：Ctrl+E 在主对话是 "toggle all"，focus view 里等价 't' 的全部折叠/展开，语义一致。
- **为何 Ctrl+O = 最后一条 tool call**：Ctrl+O 在主对话是 "翻转最后一条消息"。focus view 里 message 折叠只有 tool call 维度（assistant content 不折叠），故映射到"最后一条 tool call"。
- **为何不 pass through 到全局**：Ctrl+O/Ctrl/E 全局处理操作主对话，focus view 遮住主对话，pass through 会副作用。focus view 里拦截并操作 focus view 才符合 spec。

### 验证

- `cargo build` / `cargo clippy --lib -- -D warnings` 通过
- `cargo test --lib` 通过
- delta spec scenario 可断言
- 根因消除：`ToggleCollapseLatest`/`ToggleCollapseAll` 在 `subagent_focus` 激活时不再操作 `committed_messages`
