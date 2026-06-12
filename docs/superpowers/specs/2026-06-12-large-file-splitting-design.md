# 大文件拆分设计

**日期**: 2026-06-12
**范围**: 拆分项目中最大的 3 个 Rust 源文件
**方式**: 子目录 + 多文件组织

## 背景

项目 `wgenty-code` 共 ~38,000 行 Rust 代码。最大的 3 个文件职责过重，难以维护和审查：

| 文件 | 行数 | 核心问题 |
|------|------|----------|
| `src/tui/app.rs` | 1286 | 类型定义、事件处理、渲染、输入处理、Turn 管理全部混在一个 1000+ 行的 impl 块中 |
| `src/gui/chat.rs` | 855 | ChatPanel 的 UI 入口、消息渲染、输入区域全部在一个 impl 块 |
| `src/tasks/management.rs` | 787 | 核心逻辑 + Tool trait + 165 行测试代码混在一个文件 |

## 目标

- 每个文件控制在 ~300 行以内，单一职责
- 保持或改进 API 清晰度（允许调整公开 API）
- 使用子目录组织（`tui/app/mod.rs` + 子模块）
- `cargo build` / `cargo clippy` / `cargo test` 全部通过

---

## 1. `src/tui/app.rs` → `src/tui/app/` (子目录)

### 当前结构 (1286 行)

```
app.rs
├── AgentMode, AppEvent, UIMessage 等类型定义     (1-245)
├── impl App { new(), run(), handle_event() }     (247-885)
├── render 方法 (render, render_chat, etc.)        (913-1050)
├── submit_input() + 斜杠命令处理                  (1051-1176)
├── spawn/cancel_turn + pending_count              (1177-1273)
└── pub use 工具函数重导出                          (1275-1286)
```

### 拆分后结构

```
tui/app/
├── mod.rs       — App 结构体 + new() + run() + 事件循环 (~150行)
├── types.rs     — AgentMode, AppEvent, UIMessage, MessageRole, DiffData,
│                  QuestionResponder, PermissionResponder, PermissionResponse (~200行)
├── event.rs     — impl App { handle_event(), push_question_answer(), 
│                  push_permission_result() } (~320行)
├── render.rs    — impl App { render(), render_chat(), render_status(),
│                  render_input(), render_mode_label(), render_pending_inputs() } (~180行)
├── input.rs     — impl App { submit_input() + 斜杠命令 } (~130行)
└── turn.rs      — impl App { start_next_turn(), spawn_agent_turn(),
                   cancel_current_turn(), pending_count() } (~100行)
```

### 模块关系

- `mod.rs` 声明并 `pub use` 子模块类型
- `types.rs` 定义所有共享类型（被其他子模块 `use super::types::*` 引用）
- `event.rs`, `render.rs`, `input.rs`, `turn.rs` 各自实现 `impl App` 的方法
- Rust 允许同一 struct 的 impl 块分布在不同模块文件中

### 重导出策略

`tui/mod.rs` 中原来是 `pub mod app;`，拆分后保持不变——`tui/app/mod.rs` 通过 `pub use` 重导出所有公开类型：

```rust
// tui/app/mod.rs
pub mod types;
mod event;
mod render;
mod input;
mod turn;

pub use types::*;
```

外部调用者 `use crate::tui::app::App` / `use crate::tui::app::AppEvent` 等保持不变。

文件末尾的 `pub use super::util::*` 重导出移至 `mod.rs`。

---

## 2. `src/gui/chat.rs` → `src/gui/chat/` (子目录)

### 当前结构 (855 行)

```
chat.rs
├── ChatPanel struct + Default                    (1-46)
├── ui() 入口 + welcome banner                     (48-199)
├── render_message() + avatar + bubble             (200-336)
├── render_claude_message_content + thinking       (338-469)
├── render_markdown_content                        (471-600)
├── tool call card, attachment, cursor, loading    (602-726)
├── render_input_area + send_message               (728-855)
```

### 拆分后结构

```
gui/chat/
├── mod.rs       — ChatPanel struct + Default + ui() 入口 + 
│                  welcome banner + set_on_send_message() +
│                  set_current_model() + 公开 API (add_message, clear_messages,
│                  set_loading, update_last_message) (~200行)
├── message.rs   — render_message(), render_claude_avatar(), render_user_avatar(),
│                  render_user_message_bubble(), render_claude_message_content(),
│                  render_thinking_process(), render_markdown_content(),
│                  apply_inline_formatting(), render_tool_call_card(),
│                  render_attachment(), render_cursor_animation(),
│                  render_loading_indicator() (~400行)
└── input.rs     — render_input_area(), send_message(), convert_to_api_messages() (~130行)
```

### 模块关系

- `mod.rs` 持有 `ChatPanel` struct 定义和公开 API
- `message.rs` 和 `input.rs` 通过 `impl ChatPanel` 实现渲染方法
- `message.rs` 中的 `render_markdown_content` 是最大方法 (~130 行)，可考虑进一步提取为 `markdown.rs`，但当前保持在 `message.rs` 内

### 重导出策略

```rust
// gui/chat/mod.rs
mod message;
mod input;

pub use super::chat_types::{Attachment, ChatMessage, MessageRole};
// ChatPanel 在本文件定义，自动 pub
```

`gui/mod.rs` 中 `pub mod chat;` 不变。

---

## 3. `src/tasks/management.rs` → 提取测试 (同级文件)

### 当前结构 (787 行)

```
management.rs
├── debug_log, TaskManagementTool struct + new()   (1-53)
├── get_all_tasks, generate_id, create_task, etc.  (54-220)
├── impl Tool for TaskManagementTool               (222-334)
├── handle_create/update/delete/list/...           (336-621)
└── mod tests { ... }                              (623-787)
```

### 拆分后结构

```
tasks/
├── management.rs   — 核心逻辑 + Tool trait impl (保留 ~620行)
├── tests.rs        — 从 management.rs 提取的测试 (~165行)
├── mod.rs          — 更新: 添加 #[cfg(test)] mod tests;
├── todo_write.rs   — 不变
└── types.rs        — 不变
```

### 提取方式

1. 将 `management.rs` 中的 `#[cfg(test)] mod tests { ... }` 块移到 `tests.rs`
2. 在 `management.rs` 底部添加 `#[cfg(test)] #[path = "tests.rs"] mod tests;`
3. 或者在 `tasks/mod.rs` 中添加 `#[cfg(test)] mod tests;`

选择方案 2：在 `management.rs` 底部用 `#[path]` 属性引用外部测试文件，保持测试与模块的关联性。

---

## 执行顺序

1. **tasks/management.rs** — 最简单，仅提取测试代码，零风险
2. **gui/chat.rs** — 中等复杂度，egui 渲染方法拆分
3. **tui/app.rs** — 最复杂，需要将 1000+ 行 impl 块拆分为 5 个子文件

每完成一步都执行 `cargo build && cargo clippy && cargo test` 验证。

---

## 验证标准

- [ ] `cargo build` 成功（无编译错误）
- [ ] `cargo clippy -- -D warnings` 零警告
- [ ] `cargo test --all` 所有测试通过
- [ ] 拆分后最大的文件不超过 ~400 行
- [ ] 无功能变更——纯结构重构
