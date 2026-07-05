## Context

当前焦点视图（`subagent_focus_view.rs`）渲染 `action_log` 事件流为时间线。`SubagentProgress` 有 `action_log`（事件）和 `text_snapshot`（最后 assistant 文本），但没有完整对话。主聊天（`chat.rs`）渲染 `UIMessage` 列表为 turn-based 对话。子代理 `subagent_loop` 内部维护 `messages: Vec<ChatMessage>` 对话，但未发送到 TUI。

## Goals / Non-Goals

**Goals:**
- 焦点视图展示子代理完整对话（user prompt + assistant 响应 + tool calls），像主聊天一样可读。
- 复用主聊天渲染样式（assistant 文本块、可折叠工具调用、diff）。
- 实时刷新（子代理运行时新消息追加）。

**Non-Goals:**
- 不改状态条、主聊天、子代理执行逻辑。
- 不改 action_log 采集（状态条仍用）。

## Decisions

- **数据来源**：`SubagentProgress` 新增 `messages: Vec<ChatMessage>` 字段，`subagent_loop` emit 时克隆当前 `messages` 快照。 alternatives: (a) 从 action_log 重构对话——丢失 user prompt 且角色信息不全；(b) 增量发送 messages delta——复杂，留待 design 阶段评估。
- **渲染复用**：复用 `chat.rs::message_to_lines`（已是 `pub fn`），新增 `ChatMessage→UIMessage` 转换。alternatives: 焦点视图新写一套渲染——重复代码。
- **工具折叠**：默认折叠（像主聊天），`FocusViewState` 新增 `collapsed_tool_indices: HashSet<usize>` 管理展开状态。
- **实时刷新**：`SubagentUpdate` 时刷新 `messages`，`auto_scroll=true` 锚定最新（沿用现有滚动模型）。
- **数据体量**：全量发送 messages（design 阶段决定是否截断最近 N 轮或增量）。

## Risks / Trade-offs

- [messages 体量大] → 全量序列化每 500ms poll，长跑子代理开销大。Mitigation: design 阶段评估截断/增量；可先全量，性能问题再优化。
- [ChatMessage→UIMessage 转换丢失字段] → 子代理消息可能没有主聊天的某些 metadata（如 tool_metadata）。Mitigation: 转换时补默认值，渲染降级。
- [折叠状态随 messages 增长失效] → 新消息追加后索引变化。Mitigation: 用 tool_call_id 而非索引追踪折叠状态。
