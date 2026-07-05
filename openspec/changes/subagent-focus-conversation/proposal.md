## Why

子代理焦点视图当前以"事件时间线"展示执行过程——每条 Thought/Action/ToolResult/Error/Completion 事件一行，带时间戳和类型标签。噪声大、不直观，用户难以快速理解子代理的对话流。主 agent 的聊天历史是 turn-based 对话视图（assistant 文本块 + 可折叠工具调用），清晰可读。焦点视图应采用同样的对话式渲染，让子代理执行过程像主聊天一样直观。

## What Changes

- 焦点视图从"事件时间线"重设计为"对话视图"：展示子代理完整 `messages` 对话（user prompt + assistant 响应 + tool calls + tool results），复用主聊天渲染样式（User/Assistant/Tool 块、可折叠工具调用、diff 支持）。
- `SubagentProgress` 新增 `messages: Vec<ChatMessage>` 字段，由 `subagent_loop` 填充并随进度发送到 daemon，TUI 轮询后供焦点视图渲染。
- 焦点视图复用/适配 `chat.rs` 的 `message_to_lines` 渲染逻辑（`ChatMessage` → `UIMessage` 风格块）；Thought 事件不再单独显示——assistant 文本块即覆盖。
- 工具调用默认折叠（像主聊天），可展开查看参数/结果/diff。
- 状态条（`subagent_status_bar`）不变，仍用 `action_log` 显示当前工具。

## Capabilities

### New Capabilities
<!-- 无新增 capability -->

### Modified Capabilities
- `subagent-focus-view`: 焦点视图渲染从"事件时间线"改为"对话视图"——展示子代理完整 messages 对话，复用主聊天样式（assistant 文本块 + 可折叠工具调用）。原 event-timeline 相关 requirements 调整为 conversation 语义。

## Impact

- `src/agent/progress.rs`: `SubagentProgress` 新增 `messages: Vec<ChatMessage>` 字段（serde 序列化）。
- `src/teams/subagent_loop.rs`: emit 闭包填充 `messages`（快照当前 `messages` 对话）；`messages` 在 loop_future 内部，需在 emit 时克隆。
- `src/tui/components/subagent_focus_view.rs`: `FocusViewState` 新增 `messages` 缓存 + 工具折叠状态；`FocusView::render` 改为对话式渲染。
- `src/tui/app/event.rs`: `SubagentUpdate` 处理时刷新焦点视图 `messages`。
- `src/tui/components/chat.rs`: 抽取/复用 `message_to_lines`（当前 `pub fn`，参数为 `UIMessage`——需 `ChatMessage→UIMessage` 转换或泛化）。
- 数据流开销：`messages` 全量序列化每 500ms poll，子代理长跑体量大——design 阶段决定截断/增量策略。
