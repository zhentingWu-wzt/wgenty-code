# Comet Design Handoff

- Change: subagent-focus-conversation
- Phase: design
- Mode: compact
- Context hash: 8e2f8ddc92b6dfc74ad581a7eb2a42e98ed376e4c38957d00fab9378ca59cba5

Generated-by: comet-handoff.sh

OpenSpec remains the canonical capability spec. This handoff is a deterministic, source-traceable context pack, not an agent-authored summary.

## openspec/changes/subagent-focus-conversation/proposal.md

- Source: openspec/changes/subagent-focus-conversation/proposal.md
- Lines: 1-28
- SHA256: 6073260c621896e9c6b00b9d074f3889c581e93f245a17d3cbda069a48c5c3a8

```md
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
```

## openspec/changes/subagent-focus-conversation/design.md

- Source: openspec/changes/subagent-focus-conversation/design.md
- Lines: 1-28
- SHA256: cf6c8aed65960d586730fdf005acaf2618a118181db5e03fa2f64863fae4dc37

```md
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
```

## openspec/changes/subagent-focus-conversation/tasks.md

- Source: openspec/changes/subagent-focus-conversation/tasks.md
- Lines: 1-30
- SHA256: 0c7fee575a5b57844de98a22ad38e55c527955ddccd97ec9cb66b5f51b446c23

```md
## 1. Data Flow — SubagentProgress.messages

- [ ] 1.1 Add `messages: Vec<ChatMessage>` field to `SubagentProgress` (`src/agent/progress.rs`) with serde serialization
- [ ] 1.2 Subagent loop (`src/teams/subagent_loop.rs`): snapshot `messages` at each emit point and set `progress.messages = messages.clone()`
- [ ] 1.3 Verify daemon serialization round-trip: subagent loop → daemon store → TUI poll carries `messages`

## 2. Focus View — ChatMessage→UIMessage Conversion

- [ ] 2.1 Add a conversion function `chat_message_to_ui_lines` in `chat.rs` or `subagent_focus_view.rs` that renders a `ChatMessage` as `Vec<ratatui::text::Line>`, reusing existing `message_to_lines` logic (User→"› You" block, Assistant→text block, Tool→collapsible block with verb+args+result+diff)
- [ ] 2.2 Unit tests for the conversion: each ChatMessage role produces expected line groups

## 3. Focus View — Render

- [ ] 3.1 `FocusViewState` (`subagent_focus_view.rs`): add `messages: Vec<ChatMessage>`, `collapsed_tool_indices: HashSet<String>` (keyed by tool_call_id to survive message insertions)
- [ ] 3.2 `FocusViewState::build` and `::rebuild`: populate `messages` from `node.progress.messages`; rebuild refreshes messages and preserves collapse state
- [ ] 3.3 `FocusView::render`: replace the timeline area with a conversation view — iterate `messages`, convert each to lines via step 2.1, apply collapse state for tool messages, scroll with existing scroll model. Keep the header and selector bars.
- [ ] 3.4 Add keyboard handling: ↑↓ scroll conversation; `t` toggle expand selected tool (or Ctrl+O like main chat)

## 4. Event Routing

- [ ] 4.1 `SubagentUpdate` handler (`event.rs`): call `focus.rebuild()` to refresh messages (existing logic, verify it works for messages too)
- [ ] 4.2 Ensure auto_scroll pins to latest conversation message on entry/rebuild

## 5. Tests & Validation

- [ ] 5.1 Unit tests for ChatMessage→UIMessage conversion
- [ ] 5.2 Unit tests for FocusViewState messages rebuild (preserves collapse state)
- [ ] 5.3 `cargo clippy --all-targets -- -D warnings` passes
- [ ] 5.4 `cargo test --all` passes
- [ ] 5.5 `cargo fmt --check` passes
```

