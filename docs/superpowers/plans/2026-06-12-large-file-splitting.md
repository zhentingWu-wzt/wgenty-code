# Large File Splitting Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Split the 3 largest Rust source files into focused, single-responsibility modules using subdirectory organization.

**Architecture:** Convert `tui/app.rs` and `gui/chat.rs` from flat files into subdirectory modules (`tui/app/mod.rs` + sub-modules, `gui/chat/mod.rs` + sub-modules). Extract test code from `tasks/management.rs` into a sibling file. Each resulting file targets ≤300 lines.

**Tech Stack:** Rust, cargo build/test/clippy

**Spec:** `docs/superpowers/specs/2026-06-12-large-file-splitting-design.md`

---

## File Structure

### Created files:
- `src/tasks/tests.rs` — extracted test module from management.rs
- `src/gui/chat/mod.rs` — ChatPanel struct + ui() + public API
- `src/gui/chat/message.rs` — message rendering methods
- `src/gui/chat/input.rs` — input area + send logic
- `src/tui/app/mod.rs` — App struct + new() + run()
- `src/tui/app/types.rs` — AgentMode, AppEvent, UIMessage, etc.
- `src/tui/app/event.rs` — handle_event() + helpers
- `src/tui/app/render.rs` — render methods
- `src/tui/app/input.rs` — submit_input() + slash commands
- `src/tui/app/turn.rs` — turn lifecycle methods

### Deleted files:
- `src/gui/chat.rs` — replaced by gui/chat/ directory
- `src/tui/app.rs` — replaced by tui/app/ directory

### Modified files:
- `src/tasks/management.rs` — remove inline tests, add `#[path]` reference
- `src/tui/util.rs` — update import path if needed

---

## Task 1: Extract tests from tasks/management.rs

**Files:**
- Create: `src/tasks/tests.rs`
- Modify: `src/tasks/management.rs:622-787`

- [ ] **Step 1: Extract test code to src/tasks/tests.rs**

Extract lines 623–787 from `src/tasks/management.rs` (the contents of `mod tests { ... }`) into a new file `src/tasks/tests.rs`:

```rust
use super::*;
use crate::tools::Tool;

#[tokio::test]
async fn test_shared_task_store() {
    // Simulate DaemonState initialization
    let task_manager = Arc::new(TaskManagementTool::new());
    let shared_store = task_manager.task_store();
    let tool = TaskManagementTool::from_arc(shared_store);

    // Create a task via the tool (simulates agent calling task_management)
    let input = serde_json::json!({
        "operation": "create",
        "subject": "test task",
        "description": "verify shared store",
        "priority": "high"
    });
    let result = tool.execute(input).await.unwrap();
    assert!(result.content.contains("success"));

    // Extract task_id for dependency test
    let data: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    let task_a_id = data["task_id"].as_str().unwrap().to_string();

    // Verify task_manager sees the task
    let all = task_manager.get_all_tasks().await;
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].subject, "test task");
    assert_eq!(all[0].status, TaskStatus::Pending);
    assert_eq!(all[0].priority, TaskPriority::High);

    // Create another task via task_manager
    let input2 = serde_json::json!({
        "operation": "create",
        "subject": "second task",
        "description": "another one"
    });
    task_manager.execute(input2).await.unwrap();

    // Verify tool sees both tasks
    let result = tool
        .execute(serde_json::json!({"operation": "list"}))
        .await
        .unwrap();
    let data: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(data["count"].as_u64().unwrap(), 2);

    // Test task dependencies (blockedBy)
    let result_b = tool
        .execute(serde_json::json!({
            "operation": "create",
            "subject": "blocked task",
            "description": "depends on first task",
            "blockedBy": [task_a_id]
        }))
        .await
        .unwrap();
    let data_b: serde_json::Value = serde_json::from_str(&result_b.content).unwrap();
    let task_b_id = data_b["task_id"].as_str().unwrap().to_string();

    // Try to complete B before A -> should fail
    let err = tool
        .execute(serde_json::json!({
            "operation": "complete",
            "task_id": task_b_id
        }))
        .await
        .unwrap_err();
    assert!(
        err.message.contains("blocked by"),
        "Expected blocked by error, got: {}",
        err.message
    );

    // Try to set B to in_progress before A is completed -> should fail
    let err = tool
        .execute(serde_json::json!({
            "operation": "update",
            "task_id": task_b_id,
            "status": "in_progress"
        }))
        .await
        .unwrap_err();
    assert!(
        err.message.contains("blocked by"),
        "Expected blocked by error, got: {}",
        err.message
    );

    // Complete A first
    tool.execute(serde_json::json!({
        "operation": "complete",
        "task_id": task_a_id
    }))
    .await
    .unwrap();

    // Now complete B -> should succeed
    let result = tool
        .execute(serde_json::json!({
            "operation": "complete",
            "task_id": task_b_id
        }))
        .await
        .unwrap();
    assert!(
        result.content.contains("success"),
        "Expected success completing B after A is done"
    );

    // Test blocked operation — all tasks completed, so no blocked tasks
    let blocked_result = tool
        .execute(serde_json::json!({
            "operation": "blocked"
        }))
        .await
        .unwrap();
    let blocked_data: serde_json::Value =
        serde_json::from_str(&blocked_result.content).unwrap();
    assert_eq!(blocked_data["count"].as_u64().unwrap(), 0);

    // Test set_dependencies operation with invalid blocker
    let result_c = tool
        .execute(serde_json::json!({
            "operation": "create",
            "subject": "task C",
            "description": "will be blocked"
        }))
        .await
        .unwrap();
    let data_c: serde_json::Value = serde_json::from_str(&result_c.content).unwrap();
    let task_c_id = data_c["task_id"].as_str().unwrap().to_string();

    // Invalid blocker should fail
    let err = tool
        .execute(serde_json::json!({
            "operation": "set_dependencies",
            "task_id": task_c_id,
            "blockedBy": ["nonexistent-id"]
        }))
        .await
        .unwrap_err();
    assert!(
        err.message.contains("Blocker task not found"),
        "Expected blocker not found error"
    );

    // Set valid dependencies
    let result = tool
        .execute(serde_json::json!({
            "operation": "set_dependencies",
            "task_id": task_c_id,
            "blockedBy": [task_a_id]
        }))
        .await
        .unwrap();
    assert!(
        result.content.contains("Dependencies updated"),
        "Expected dependencies updated"
    );
}
```

- [ ] **Step 2: Replace inline test module with #[path] reference**

In `src/tasks/management.rs`, delete the entire `#[cfg(test)] mod tests { ... }` block (lines 622–787) and replace with:

```rust
#[cfg(test)]
#[path = "tests.rs"]
mod tests;
```

- [ ] **Step 3: Verify build and tests pass**

Run:
```bash
cargo test -p wgenty_code tasks::tests::test_shared_task_store
```
Expected: test passes. Then:
```bash
cargo build && cargo clippy -- -D warnings
```
Expected: clean build, zero warnings.

- [ ] **Step 4: Commit**

```bash
git add src/tasks/tests.rs src/tasks/management.rs
git commit -m "refactor(tasks): extract test module to separate file"
```

---

## Task 2: Convert gui/chat.rs to gui/chat/ — create types (message.rs)

**Files:**
- Create: `src/gui/chat/message.rs`

- [ ] **Step 1: Create gui/chat/ directory and message.rs**

```bash
mkdir -p src/gui/chat
```

Create `src/gui/chat/message.rs` with the message rendering methods extracted from `src/gui/chat.rs` lines 201–726. The file needs its own imports and uses `impl ChatPanel` (Rust allows multiple impl blocks across files):

```rust
//! Chat message rendering — avatars, bubbles, markdown, tool calls, indicators.

use super::ChatPanel;
use super::super::chat_types::{Attachment, ChatMessage, MessageRole};
use super::super::content_parser::{split_by_code_blocks, ContentPart};
use super::super::syntax_highlight::format_code_block;
use super::super::tool_calls::{ToolCall, ToolCallStatus};
use egui::{
    Align, Color32, CornerRadius, Frame, Layout, Margin, RichText, Stroke, Ui, Vec2,
};
```

Then add `impl ChatPanel { ... }` containing these methods (copy verbatim from `src/gui/chat.rs`):
- `render_message()` (line 201)
- `render_claude_avatar()` (line 290)
- `render_user_avatar()` (line 300)
- `render_user_message_bubble()` (line 313)
- `render_claude_message_content()` (line 338)
- `render_thinking_process()` (line 429)
- `render_markdown_content()` (line 471)
- `apply_inline_formatting()` (line 602)
- `render_tool_call_card()` (line 607)
- `render_attachment()` (line 655)
- `render_cursor_animation()` (line 683)
- `render_loading_indicator()` (line 699)

Copy each method body verbatim from `src/gui/chat.rs` into the impl block.

- [ ] **Step 2: Commit**

```bash
git add src/gui/chat/message.rs
git commit -m "refactor(gui): create chat/message.rs with rendering methods"
```

---

## Task 3: Create gui/chat/input.rs

**Files:**
- Create: `src/gui/chat/input.rs`

- [ ] **Step 1: Create input.rs with input area methods**

Create `src/gui/chat/input.rs` with the input-area methods extracted from `src/gui/chat.rs` lines 728–831:

```rust
//! Chat input area — text editor, send button, message conversion.

use super::ChatPanel;
use super::super::chat_types::MessageRole;
use egui::{
    Color32, CornerRadius, Frame, Margin, RichText, Stroke, TextEdit, Ui, Vec2,
};
```

Then add `impl ChatPanel { ... }` containing these methods (copy verbatim from `src/gui/chat.rs`):
- `render_input_area()` (line 728)
- `send_message()` (line 801)
- `convert_to_api_messages()` (line 822)

- [ ] **Step 2: Commit**

```bash
git add src/gui/chat/input.rs
git commit -m "refactor(gui): create chat/input.rs with input area methods"
```

---

## Task 4: Finalize gui/chat/ conversion — replace chat.rs with mod.rs

**Files:**
- Create: `src/gui/chat/mod.rs`
- Delete: `src/gui/chat.rs`

- [ ] **Step 1: Create mod.rs with ChatPanel struct and remaining methods**

Create `src/gui/chat/mod.rs` containing:
- Module declarations at the top
- All `use` imports from the original `src/gui/chat.rs` (lines 1–18)
- `ChatPanel` struct definition (lines 21–30)
- `impl Default for ChatPanel` (lines 32–46)
- `impl ChatPanel` with: `set_on_send_message()`, `set_current_model()`, `ui()` (lines 48–106), `render_welcome_banner()` (lines 108–199), and the public API methods: `add_message()`, `clear_messages()`, `set_loading()`, `update_last_message()` (lines 834–855)

The top of the file:

```rust
//! Chat panel — main chat UI for the egui desktop frontend.

mod message;
mod input;

use super::chat_types::{Attachment, ChatMessage, MessageRole};
use super::content_parser::{split_by_code_blocks, ContentPart};
use super::syntax_highlight::format_code_block;
use super::tool_calls::ToolCall;
use egui::{
    Align, Color32, CornerRadius, Frame, Layout, Margin, RichText, ScrollArea, Stroke,
    TextEdit, Ui, Vec2,
};
```

Followed by the `ChatPanel` struct, `Default` impl, and `impl ChatPanel` block with the methods listed above.

- [ ] **Step 2: Delete the old gui/chat.rs file**

```bash
rm src/gui/chat.rs
```

- [ ] **Step 3: Verify build passes**

```bash
cargo build --features gui-egui
```
Expected: clean build with no errors. If `--features gui-egui` is the default feature, just `cargo build` works.

- [ ] **Step 4: Run clippy and tests**

```bash
cargo clippy -- -D warnings && cargo test --all
```
Expected: zero warnings, all tests pass.

- [ ] **Step 5: Commit**

```bash
git add -A src/gui/
git commit -m "refactor(gui): convert chat.rs to chat/ subdirectory module"
```

---

## Task 5: Create tui/app/types.rs — type definitions

**Files:**
- Create: `src/tui/app/types.rs`

- [ ] **Step 1: Create tui/app/ directory**

```bash
mkdir -p src/tui/app
```

- [ ] **Step 2: Create types.rs with all type definitions**

Create `src/tui/app/types.rs` containing types extracted from `src/tui/app.rs` lines 1–188 (everything before the `App` struct). Include the module doc comment and necessary imports:

```rust
//! Type definitions for the TUI application — events, messages, modes.

use crate::state::agent_phase::{TurnAbortReason, TurnId};
use crate::tui::client::{SessionInfo, TodoItem};
use crossterm::event::KeyEvent;
use ratatui::style::Color;

/// Agent operating mode, cycled via Shift+Tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentMode {
    Normal,
    PlanMode,
    AcceptEdits,
    Yolo,
}

impl AgentMode {
    pub fn label(&self) -> &str { ... }
    pub fn color(&self) -> Color { ... }
    pub fn next(&self) -> Self { ... }
}

/// Wraps a oneshot sender for returning question answers.
pub struct QuestionResponder(pub Option<tokio::sync::oneshot::Sender<Vec<String>>>);
impl std::fmt::Debug for QuestionResponder { ... }

#[derive(Debug)]
pub enum PermissionResponse {
    AllowOnce,
    AlwaysAllow,
    Deny,
}

/// Wraps a oneshot sender for returning permission decisions.
pub struct PermissionResponder(pub Option<tokio::sync::oneshot::Sender<PermissionResponse>>);
impl std::fmt::Debug for PermissionResponder { ... }

/// Events that drive the UI loop.
#[derive(Debug)]
pub enum AppEvent {
    // ... all variants from lines 93-161, copy verbatim
}

/// UI state for a single message in the chat view.
#[derive(Debug, Clone, PartialEq)]
pub enum MessageRole {
    User,
    Assistant,
    Tool,
    System,
}

#[derive(Debug, Clone)]
pub struct UIMessage {
    pub role: MessageRole,
    pub content: String,
    pub tool_name: Option<String>,
    pub tool_args: Option<serde_json::Value>,
    pub content_collapsed: bool,
    pub tool_collapsed: bool,
    pub diff_data: Option<DiffData>,
    pub tool_metadata: Option<serde_json::Value>,
}

/// Structured diff data for syntax-highlighted diff rendering in the TUI.
#[derive(Debug, Clone)]
pub struct DiffData {
    pub file_path: String,
    pub old_content: String,
    pub new_content: String,
}
```

Copy each struct/enum/impl body **verbatim** from `src/tui/app.rs`. The `{ ... }` placeholders above mean: copy the original code exactly.

- [ ] **Step 3: Commit**

```bash
git add src/tui/app/types.rs
git commit -m "refactor(tui): create app/types.rs with type definitions"
```

---

## Task 6: Create tui/app/event.rs — event handling

**Files:**
- Create: `src/tui/app/event.rs`

- [ ] **Step 1: Create event.rs with handle_event and helpers**

Create `src/tui/app/event.rs` containing the event handling methods from `src/tui/app.rs` lines 402–912:

```rust
//! Event handling for the TUI application.

use super::types::*;
use super::App;
use crate::api::ChatMessage;
use crate::prompts::{self, PromptContext};
use crate::state::agent_phase::AgentPhase;
use crate::tui::client::SessionInfo;
use crate::tui::components;
use crate::tui::components::plan_panel::{PlanItem, PlanStatus};
use crate::tui::traits::Component;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;
```

Then add `impl App { ... }` containing these methods (copy verbatim from `src/tui/app.rs`):
- `handle_event()` (lines 402–885)
- `push_question_answer()` (lines 887–900)
- `push_permission_result()` (lines 901–912)

- [ ] **Step 2: Commit**

```bash
git add src/tui/app/event.rs
git commit -m "refactor(tui): create app/event.rs with event handling"
```

---

## Task 7: Create tui/app/render.rs — rendering methods

**Files:**
- Create: `src/tui/app/render.rs`

- [ ] **Step 1: Create render.rs with all render methods**

Create `src/tui/app/render.rs` containing render methods from `src/tui/app.rs` lines 913–1050:

```rust
//! TUI rendering — layout, chat view, status bar, input area.

use super::types::*;
use super::App;
use crate::tui::components;
use crate::tui::theme;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use ratatui::Frame;
```

Then add `impl App { ... }` containing these methods (copy verbatim):
- `render()` (lines 913–983)
- `render_chat()` (lines 984–995)
- `render_status()` (lines 996–1001)
- `render_input()` (lines 1002–1009)
- `render_mode_label()` (lines 1012–1019)
- `render_pending_inputs()` (lines 1021–1050)

- [ ] **Step 2: Commit**

```bash
git add src/tui/app/render.rs
git commit -m "refactor(tui): create app/render.rs with rendering methods"
```

---

## Task 8: Create tui/app/input.rs — input handling

**Files:**
- Create: `src/tui/app/input.rs`

- [ ] **Step 1: Create input.rs with submit_input**

Create `src/tui/app/input.rs` containing the input handling method from `src/tui/app.rs` lines 1051–1176:

```rust
//! User input handling — submit text and slash commands.

use super::types::*;
use super::App;
use crate::api::ChatMessage;
use crate::state::agent_phase::{AgentPhase, TurnAbortReason};
use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;
```

Then add `impl App { ... }` containing:
- `submit_input()` (lines 1052–1176)

- [ ] **Step 2: Commit**

```bash
git add src/tui/app/input.rs
git commit -m "refactor(tui): create app/input.rs with input handling"
```

---

## Task 9: Create tui/app/turn.rs — turn management

**Files:**
- Create: `src/tui/app/turn.rs`

- [ ] **Step 1: Create turn.rs with turn lifecycle methods**

Create `src/tui/app/turn.rs` containing the turn management methods from `src/tui/app.rs` lines 1177–1273:

```rust
//! Turn lifecycle — spawn, cancel, and queue management.

use super::types::*;
use super::App;
use crate::api::ChatMessage;
use crate::state::agent_phase::{AgentPhase, TurnAbortReason, TurnId};
use crate::tui::agent::AgentLoop;
use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;
```

Then add `impl App { ... }` containing:
- `start_next_turn()` (lines 1178–1198)
- `spawn_agent_turn()` (lines 1203–1257)
- `cancel_current_turn()` (lines 1259–1268)
- `pending_count()` (lines 1270–1272)

- [ ] **Step 2: Commit**

```bash
git add src/tui/app/turn.rs
git commit -m "refactor(tui): create app/turn.rs with turn management"
```

---

## Task 10: Finalize tui/app/ — replace app.rs with mod.rs

**Files:**
- Create: `src/tui/app/mod.rs`
- Delete: `src/tui/app.rs`

- [ ] **Step 1: Create mod.rs with App struct and lifecycle**

Create `src/tui/app/mod.rs` containing:
- Module declarations and re-exports
- The `App` struct definition (lines 189–246 from original)
- `impl App` with: `new()` (lines 248–358), `event_sender()` (lines 359–361), `run()` (lines 363–401)
- Utility re-exports (lines 1276–1285)

```rust
//! Application main loop — event handling, layout, and daemon lifecycle.

pub mod types;
mod event;
mod render;
mod input;
mod turn;

pub use types::*;

use crate::api::ChatMessage;
use crate::config;
use crate::prompts::{self, PromptContext};
use crate::tui::client::DaemonClient;
use crate::tui::components::input::InputBox;
use crate::tui::components::permission::PermissionState;
use crate::tui::components::question::QuestionState;
use crate::tui::components::session::SessionState;
use crate::tui::components::plan_panel::PlanPanelState;
use crate::tui::components::task_panel::TaskPanelState;
use crate::state::agent_phase::AgentPhase;

use crossterm::event::EnableBracketedPaste;
use ratatui::Terminal;
use std::collections::VecDeque;
use std::io;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::Mutex as TokioMutex;

/// Application state for the TUI.
pub struct App {
    // ... copy struct fields verbatim from original lines 191-244
}

impl App {
    pub fn new(
        daemon_client: DaemonClient,
        session_id: String,
        settings_lock: config::watcher::SettingsHandle,
    ) -> Self {
        // ... copy body verbatim from original lines 253-357
    }

    pub fn event_sender(&self) -> mpsc::UnboundedSender<AppEvent> {
        self.event_tx.clone()
    }

    /// Run the main event loop.
    pub async fn run<B: ratatui::backend::Backend + std::marker::Unpin>(
        &mut self,
        terminal: &mut Terminal<B>,
    ) -> anyhow::Result<()> {
        // ... copy body verbatim from original lines 367-400
    }
}

// Utility re-exports (moved from bottom of original app.rs)
pub use super::util::truncate_session_name;
pub use super::util::start_daemon;
pub use super::util::compute_collapse_state;
pub use super::util::extract_diff_data;
pub use super::util::split_unified_diff;
pub use super::util::extract_tool_metadata;
pub use super::util::format_tool_result;
pub use super::util::tool_label;
pub use super::util::agent_phase_from_event;
pub use super::util::centered_rect;
```

- [ ] **Step 2: Delete the old tui/app.rs**

```bash
rm src/tui/app.rs
```

- [ ] **Step 3: Verify build passes**

```bash
cargo build
```
Expected: clean build. If there are import errors in sub-modules (missing `use` statements), fix them now. Common issues:
- Sub-modules may need `use super::types::*;` or specific type imports
- Sub-modules referencing `self.committed_messages`, `self.event_tx`, etc. need access to `App` fields — since they are `impl App` blocks in the same crate, they have direct field access

- [ ] **Step 4: Run clippy and tests**

```bash
cargo clippy -- -D warnings && cargo test --all
```
Expected: zero warnings, all tests pass.

- [ ] **Step 5: Commit**

```bash
git add -A src/tui/
git commit -m "refactor(tui): convert app.rs to app/ subdirectory module"
```

---

## Task 11: Final verification — line counts and full build

**Files:** (none modified — verification only)

- [ ] **Step 1: Check line counts of split files**

```bash
wc -l src/tui/app/mod.rs src/tui/app/types.rs src/tui/app/event.rs \
      src/tui/app/render.rs src/tui/app/input.rs src/tui/app/turn.rs \
      src/gui/chat/mod.rs src/gui/chat/message.rs src/gui/chat/input.rs \
      src/tasks/management.rs src/tasks/tests.rs
```
Expected: No file exceeds ~400 lines. The largest should be `event.rs` at ~320 lines.

- [ ] **Step 2: Full build, clippy, and test suite**

```bash
cargo build --release && cargo clippy --all-targets -- -D warnings && cargo test --all
```
Expected: release build succeeds, zero clippy warnings, all tests pass.

- [ ] **Step 3: Verify no orphaned references to old files**

```bash
grep -rn "tui::app\." src/ | grep -v "tui/app/"
grep -rn "gui::chat\." src/ | grep -v "gui/chat/"
```
Expected: no results (all references use module paths, not file paths).

- [ ] **Step 4: Final commit (if any fixes were needed)**

```bash
git add -A
git commit -m "refactor: finalize large file splitting — all verifications pass"
```
