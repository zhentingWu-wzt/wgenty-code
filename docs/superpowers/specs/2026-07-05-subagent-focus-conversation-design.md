---
comet_change: subagent-focus-conversation
role: technical-design
canonical_spec: openspec
---

# Design Doc: Subagent Focus Conversation View

## Context

### Problem

子代理焦点视图（`subagent_focus_view.rs`）当前使用 `action_log` 事件流渲染为"事件时间线"——每条 Thought/Action/ToolResult/Error/Completion 事件一行，带时间戳和类型标签。用户反馈噪声大、不直观，难以快速理解子代理的对话流。

主 agent 的聊天历史（`chat.rs`）是 turn-based 对话视图（assistant 文本块 + 可折叠工具调用 + diff），清晰可读。用户要求在焦点视图中以同样方式展示子代理的执行过程。

### Current Data Flow

```
subagent_loop.rs → emit(SubagentProgress) → daemon shared store
    → TUI poll_subagent_progress() every 500ms
    → AppEvent::SubagentUpdate → subagent_tree.upsert()
    → FocusView: build_timeline_lines(action_log events)
```

### Goal

焦点视图从"事件时间线"改为"对话视图"——展示子代理的完整 `messages` 对话（user prompt + assistant 响应 + tool calls），用主聊天的渲染样式，像看主聊天一样读子代理的执行过程。

## Goals / Non-Goals

**Goals:**
- 焦点视图展示子代理完整 `messages` 对话，用主聊天渲染样式（User/Assistant/Tool 块、可折叠工具调用、diff）
- 实时刷新（子代理运行时新消息追加）
- 工具调用默认折叠，可展开

**Non-Goals:**
- 不改状态条（`subagent_status_bar`）
- 不改主聊天视图（`chat.rs`）
- 不改子代理执行逻辑（`subagent_loop` 的轮次/工具调用本身）
- 不改 `action_log` 采集（状态条的"当前工具"显示仍用它）

## Technical Design

### 1. Data Flow: SubagentProgress.messages

`SubagentProgress`（`src/agent/progress.rs`）新增字段：

```rust
pub struct SubagentProgress {
    // ... existing fields ...
    /// Full conversation messages from the subagent's loop,
    /// for rendering the focus view as a chat history.
    pub messages: Vec<ChatMessage>,
}
```

无需额外的 serde attribute——`ChatMessage` 已实现 `Serialize + Deserialize`，新增字段自动通过 JSON 透传到 TUI。

**subagent_loop emit 时填充**（`src/teams/subagent_loop.rs`）：

在 emit 闭包的 `cb(SubagentProgress { ... })` 构造处新增：

```rust
messages: messages.clone(),  // snapshot the in-loop messages Vec
```

`messages` 在 `loop_future` 内部（`let mut messages: Vec<ChatMessage> = vec![ChatMessage::system(...), ChatMessage::user(...)];`），emit 闭包已捕获它。`clone()` 全量快照每轮发送。

**数据体量**：先全量发送。子代理消息通常 <50 轮，每轮几 KB。如后续长跑子代理频繁出现超大 messages，再优化为截断最近 N 轮或增量 delta。

### 2. ChatMessage → UIMessage Conversion

新增转换函数（放 `subagent_focus_view.rs` 或 `chat.rs`），将 `Vec<ChatMessage>` 转为 `Vec<UIMessage>` 列表，然后逐条调用现有的 `chat::message_to_lines` 渲染。

**转换规则**（两步：先预处理构建合并 map，再遍历生成 UIMessage 列表）：

**Step A — 预处理**：扫描 messages，构建 `tool_call.id → tool_result` 的 map。对每条 role="tool" 的消息，用其 `tool_call_id` 作为 key，记录 result content（含 diff data 解析）。

**Step B — 遍历生成**：

| ChatMessage role | → UIMessage(s) |
|---|---|
| `"system"` | 跳过（子代理 system prompt 不渲染） |
| `"user"` | `UIMessage { role: User, content }` |
| `"assistant"`（有 content，无 tool_calls） | `UIMessage { role: Assistant, content }` |
| `"assistant"`（有 tool_calls） | 1 个 `UIMessage { role: Assistant, content }` + 每个 tool_call 检查 map：**有 result** → 合并为 1 个 `UIMessage { role: Tool, tool_name, tool_args, content: result_text, tool_running: false, tool_collapsed: true }`；**无 result**（正在执行中）→ 1 个 `UIMessage { role: Tool, tool_name, tool_args, tool_running: true, tool_collapsed: false }` |
| `"tool"` | **不单独生成 UIMessage**（已被上一步的 tool_call 合并消费）。但如果 Step A 中未找到对应的 tool_call（边缘情况：孤立的 tool result），则降级生成独立的 `UIMessage { role: Tool, content: result_text, tool_collapsed: true }` |

这样焦点视图里一个工具调用就是一个 block——spinner 状态下为"running"，收到 result 后刷新为"done"——跟主聊天行为一致。 |

**tool_name 回查**：预处理扫描 messages，构建 `HashMap<String, String>`（`tool_call.id → function.name`）。`"tool"` 消息的 `tool_call_id` 用于回查 `tool_name`。回查失败时降级显示 `tool_call_id` 本身。

**`tool_args`**：`ChatMessage.tool_calls[].function.arguments` 是 JSON 字符串，需 `serde_json::from_str` 解析为 `serde_json::Value`。

**为什么不改 `message_to_lines` 直接吃 `ChatMessage`**：`message_to_lines` 已经是 `pub fn`，参数是 `UIMessage`。先转换再调用改动最小，不波及主聊天渲染逻辑。

### 3. FocusViewState Changes

保留 header 元数据（status、elapsed、tokens、round），新增对话相关字段：

```rust
pub struct FocusViewState {
    // 保留:
    pub node_id: String,
    pub label: String,
    pub status: SubagentStatus,
    pub scroll_offset: usize,
    pub auto_scroll: bool,
    pub active_area: FocusArea,
    pub selector_index: usize,
    // Header summary:
    pub elapsed_ms: u64,
    pub cumulative_tokens: u64,
    pub token_budget_k: Option<u64>,
    pub round: Option<usize>,
    pub max_rounds: Option<usize>,
    pub error_message: Option<String>,
    pub current_tool: Option<String>,        // 保留: header 显示当前工具
    pub current_params: Option<String>,      // 保留: header 显示当前参数
    // 移除: events (Vec<SubagentEvent>) — 由 messages 替代
    // 新增:
    pub messages: Vec<ChatMessage>,           // 替代 events 作为主体渲染
    pub collapsed_tool_ids: HashSet<String>,  // 展开的 tool_call_id 集合
}
```

**`build()` 和 `rebuild()` 更新**：
- `build`: 新增 `messages: p.messages.clone()`，`collapsed_tool_ids: HashSet::new()`（初始全部折叠）
- `rebuild`: 刷新 `messages`（`self.messages = p.messages.clone()`），保留 `collapsed_tool_ids`

**折叠管理**：
- 默认全部折叠（`collapsed_tool_ids` 为空表示没有展开的工具 = 全部折叠）。
- 键盘 `t`：切换当前可见行内最近的一个 tool UIMessage 的折叠状态——如果 tool_call_id 在 set 中则移除（折叠），否则插入（展开）。
- `collapsed_tool_ids` 用 `tool_call.id` 而不是索引——不随 messages 列表变化而漂移。

### 4. FocusView::render — build_conversation_lines

布局不变（header | conversation | selector | help）。中间的 `timeline` 区域改为 `conversation` 区域。

新增 `build_conversation_lines(state, inner) → Vec<Line>`：

1. 预处理 messages：构建 `tool_call.id → tool_name` map。
2. 遍历 messages，对每条 ChatMessage 转成 1-N 个 UIMessage（按 §2 规则），然后调用 `chat::message_to_lines` 生成行。
3. 滚动：复用现有 `timeline_start_index`——conversation_lines 的行数替代 events.len()，`available = inner.height`，`scroll_offset` 语义不变（lines-from-bottom，0=newest）。

**Header/Selector 不变**。Help bar 更新为 `↑↓ scroll  t toggle fold  Tab selector  Esc back`。

### 5. Event Routing

- `SubagentUpdate`（`event.rs`）：现有 `focus.rebuild()` 调用自动刷新 messages（rebuild 中 `self.messages = p.messages.clone()`）。
- `auto_scroll` 锚定最新：现有逻辑（rebuild 时 `if auto_scroll { scroll_offset = 0; }`）——在新的 lines-from-bottom 模型下正确（0=newest）。

## Key Decisions

| Decision | Rationale | Alternative |
|---|---|---|
| 全量 messages 发送 | 简单，子代理消息体量小 | 截断/增量——复杂，先不做；性能问题后再优化 |
| ChatMessage→UIMessage 转换后复用 `message_to_lines` | 最大复用，视觉与主聊天一致 | 新写渲染——代码重复，视觉不一致 |
| 折叠状态用 `HashSet<String>`（tool_call_id） | 抵御 messages 增长时索引漂移 | 用索引——messages 刷新后索引失效 |
| `collapsed_tool_ids` 为空 = 默认全部折叠 | 简洁，与主聊天默认行为一致 | 默认展开——需要更多状态 |

## Risks / Trade-offs

- [messages 体量大] → 全量 clone 每轮 emit + 全量序列化每 500ms poll。Mitigation: 先全量，性能问题后再优化（截断最近 N 轮或增量）。
- [ChatMessage→UIMessage 转换中 tool_args 解析失败] → JSON parse 失败时传空的 `serde_json::Value`，渲染降级为只显示 tool_name（无参数）。
- [tool_call_id 回查失败] → 降级显示 `tool_call_id` 作为 tool_name。
- [collapsed_tool_ids 跨子代理切换] → rebuild 保留 set；切换子代理时 `build` 重新创建空 set（合理的 fresh start）。

## Testing

- **Unit**: ChatMessage→UIMessage 转换（4 种角色 + tool_call 拆分 + tool_call_id 回查）
- **Unit**: FocusViewState::rebuild（messages 刷新 + 折叠保留）
- **Unit**: collapsed_tool_ids 管理（插入/删除/渲染）
- **CI**: `cargo clippy --all-targets -- -D warnings` + `cargo test --all` + `cargo fmt --check`

## Spec Patch

经评估，此变更修改了一个 capability：
- `subagent-focus-view`: 焦点视图渲染从"事件时间线"改为"对话视图"。事件时间线相关 requirements（event type visual distinction、scrolling event timeline、selector bar indicator for current view）将被对话视图相关 requirements 替代或调整。需要在 build 阶段创建 delta spec。
