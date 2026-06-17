---
comet_change: tui-message-display-filter
role: technical-design
canonical_spec: openspec
status: archived
archived-with: commit 8d64524
archived-date: 2026-06-17
---

# TUI 消息展示过滤 — 技术设计

## 目标

修改 TUI 消息渲染行为，实现：
- **System 消息不展示**
- **Tool 结果默认完全折叠**（仅显示操作标签，无内容行），展开后最多 100 行
- **Session 恢复时还原 tool name/args**，使折叠标签显示正确的 "Read /path/to/file"

## 影响文件

| 文件 | 改动类型 |
|------|----------|
| `src/tui/app/event.rs` | HistoryLoaded 重构 + 正常对话无需改 |
| `src/tui/components/chat.rs` | Tool 渲染阈值 + System 跳过 |
| `src/tui/util.rs` | 无需改（compute_collapse_state 已正确） |

## 不改动

- `conversation_history` 数据结构 — 保持完整，API 调用不受影响
- Session 存储格式 — system 消息仍存在文件中，只是不展示
- App 初始化 (`mod.rs`) — system_messages 注入逻辑不变
- `save_session` / `load_session` API — 不变
- `/clear`、auto-compaction 逻辑 — 不变

---

## 详细设计

### 1. `HistoryLoaded` — system 过滤 + tool name 匹配

**文件**: `src/tui/app/event.rs:695-721`

**核心数据结构**:

```rust
// ChatMessage (API 层) — 不变
struct ChatMessage {
    role: String,                        // "user" | "assistant" | "tool"
    content: Option<String>,
    tool_calls: Option<Vec<ToolCall>>,   // assistant: tool_use blocks
    tool_call_id: Option<String>,        // tool: 关联到 tool_use.id
    ...
}

// ToolCall
struct ToolCall {
    id: String,                          // "toolu_001"
    function: ToolCallFunction {
        name: String,                    // "read_file"
        arguments: String,               // JSON string: {"path": "/foo"}
    },
    ...
}

// UIMessage (显示层) — 不变
struct UIMessage {
    role: MessageRole,                   // User | Assistant | Tool | System
    tool_name: Option<String>,           // "read_file"
    tool_args: Option<serde_json::Value>, // {"path": "/foo"}
    tool_collapsed: bool,
    ...
}
```

**算法: 两遍扫描**:

```
第一遍: 构建 tool_use_map
  messages.iter()
    ├── role == "assistant" + tool_calls is Some
    │   └── for each ToolCall:
    │       map.insert(tc.id, (tc.function.name, parse(tc.function.arguments)))
    └── 其他消息 → skip

第二遍: 转换 ChatMessage → UIMessage
  messages.iter()
    ├── role == "system" → continue (跳过)
    ├── role == "user" → User + compute_collapse_state
    ├── role == "assistant" → Assistant + compute_collapse_state
    ├── role == "tool" →
    │   let (name, args) = map.get(&msg.tool_call_id).cloned().unwrap_or_default();
    │   → Tool { tool_name: name, tool_args: args, tool_collapsed: true }
    └── _ → continue (未知角色也跳过)
```

**关键细节**:
- `ToolCall.function.arguments` 是 JSON string，需要 `serde_json::from_str` 解析为 `Value`
- 解析失败时 tool_args 为 None，不影响后续渲染（tool_label 返回空字符串）
- 匹配失败的 tool 消息（tool_call_id 不在 map 中）：tool_name 退化为 tool_call_id，tool_args 为 None。折叠标签显示 " Used"（兜底行为）

### 2. Tool 折叠渲染 — 0 行内容

**文件**: `src/tui/components/chat.rs:478-489`

**当前代码**:
```rust
let show = if msg.tool_collapsed {
    content_lines.iter().take(3).copied().collect::<Vec<_>>()  // 3行
} else {
    content_lines.iter().take(MAX_TOOL_DISPLAY_LINES).copied().collect::<Vec<_>>() // 5行
};
```

**新代码**:
```rust
const MAX_TOOL_EXPANDED_LINES: usize = 100;

let show: Vec<&str> = if msg.tool_collapsed {
    Vec::new()  // 折叠态: 0 行内容，仅显示 header
} else {
    content_lines
        .iter()
        .take(MAX_TOOL_EXPANDED_LINES)
        .copied()
        .collect()
};
```

**效果对比**:

```
折叠态:
  当前: • Read /path/to/file          期望: • Read /path/to/file
        line1 content                        ... +30 lines (Enter to expand)
        line2 content
        line3 content
        ... +27 lines (Ctrl+O to expand)

展开态:
  当前: 最多 5 行                        期望: 最多 100 行
```

**提示文字更新**: 原 `Ctrl+O to expand` 改为 `Enter to expand`（与实际交互一致 — 实际选中行按 Enter 展开的是最后一条消息）

> **注意**: 当前 tool 折叠消息的单个选中展开/折叠交互由 `ToggleCollapseLatest` (Enter键) 和 `ToggleCollapseAll` (Ctrl+O) 控制，它们同时设置 `content_collapsed` 和 `tool_collapsed`。需要确认 tool 单独选中展开的行为是否已支持。如 `event.rs:449-466` 所示，两者通过 toggle 全部/最后一条消息实现，对 tool 消息同样有效。展开后的 "+N lines" 提示保持 `Ctrl+O to expand` 文案。

### 3. System 消息渲染 — 跳过

**文件**: `src/tui/components/chat.rs:526-535`

**当前代码**:
```rust
MessageRole::System => msg
    .content.lines()
    .map(|line| Line::from(Span::styled(line.to_string(), Style::default().fg(DIM_COLOR))))
    .collect(),
```

**新代码**:
```rust
MessageRole::System => Vec::new(),
```

### 4. 正常对话流程 — 无需修改

正常对话中 tool 消息已通过以下路径默认折叠：
- `AppEvent::ToolStart` (event.rs:518-527): `tool_collapsed: true`
- `AppEvent::ToolResult` (event.rs:529-561): `tool_collapsed: true`
- `compute_collapse_state` (util.rs:76): `MessageRole::Tool => (false, true)`

渲染时折叠态展示 0 行内容由改动 2 统一生效。

---

## 数据流

```
Session 恢复 (HistoryLoaded)
═══════════════════════════════════════════════════════════════

  ChatMessage[]  ──第一遍扫描──▶  tool_use_map: HashMap<id, (name, args)>
       │
       └──第二遍转换──▶  committed_messages (Vec<UIMessage>)
                              │
                              ├── system → (skip)
                              ├── user → User
                              ├── assistant → Assistant
                              └── tool → Tool { tool_name, tool_args, collapsed }

正常对话 (ToolStart → ToolResult)
═══════════════════════════════════════════════════════════════

  ToolStart → UIMessage { tool_name, tool_args, content: "", collapsed: true }
  ToolResult → 替换 content, 保持 collapsed: true

渲染 (chat.rs render_message)
═══════════════════════════════════════════════════════════════

  MessageRole::System → Vec::new()
  MessageRole::Tool + collapsed → header only (verb + label), 0 content lines
  MessageRole::Tool + expanded → header + up to 100 content lines
  MessageRole::User/Assistant → (不变)
```

---

## 测试检查清单

1. `/clear` → 无 system 消息展示
2. 正常对话发送请求 → tool 结果仅显示折叠标签 (如 `• Read /path/to/file`)，无内容行
3. Ctrl+O toggle → tool 展开显示最多 100 行，再次 Ctrl+O 折叠
4. 保存 session → 退出 → 重新启动 → 加载 session:
   - system 消息不展示
   - tool 标签正确显示 (如 `• Read /path/to/file` 而非 `• Used` )
   - 折叠态无内容行
5. Enter 键 toggle 最后一条消息 → tool 参与 toggle
6. 滚动行为：折叠/展开时 scroll_offset 不变（已由 ToggleCollapseLatest/ToggleCollapseAll 保证）
