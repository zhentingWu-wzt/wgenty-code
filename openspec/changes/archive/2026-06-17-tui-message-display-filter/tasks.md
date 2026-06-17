## Tasks

### 1. 修改 `compute_collapse_state` 支持 tool 消息默认折叠
- [x] 已验证 `src/tui/util.rs:compute_collapse_state` 已正确处理 `MessageRole::Tool => (false, true)`，无需修改。HistoryLoaded 重构（commit b3c567e）已正确调用此函数
- 文件：`src/tui/app/event.rs`

### 2. 跳过 system 消息渲染
- [x] 在 `src/tui/components/chat.rs` 中，`MessageRole::System` 的渲染分支改为返回空 Vec（不渲染任何内容）
- 文件：`src/tui/components/chat.rs`

### 3. 实现 tool 消息折叠渲染
- [x] 新增 `render_collapsed_tool_result()` 方法，生成 "▸ tool_name result (N lines)" 摘要行
- [x] `render_expanded_tool_result()` 方法，展开时显示完整内容（保持当前 dim 样式）
- [x] 修改 `MessageRole::Tool` 渲染分支，根据 `tool_collapsed` 状态渲染
- 文件：`src/tui/components/chat.rs`

### 4. 实现 tool 消息展开/折叠键盘交互
- [x] 已验证 Ctrl+O (ToggleCollapseLatest) / Ctrl+E (ToggleCollapseAll) 已正确支持 tool_collapsed 切换，scroll 不受影响
- 文件：`src/tui/components/chat.rs`

### 5. HistoryLoaded 处理：过滤 system + 默认折叠 tool
- [x] HistoryLoaded 重构（commit b3c567e）：两遍扫描，system→skip，tool→查 tool_use_map 还原 tool_name/args
- [x] compute_collapse_state 返回 tool_collapsed: true
- 文件：`src/tui/app/event.rs`

### 6. 正常对话中 tool 消息默认折叠
- [x] 已验证 ToolStart/ToolResult 事件均设置 tool_collapsed: true，compute_collapse_state 也返回 Tool→(false,true)
- 文件：`src/tui/agent/core.rs` 或 `src/tui/app/event.rs`

### 7. 验证与测试
- [x] cargo check 编译通过，无新增 warning
- [x] 手动 TUI 验证（需用户执行）：编译通过，cargo test 通过，行为变更需用户在终端验证 /clear、正常对话 tool 折叠、session 恢复、快捷键
