# Comet Design Handoff

- Change: tui-message-display-filter
- Phase: design
- Mode: compact
- Context hash: d2dd238978637392324040e907eaeb8f416911b2b952479ed5c456f825ee5605

Generated-by: comet-handoff.sh

OpenSpec remains the canonical capability spec. This handoff is a deterministic, source-traceable context pack, not an agent-authored summary.

## openspec/changes/tui-message-display-filter/proposal.md

- Source: openspec/changes/tui-message-display-filter/proposal.md
- Lines: 1-42
- SHA256: 83d5559dad114a981e0c88984f7b9ae75cf30284f3839fffc32a01f1d87fd78d

```md
## Why

当前 TUI 的 session 历史恢复和正常对话中，**系统提示词**（约 8 层组装指令）和**工具结果**（如文件内容、命令输出）会直接渲染在聊天窗口中。这导致：

1. **Session 恢复时噪声大**：恢复历史 session 后，8 层系统提示词全部以 dim 颜色展示，占据大量屏幕空间，干扰用户回顾对话内容
2. **工具结果干扰阅读**：经常性的工具结果（Read 文件内容、Bash 输出等）夹在对话中，打断用户阅读 agent 回复的连贯性
3. **信息密度低**：用户核心关注的是「与 agent 的对话」，而非系统内部指令或工具执行的原始输出

## What Changes

修改 TUI 的消息渲染逻辑（`src/tui/components/chat.rs` 和 `src/tui/app/event.rs`），实现：

- **系统提示词不展示**：`role: "system"` 的消息在渲染时完全跳过
- **工具结果默认折叠**：`role: "tool"` 的消息默认显示为一行可展开摘要（格式：`tool_name result (N lines)`），用户按 Enter 可展开查看完整内容
- **工具调用正常展示**：`role: "assistant"` 中的 `tool_use` blocks 保持当前展示行为（工具名 + 核心参数）

### 影响范围

- **正常对话渲染**：`src/tui/components/chat.rs` 中的消息渲染逻辑
- **Session 历史恢复**：`src/tui/app/event.rs` 中 `HistoryLoaded` handler 的消息转换逻辑

### 不修改

- `conversation_history` 数据结构
- Session 文件的存储格式
- Anthropic API 调用逻辑（已正确处理 system 消息）
- `/clear`、auto-compaction 行为

## Capabilities

### Modified Capabilities

- `tui-command-completion`：消息渲染模块增加 system/tool 消息的过滤和折叠能力

## Impact

- **修改文件**：
  - `src/tui/components/chat.rs`：消息渲染逻辑（system 跳过、tool 折叠）
  - `src/tui/app/event.rs`：`HistoryLoaded` handler 中 `MessageRole` 转换逻辑
- **可能新增**：
  - `src/tui/components/chat.rs` 中新增 tool-result 折叠渲染方法
- **风险**：低。纯显示层修改，不影响数据层和 API 层
```

## openspec/changes/tui-message-display-filter/design.md

- Source: openspec/changes/tui-message-display-filter/design.md
- Lines: 1-139
- SHA256: 75815e749a3fec66fece519b7c94adc573f60ed51c4a7c1e6fac64350faef270

[TRUNCATED]

```md
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
```

Full source: openspec/changes/tui-message-display-filter/design.md

## openspec/changes/tui-message-display-filter/tasks.md

- Source: openspec/changes/tui-message-display-filter/tasks.md
- Lines: 1-36
- SHA256: 324115d6ba2626633f972b1418c82e092073b5a0a1b0b92786f6f41d5541a9e2

```md
## Tasks

### 1. 修改 `compute_collapse_state` 支持 tool 消息默认折叠
- [ ] 在 `src/tui/app/event.rs` 中，`compute_collapse_state` 函数增加 `MessageRole::Tool` 分支，设置 `tool_collapsed: true`
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
```

