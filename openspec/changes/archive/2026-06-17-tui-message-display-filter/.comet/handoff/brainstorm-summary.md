# Brainstorm Summary

- Change: tui-message-display-filter
- Date: 2026-06-17

## 确认的技术方案

### 总览
2 文件，4 改动点：
- `src/tui/app/event.rs` — HistoryLoaded 过滤 + tool_use 匹配
- `src/tui/components/chat.rs` — 渲染：system跳过、tool折叠0行、展开上限100行

### 改动 1: `event.rs` HistoryLoaded — system 过滤 + tool name 匹配
- 第一遍扫描：从 assistant 消息 content 中提取 tool_use blocks，构建 `HashMap<tool_use_id, (name, input)>`
- 第二遍扫描：system → skip；tool → 查 map 设置 tool_name/tool_args，tool_collapsed: true
- 匹配失败兜底：显示 "tool_call_id result (N lines)"

### 改动 2: `chat.rs` Tool 折叠渲染
- 折叠态：`show = Vec::new()`（0 行内容）
- 展开态：`take(MAX_TOOL_EXPANDED_LINES = 100)`
- 删除旧的 `take(3)` / `take(5)` 逻辑

### 改动 3: `chat.rs` System 渲染
- `MessageRole::System` 返回空 Vec，不渲染

### 改动 4: `event.rs` compute_collapse_state
- 当前已返回 `MessageRole::Tool => (false, true)`，无需修改

## 关键取舍与风险

| 决策 | 选择 | 原因 |
|------|------|------|
| tool_use 匹配在加载时做 | ✓ | 不持久化额外数据 |
| system 渲染彻底跳过 | ✓ | 无场景需展示 |
| 展开上限 100 行 | ✓ | 平衡完整性和屏幕空间 |

- **风险低**：纯显示层修改，不影响 conversation_history、API、session 存储
- **tool_use 匹配失败兜底**：回退显示 "tool_call_id result (N lines)"

## 测试策略

1. /clear → 无 system 消息
2. 正常对话 → tool 仅显示折叠标签，0 行内容
3. Enter 选中 tool 标签 → 展开完整内容（≤100行）
4. 保存 session → 退出 → 重新加载 → system 不可见、tool 标签正确、折叠无内容
5. Ctrl+O toggle 所有消息 → tool 参与 toggle

## Spec Patch

无
