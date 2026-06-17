## Tasks

### 1. 修改 `compute_collapse_state` 支持 tool 消息默认折叠
- [x] 已验证 `src/tui/util.rs:compute_collapse_state` 已正确处理 `MessageRole::Tool => (false, true)`，无需修改。HistoryLoaded 重构（commit b3c567e）已正确调用此函数
- 文件：`src/tui/app/event.rs`

### 2. 跳过 system 消息渲染
- [ ] 在 `src/tui/components/chat.rs` 中，`MessageRole::System` 的渲染分支改为返回空 Vec（不渲染任何内容）
- 文件：`src/tui/components/chat.rs`

### 3. 实现 tool 消息折叠渲染
- [ ] 新增 `render_collapsed_tool_result()` 方法，生成 "▸ tool_name result (N lines)" 摘要行
- [ ] `render_expanded_tool_result()` 方法，展开时显示完整内容（保持当前 dim 样式）
- [ ] 修改 `MessageRole::Tool` 渲染分支，根据 `tool_collapsed` 状态渲染
- 文件：`src/tui/components/chat.rs`

### 4. 实现 tool 消息展开/折叠键盘交互
- [ ] 在键盘事件处理中，选中 tool 摘要行按 Enter 时切换 `tool_collapsed` 状态
- [ ] 确保展开/折叠后 scroll 行为正常
- 文件：`src/tui/components/chat.rs`

### 5. HistoryLoaded 处理：过滤 system + 默认折叠 tool
- [ ] 在 `AppEvent::HistoryLoaded` handler 中，system 消息直接 `continue` 跳过
- [ ] tool 消息在转换时设置 `tool_collapsed: true`
- 文件：`src/tui/app/event.rs`

### 6. 正常对话中 tool 消息默认折叠
- [ ] 验证正常对话中 agent 返回的 tool 消息是否通过 `compute_collapse_state` 默认折叠
- [ ] 如正常对话的 tool 消息不走 `compute_collapse_state`，在消息 push 时设置
- 文件：`src/tui/agent/core.rs` 或 `src/tui/app/event.rs`

### 7. 验证与测试
- [ ] 启动 TUI，执行 "/clear" 确认 system 消息不展示
- [ ] 正常对话：发送请求，确认 tool 结果默认折叠，按 Enter 可展开
- [ ] Session 恢复：保存 session 后退出，重新启动，加载 session 确认 system 不展示、tool 折叠
- [ ] 确认快捷键仍正常工作
