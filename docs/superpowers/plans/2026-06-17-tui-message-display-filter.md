---
change: tui-message-display-filter
design-doc: docs/superpowers/specs/2026-06-17-tui-message-display-filter-design.md
base-ref: a27faf8a82946a5dc5931653b9ed9c95867adafa
status: archived
archived-date: 2026-06-17
---

# TUI 消息展示过滤 — 实施计划

**Goal:** System 消息不展示，Tool 结果默认完全折叠（仅显示操作标签、无内容行），Session 恢复时还原 tool name/args 使折叠标签显示正确名称。

**Architecture:**
- `HistoryLoaded` 处理从单遍扫描改为两遍扫描：先构建 `tool_use_map`（assistant 消息的 tool_calls 建立 id→(name, args) 映射），再逐条转换并跳过 system 消息、为 tool 消息匹配 tool_name/tool_args。
- Tool 渲染折叠态从展示 3 行预览改为 0 行，展开态从全部改为最多 100 行，System 渲染直接返回空 Vec。
- 正常对话中的 tool 消息已在 ToolStart/ToolResult 阶段设置 `tool_collapsed: true`，无需改动。

**Tech Stack:** Rust, ratatui (TUI), crossterm (键盘事件), serde_json (JSON 解析)

## Global Constraints

- `conversation_history` 数据结构保持不变 — API 调用不受影响
- Session 存储格式不变 — system 消息仍在文件中，仅不展示
- App 初始化 (`mod.rs`) system_messages 注入逻辑不变
- `save_session` / `load_session` API 不变
- `/clear`、auto-compaction 逻辑不变
- `compute_collapse_state` (util.rs:72-78) 不变 — 已正确处理 tool 折叠

---

### Task 1: HistoryLoaded 重构 — system 过滤 + tool name 匹配

**Files:**
- Modify: `src/tui/app/event.rs` (HistoryLoaded handler, lines 695-721)

**Interfaces:**
- Consumes: `ChatMessage` (API 层, `src/api/mod.rs:405`), `ToolCall` (`src/api/mod.rs:392`), `UIMessage`/`MessageRole` (`src/tui/app/types.rs`), `compute_collapse_state` (`src/tui/util.rs:72`)
- Produces: 根据 `tool_calls` 及其 `id` 字段匹配 tool 消息的 `tool_name`/`tool_args`

- [x] **Step 1: 添加 HashMap 导入**

在 `src/tui/app/event.rs` 的 use 区块中添加 `use std::collections::HashMap;`

- [x] **Step 2: 重写 HistoryLoaded handler — 两遍扫描算法**

将 `AppEvent::HistoryLoaded(messages)` 的处理代码（当前 695-721 行）替换：

```
第一遍扫描: 遍历 messages，从 assistant 消息的 tool_calls 中提取 id→(name, args) 映射
第二遍扫描:
  - "system" → continue (跳过)
  - "tool" → 查找 tool_use_map[tool_call_id]，设置 tool_name/tool_args；tool_collapsed: true
  - 其余角色不变
  - 未知角色 → continue (跳过)
```

- [x] **Step 3: 编译验证**

`cargo check --bin wgenty-code 2>&1 | head -40`

- [x] **Step 4: 提交**

---

### Task 2: Tool 折叠渲染 — 0 行内容 + 展开上限 100 行

**Files:**
- Modify: `src/tui/components/chat.rs` (render_message 的 Tool 分支, lines 390-512; 常量 line 539-540)

- [x] **Step 1: 修改常量为 `MAX_TOOL_EXPANDED_LINES = 100`**

- [x] **Step 2: 修改 Tool body lines 渲染**

折叠态: `show = Vec::new()`（0 行内容，仅 header）
展开态: `show = content_lines.iter().take(MAX_TOOL_EXPANDED_LINES)`（最多 100 行）
提示文案: `Ctrl+O to expand` 改为 `Enter to expand`

- [x] **Step 3: 编译验证**

- [x] **Step 4: 提交**

---

### Task 3: System 消息渲染跳过

**Files:**
- Modify: `src/tui/components/chat.rs` (render_message 的 System 分支, lines 526-535)

- [x] **Step 1: System 分支改为 `Vec::new()`**

- [x] **Step 2: 编译验证**

- [x] **Step 3: 提交**

---

### Task 4: 验证与测试

- [x] 启动 TUI → `/clear` → 无 system 消息
- [x] 正常对话 → tool 仅显示折叠标签，0 行内容
- [x] Enter 选中 tool → 展开 ≤100 行；再次 Enter → 折叠
- [x] Ctrl+O → 所有消息 toggle
- [x] Session 保存 → 退出 → 重新加载 → system 不可见、tool 标签正确
- [x] 提交
