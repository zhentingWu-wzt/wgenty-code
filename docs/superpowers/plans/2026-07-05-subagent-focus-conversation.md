---
change: subagent-focus-conversation
design-doc: docs/superpowers/specs/2026-07-05-subagent-focus-conversation-design.md
base-ref: e9bf59db48dd5769fa27abef280fffe6c8958d22
archived-with: 2026-07-05-subagent-focus-conversation
---

# Implementation Plan: Subagent Focus Conversation View

## 概述

将子代理焦点视图从"event 时间线"重设计为"对话视图"：`SubagentProgress` 新增 `messages` 字段，焦点视图用 `ChatMessage→UIMessage` 转换 + 复用主聊天 `message_to_lines` 渲染，工具调用默认折叠可展开。

## 现有代码关键结构

- `SubagentProgress` (`src/agent/progress.rs`): events, action_log, text_snapshot 等字段。serde 序列化。
- `subagent_loop.rs`: `let mut messages: Vec<ChatMessage>` 在 `loop_future` 内部维护对话。emit 闭包构造 `SubagentProgress`。
- `FocusViewState` (`src/tui/components/subagent_focus_view.rs`): events, scroll_offset, auto_scroll, active_area, selector_index。`build()` 从 `SubagentProgress` 构建。`rebuild()` 刷新。
- `FocusView::render` + `build_timeline_lines`: 当前时间线渲染。
- `chat::message_to_lines` (`src/tui/components/chat.rs`): `pub fn`，吃 `UIMessage`，返回 `Vec<Line>`。处理 User/Assistant/Tool/System 四种角色，支持折叠、spinner、diff。
- `UIMessage` (`src/tui/app/mod.rs` 或 `types.rs`): role, content, tool_name, tool_args, tool_collapsed, tool_running, diff_data, tool_metadata, content_collapsed。

archived-with: 2026-07-05-subagent-focus-conversation
---

## Task 1: SubagentProgress.messages 字段

**文件:** `src/agent/progress.rs`, `src/teams/subagent_loop.rs`

**目标:** `SubagentProgress` 新增 `messages` 字段，subagent_loop emit 时填充。

**实现:**

```rust
// src/agent/progress.rs — SubagentProgress struct
pub struct SubagentProgress {
    // ... existing fields unchanged ...
    /// Full conversation messages from the subagent's loop,
    /// for rendering the focus view as a chat history.
    pub messages: Vec<ChatMessage>,
}
```

emit 闭包单行添加（`src/teams/subagent_loop.rs:202` 附近）：
```rust
cb(SubagentProgress {
    // ... existing fields ...
    messages: messages.clone(),
});
```

**验证:** `cargo check` 编译通过。`cargo test --lib` 通过（无需新测试——字段是自动 serde，emit 闭包已有其他 clone 逻辑作为先例）。

archived-with: 2026-07-05-subagent-focus-conversation
---

## Task 2: ChatMessage → UIMessage 转换 + 合并

**文件:** `src/tui/components/subagent_focus_view.rs`（新增 `fn chat_messages_to_ui_messages`）

**目标:** 将子代理的 `Vec<ChatMessage>` 转为 `Vec<UIMessage>`，合并匹配的 tool_call/tool_result 对，供渲染使用。

**实现要点:**

```rust
use crate::api::ChatMessage;
use crate::tui::app::UIMessage;

/// Convert subagent messages into UIMessage blocks suitable for chat-style
/// rendering. Merges assistant(tool_calls) with the corresponding tool(result)
/// so each tool displays as a single block (spinner → result).
pub fn chat_messages_to_ui_messages(
    messages: &[ChatMessage],
) -> Vec<UIMessage> {
    // Step A: build tool_call.id → result lookup
    let mut result_map: HashMap<String, &ChatMessage> = HashMap::new();
    for msg in messages {
        if msg.role == "tool" {
            if let Some(ref tcid) = msg.tool_call_id {
                result_map.insert(tcid.clone(), msg);
            }
        }
    }

    // Step B: iterate and convert
    let mut ui_msgs: Vec<UIMessage> = Vec::new();
    let mut consumed_tool_ids: HashSet<String> = HashSet::new();
    for msg in messages {
        match msg.role.as_str() {
            "system" => continue,  // skip
            "user" => {
                ui_msgs.push(UIMessage {
                    role: MessageRole::User,
                    content: msg.content.clone().unwrap_or_default(),
                    tool_name: None,
                    tool_args: None,
                    tool_collapsed: false,
                    content_collapsed: false,
                    tool_running: false,
                    diff_data: None,
                    tool_metadata: None,
                });
            }
            "assistant" => {
                // Text block
                if let Some(ref content) = msg.content {
                    if !content.is_empty() {
                        ui_msgs.push(UIMessage {
                            role: MessageRole::Assistant,
                            content: content.clone(),
                            tool_name: None,
                            tool_args: None,
                            tool_collapsed: false,
                            content_collapsed: false,
                            tool_running: false,
                            diff_data: None,
                            tool_metadata: None,
                        });
                    }
                }
                // Tool call blocks (merged with result if available)
                if let Some(ref tool_calls) = msg.tool_calls {
                    for tc in tool_calls {
                        let args: serde_json::Value =
                            serde_json::from_str(&tc.function.arguments)
                                .unwrap_or(serde_json::Value::Null);
                        let has_result = result_map.contains_key(&tc.id);
                        if has_result {
                            consumed_tool_ids.insert(tc.id.clone());
                            let result = result_map[&tc.id];
                            let result_content = result.content.clone().unwrap_or_default();
                            let diff_data = extract_diff_data(
                                &tc.function.name, &args, &result_content,
                            );
                            ui_msgs.push(UIMessage {
                                role: MessageRole::Tool,
                                content: result_content,
                                tool_name: Some(tc.function.name.clone()),
                                tool_args: Some(args),
                                tool_collapsed: true,  // default collapsed
                                content_collapsed: false,
                                tool_running: false,
                                diff_data,
                                tool_metadata: None,
                            });
                        } else {
                            ui_msgs.push(UIMessage {
                                role: MessageRole::Tool,
                                content: String::new(),
                                tool_name: Some(tc.function.name.clone()),
                                tool_args: Some(args),
                                tool_collapsed: false,  // open while running
                                content_collapsed: false,
                                tool_running: true,
                                diff_data: None,
                                tool_metadata: None,
                            });
                        }
                    }
                }
            }
            "tool" => {
                // Orphan tool result (no matching tool_call found)
                if let Some(ref tcid) = msg.tool_call_id {
                    if !consumed_tool_ids.contains(tcid) {
                        ui_msgs.push(UIMessage {
                            role: MessageRole::Tool,
                            content: msg.content.clone().unwrap_or_default(),
                            tool_name: tcid.clone(),
                            tool_args: None,
                            tool_collapsed: true,
                            content_collapsed: false,
                            tool_running: false,
                            diff_data: None,
                            tool_metadata: None,
                        });
                    }
                    // else: already consumed by the tool_call merge above
                }
            }
            _ => {}
        }
    }
    ui_msgs
}
```

**`extract_diff_data` 复用:** 转换中直接调用 `crate::tui::util::extract_diff_data`（已有 pub fn），对已完成的 tool result 提取 diff（如果内容包含 unified diff）。

**测试:** 在 `subagent_focus_view.rs` 的 `#[cfg(test)]` 模块中添加：
- `test_convert_user_message` — user → User UIMessage
- `test_convert_assistant_text` — assistant with text, no tools → Assistant
- `test_convert_assistant_with_tool_calls_merged` — assistant tool_calls + matching tool result → 1 Tool (collapsed, done)
- `test_convert_tool_call_without_result` — assistant tool_calls, no result → 1 Tool (running)
- `test_convert_orphan_tool_result` — tool result without matching tool_call → 1 Tool (collapsed)
- `test_skip_system_message` — system → skip

**验收:** `cargo test subagent_focus_view` 通过。

archived-with: 2026-07-05-subagent-focus-conversation
---

## Task 3: FocusViewState 变更

**文件:** `src/tui/components/subagent_focus_view.rs`

**目标:** 移除 `events` 字段，新增 `messages` + `collapsed_tool_ids`。更新 `build()` 和 `rebuild()`。

**实现:**

```rust
// Remove: pub events: Vec<SubagentEvent>,
// Add:
pub messages: Vec<ChatMessage>,
pub collapsed_tool_ids: HashSet<String>,
```

**`build()` 更新**（`events: p.events.clone()` → 移除，新增）:
```rust
messages: p.messages.clone(),
collapsed_tool_ids: HashSet::new(),
```

**`rebuild()` 更新**（`self.events = p.events.clone()` → 移除，新增）:
```rust
self.messages = p.messages.clone();
// collapsed_tool_ids preserved (fresh messages may invalidate some
// entries, but stale IDs in the set are harmless — they simply won't
// match any tool_call in the new messages)
// ... rest of rebuild (status, elapsed, tokens) unchanged
```

**import 更新:** 添加 `use crate::api::ChatMessage; use std::collections::HashSet;`。

**验证:** `cargo check` 编译通过。现有 6 个 FocusViewState 测试需更新（移除 events 相关断言，添加 messages 相关断言）：
- `test_build_from_node`: 验证 `messages` 已填充
- `test_rebuild_preserves_ui_state`: 验证 `collapsed_tool_ids` 保留
- `test_rebuild_missing_node_noop`: messages 不变

archived-with: 2026-07-05-subagent-focus-conversation
---

## Task 4: build_conversation_lines + FocusView::render 对话化

**文件:** `src/tui/components/subagent_focus_view.rs`

**目标:** 用 `build_conversation_lines` 替代 `build_timeline_lines`，`FocusView::render` 中使用对话视图。

**实现:**

```rust
fn build_conversation_lines(
    state: &FocusViewState,
    inner: Rect,
    spinner_frame: u8,
) -> Vec<Line<'static>> {
    let mut lines: Vec<Line> = Vec::new();
    if state.messages.is_empty() {
        lines.push(Line::from(Span::styled(
            "Waiting for subagent…",
            Style::default().fg(Color::Rgb(108, 112, 134)),
        )));
        return lines;
    }

    let ui_messages = chat_messages_to_ui_messages(&state.messages);
    let total = ui_messages.len();

    // Render each UIMessage via chat::message_to_lines, applying
    // fold state for tool messages. Skip scroll — scroll handled
    // by the Paragraph::scroll() call in FocusView::render.
    for (idx, ui_msg) in ui_messages.iter().enumerate() {
        let show_expand_hint = idx == total - 1;  // only on last

        // Apply collapse override for tool messages
        let mut uim = ui_msg.clone();
        if uim.role == MessageRole::Tool {
            if let Some(ref tool_name) = uim.tool_name {
                // Use a synthetic tool_call_id: the name is the best
                // proxy (tool_call.id not stored in UIMessage).
                // For merged tools, tool_name + args provide a
                // stable key across rebuilds.
                if state.collapsed_tool_ids.contains(tool_name) {
                    uim.tool_collapsed = true;
                }
            }
        }

        lines.extend(message_to_lines(
            &uim,
            inner.width,
            spinner_frame,
            show_expand_hint,
        ));
    }

    lines
}
```

Hmm — tool_call_id 在转换时作为 `tool_name` 被存储，但这会造成模糊性。更好的做法：用 tool_call.id（唯一 UUID）作为折叠 key。为此需要转换函数同时返回 `tool_call.id` 的映射。或者直接在 build_conversation_lines 内处理折叠，避开索引。

**修正方案：在 `chat_messages_to_ui_messages` 中为 tool UIMessage 使用 `tool_call.id` 作为 `tool_metadata` 中的标记**，然后在折叠检查中用该标记。

```rust
// In chat_messages_to_ui_messages, for merged tool:
tool_metadata: serde_json::json!({"tool_call_id": tc.id}),
```

然后在折叠检查中取 tool_metadata 的 tool_call_id。

```rust
// In build_conversation_lines, apply fold:
let tool_id = uim.tool_metadata.as_ref()
    .and_then(|m| m.get("tool_call_id"))
    .and_then(|v| v.as_str());
if let Some(tid) = tool_id {
    if state.collapsed_tool_ids.contains(tid) {
        uim.tool_collapsed = true;
    }
}
```

**FocusView::render 更新:**
- `build_timeline_lines(state, timeline_inner)` → `build_conversation_lines(state, timeline_inner, spinner_frame)`
- 滚动仍使用 `timeline_start_index`（对 conversation_lines 的行数）。
- Help bar 更新为 `↑↓ scroll  t toggle fold  Tab selector  Esc back`。

**键盘 `t` 折叠切换（FocusView::render 内的帮助文本 + event.rs 的路由）:**
在 event.rs 的 focus routing 中添加：
```rust
KeyCode::Char('t') if focus.active_area == FocusArea::Timeline => {
    // Toggle fold for the nearest visible tool message
    // (implementation: find the tool message nearest to scroll position,
    //  toggle its tool_call_id in collapsed_tool_ids)
    // For simplicity, toggle ALL tools' fold state:
    // if collapsed_tool_ids.is_empty() { fold all } else { unfold all }
    return;
}
```
此逻辑可后续细化（当前先全局折叠/展开）。

**验证:** `cargo check` 编译通过。Tool result 的 diff 出现在展开块内。

archived-with: 2026-07-05-subagent-focus-conversation
---

## Task 5: 移除 events 残留引用

**文件:** `src/tui/components/subagent_focus_view.rs`

**目标:** 确认 `events` 字段及相关函数 (`build_timeline_lines`, `timeline_start_index` 保留但改为对 conversation_lines 使用) 已清理。移除 `use ... SubagentEvent` import（如果 events 已移除）。

**操作:**
- 移除 `use crate::agent::progress::SubagentEvent;`（如果 `FocusViewState` 不再持有 `events: Vec<SubagentEvent>`）。
- `timeline_start_index` 保留（仍被 build_conversation_lines 的滚动逻辑使用）。
- `build_timeline_lines` 变为 `build_conversation_lines`（Task 4 已完成）。
- 移除 `truncate` 函数调用（如果 conversation 渲染不依赖它——但 ChatMessage 的 content 可能超长，仍可能需要。保留 `truncate`）。

**验证:** `cargo check` 编译通过。grep `events` 和 `SubagentEvent` 在 `subagent_focus_view.rs` 中仅保留合法的（`rebuild` 不再 touch `events`，`build` 不再 clone `events`）。

archived-with: 2026-07-05-subagent-focus-conversation
---

## Task 6: 验证

**命令:**
```bash
cargo clippy --all-targets -- -D warnings
cargo test --all
cargo fmt -- --check
```

**验收:** 三项全部通过，零 warning。
