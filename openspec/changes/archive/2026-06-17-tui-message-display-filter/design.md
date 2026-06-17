## 总体架构

```
┌─────────────────────────────────────────────────────────────────────┐
│                     MESSAGE DISPLAY FILTER                           │
└─────────────────────────────────────────────────────────────────────┘

  conversation_history                    TUI 渲染
  (完整数据，不变)                        (过滤 + 折叠)
  ┌──────────────────┐                   ┌──────────────────────┐
  │ system           │────── 跳过 ──────▶│ (不渲染)              │
  │ system           │                   │                       │
  │ ... (8 layers)   │                   │                       │
  ├──────────────────┤                   ├──────────────────────┤
  │ user             │────── 原样 ──────▶│ User: "hello"         │
  ├──────────────────┤                   ├──────────────────────┤
  │ assistant        │                   │ Agent: "Hi! I'll..."  │
  │  + tool_use      │────── 展示 ──────▶│  📎 Read path/to/f   │
  ├──────────────────┤                   ├──────────────────────┤
  │ tool             │────── 折叠 ──────▶│  ▸ Read result (12L) │
  ├──────────────────┤                   ├──────────────────────┤
  │ assistant (text) │────── 原样 ──────▶│ Agent: "Here's..."   │
  └──────────────────┘                   └──────────────────────┘
```

## 改动点

### 1. `src/tui/components/chat.rs` — 消息渲染逻辑

**当前状态**：
- `MessageRole::System`：dim 颜色渲染全部内容
- `MessageRole::Tool`：dim 颜色渲染全部内容
- 折叠机制：`content_collapsed` 控制 assistant 内容折叠，`tool_collapsed` 控制 tool_use 折叠

**改动**：

#### 1a. System 消息：跳过渲染
```rust
// 当前 (line ~526)
MessageRole::System => msg.content.lines().map(|line| { ... }).collect()

// 改为
MessageRole::System => vec![], // 或 Line::from("")
```
在消息渲染的 match 分支中，`MessageRole::System` 返回空 Vec，不渲染任何内容。

#### 1b. Tool 消息：默认折叠
```rust
// 当前 (line ~526)
MessageRole::Tool => msg.content.lines().map(|line| { ... }).collect()

// 改为：默认折叠，生成一行摘要
MessageRole::Tool => {
    if msg.tool_collapsed {
        // 折叠态：显示 "▸ tool_name result (N lines)"
        render_collapsed_tool_result(&msg)
    } else {
        // 展开态：显示完整内容（保持当前 dim 样式）
        render_expanded_tool_result(&msg)
    }
}
```

新增渲染方法：
```rust
fn render_collapsed_tool_result(msg: &UIMessage) -> Vec<Line> {
    let line_count = msg.content.lines().count();
    let label = format!("▸ {} result ({} lines)", 
        msg.tool_name.as_deref().unwrap_or("tool"),
        line_count);
    vec![Line::from(Span::styled(label, Style::default().fg(DIM_COLOR).add_modifier(Modifier::ITALIC)))]
}
```

#### 1c. `compute_collapse_state` 调整
当前 `compute_collapse_state` 在 `event.rs` 中调用，为 assistant text 和 tool_use 计算折叠状态。需要增加：tool 消息默认 `tool_collapsed: true`。

### 2. `src/tui/app/event.rs` — HistoryLoaded 转换

**当前状态** (`HistoryLoaded` handler, lines 695-721)：
```rust
let role = match msg.role.as_str() {
    "user" => MessageRole::User,
    "assistant" => MessageRole::Assistant,
    "tool" => MessageRole::Tool,
    _ => MessageRole::System,
};
```

**改动**：
- system 消息直接跳过，不 push 到 `committed_messages`
- tool 消息设置 `tool_collapsed: true`

```rust
for msg in &messages {
    let role = match msg.role.as_str() {
        "user" => MessageRole::User,
        "assistant" => MessageRole::Assistant,
        "tool" => {
            let (content_collapsed, tool_collapsed) = (false, true); // 默认折叠
            // ...
        }
        _ => continue, // system 消息跳过不展示
    };
    // ...
}
```

### 3. `src/tui/components/chat.rs` — Tool 消息键盘交互

在 `chat.rs` 中，处理 tool 消息的展开/折叠交互。当前已有选中行 + Enter 切换 `content_collapsed` 的逻辑。需要扩展到 tool 消息：
- 选中 tool 摘要行，按 Enter → `tool_collapsed = false`，展开完整内容
- 再次按 Enter → `tool_collapsed = true`，折叠回摘要

## 数据流

```
conversation_history                    committed_messages (UIMessage)
(Vec<ChatMessage>)                      (Vec<UIMessage>)
                                        
  ┌──────────┐        event.rs           ┌──────────────┐     chat.rs
  │ system   │────── skip ──────────────▶│ (excluded)   │     render
  │ system   │                           │              │     ┌──────┐
  ├──────────┤                           ├──────────────┤     │ skip │
  │ user     │──▶ User ─────────────────▶│ User         │────▶│render│
  │assistant │──▶ Assistant ────────────▶│ Assistant    │────▶│render│
  │  tool_use│                           │  tool_use    │     │      │
  │ tool     │──▶ Tool ─────────────────▶│ Tool         │────▶│folded│
  │assistant │──▶ Assistant ────────────▶│ Assistant    │────▶│render│
  └──────────┘                           └──────────────┘     └──────┘
```

## 不改动

- `conversation_history` (Arc<Mutex<Vec<ChatMessage>>>) 保持完整，API 调用不受影响
- Session 文件格式不变
- `assemble_instructions()` 不变
- `/clear` 重置逻辑不变（它直接替换 conversation_history）
- App 初始化时 system_messages 注入逻辑不变
