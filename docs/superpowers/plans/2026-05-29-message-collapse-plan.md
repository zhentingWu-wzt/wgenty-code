# Message Collapse/Expand Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add fold/collapse for long tool outputs and assistant responses in both TUI and GUI with Ctrl+E (all) and Ctrl+O (latest message) keyboard shortcuts.

**Architecture:** Extend `UIMessage` (TUI) and `ChatMessage` (GUI) with collapse state fields. Compute auto-collapse on message commit via line-count thresholds. Render collapsed paragraphs as 3-line preview with line count. Toggle state via keyboard shortcuts dispatched in the event loop.

**Tech Stack:** Rust, ratatui (TUI), egui/eframe (GUI), crossterm (TUI keyboard events)

---

### Task 1: Extend TUI UIMessage struct with collapse fields

**Files:**
- Modify: `src/tui/app.rs` (UIMessage struct definition)

- [ ] **Step 1: Add `content_collapsed` and `tool_collapsed` fields to UIMessage**

In `src/tui/app.rs`, find the `UIMessage` struct and add the two fields:

```rust
#[derive(Debug, Clone)]
pub struct UIMessage {
    pub role: MessageRole,
    pub content: String,
    pub tool_name: Option<String>,
    pub content_collapsed: bool,
    pub tool_collapsed: bool,
}
```

- [ ] **Step 2: Verify compilation**

Run: `cargo check --bin wgenty-code 2>&1 | head -60`
Expected: compilation errors from missing field initializations in existing code — proceed to Task 2 to fix them.

---

### Task 2: Initialize collapse fields on message commit (TUI)

**Files:**
- Modify: `src/tui/app.rs` (all UIMessage construction sites)

UIMessage is constructed in 5 places: `Submit`, `StreamDone`, `ToolStart`, `ToolResult` (both branches), and `StreamError`. All need default `false` for the new fields, then auto-collapse applied where appropriate.

- [ ] **Step 1: Add helper function for auto-collapse computation**

Add this function near `format_tool_result` in `src/tui/app.rs`:

```rust
/// Compute initial collapse state based on line-count thresholds.
/// Returns (content_collapsed, tool_collapsed) tuple.
fn compute_collapse_state(role: &MessageRole, content: &str, tool_name: Option<&str>) -> (bool, bool) {
    let line_count = content.lines().count();
    match role {
        MessageRole::Assistant => {
            if line_count > 50 { (true, false) } else { (false, false) }
        }
        MessageRole::Tool => {
            if line_count > 10 { (false, true) } else { (false, false) }
        }
        _ => (false, false),
    }
}
```

- [ ] **Step 2: Update AppEvent::Submit handler**

```rust
AppEvent::Submit(text) => {
    let (content_collapsed, tool_collapsed) = compute_collapse_state(&MessageRole::User, &text, None);
    self.committed_messages.push(UIMessage {
        role: MessageRole::User,
        content: text.clone(),
        tool_name: None,
        content_collapsed,
        tool_collapsed,
    });
    // ... rest unchanged
}
```

- [ ] **Step 3: Update AppEvent::StreamDone handler**

```rust
AppEvent::StreamDone { .. } => {
    if !self.streaming_content.is_empty() {
        let content = std::mem::take(&mut self.streaming_content);
        let (content_collapsed, tool_collapsed) = compute_collapse_state(&MessageRole::Assistant, &content, None);
        self.committed_messages.push(UIMessage {
            role: MessageRole::Assistant,
            content,
            tool_name: None,
            content_collapsed,
            tool_collapsed,
        });
    }
    // ... rest unchanged
}
```

- [ ] **Step 4: Update AppEvent::ToolStart handler**

```rust
AppEvent::ToolStart { name } => {
    self.status = format!("executing {}", name);
    self.committed_messages.push(UIMessage {
        role: MessageRole::Tool,
        content: String::new(),
        tool_name: Some(name),
        content_collapsed: false,
        tool_collapsed: false,
    });
}
```

- [ ] **Step 5: Update AppEvent::ToolResult handler** (both branches)

```rust
AppEvent::ToolResult { name, content } => {
    let formatted = format_tool_result(&name, &content);
    let (content_collapsed, tool_collapsed) = compute_collapse_state(&MessageRole::Tool, &formatted, Some(&name));
    if let Some(last) = self.committed_messages.last_mut() {
        if last.role == MessageRole::Tool
            && last.content.is_empty()
            && last.tool_name.as_deref() == Some(&name)
        {
            last.content = formatted;
            last.content_collapsed = content_collapsed;
            last.tool_collapsed = tool_collapsed;
        } else {
            self.committed_messages.push(UIMessage {
                role: MessageRole::Tool,
                content: formatted,
                tool_name: Some(name),
                content_collapsed,
                tool_collapsed,
            });
        }
    }
    self.status = "thinking".to_string();
}
```

- [ ] **Step 6: Update AppEvent::StreamError handler**

```rust
AppEvent::StreamError(msg) => {
    self.committed_messages.push(UIMessage {
        role: MessageRole::System,
        content: format!("⚠ {}", msg),
        tool_name: None,
        content_collapsed: false,
        tool_collapsed: false,
    });
    // ... rest unchanged
}
```

- [ ] **Step 7: Check compilation**

Run: `cargo check --bin wgenty-code 2>&1 | head -30`
Expected: clean compile (no errors).

---

### Task 3: Add Ctrl+E and Ctrl+O keyboard handling (TUI)

**Files:**
- Modify: `src/tui/app.rs` (`read_input` function and `handle_event`)

- [ ] **Step 1: Add Ctrl+E and Ctrl+O intercepts in `read_input`**

In `read_input()`, add two new intercepts before the `KeyEvent` fallback. These go alongside the existing Ctrl+S / Ctrl+T / Ctrl+C intercepts:

```rust
// Ctrl+E -> toggle collapse all
if key.code == KeyCode::Char('e')
    && key.modifiers.contains(KeyModifiers::CONTROL)
{
    let _ = tx.send(AppEvent::ToggleCollapseAll);
    continue;
}
// Ctrl+O -> toggle collapse latest message
if key.code == KeyCode::Char('o')
    && key.modifiers.contains(KeyModifiers::CONTROL)
{
    let _ = tx.send(AppEvent::ToggleCollapseLatest);
    continue;
}
```

- [ ] **Step 2: Add new AppEvent variants**

In the `AppEvent` enum, add:

```rust
/// Toggle collapse all paragraphs
ToggleCollapseAll,
/// Toggle collapse latest message's paragraphs
ToggleCollapseLatest,
```

- [ ] **Step 3: Add handlers in `handle_event`**

Before the `AppEvent::KeyEvent(key)` match block, add:

```rust
AppEvent::ToggleCollapseAll => {
    let any_expanded = self.committed_messages.iter().any(|m| {
        !m.content_collapsed || !m.tool_collapsed
    });
    let new_state = any_expanded; // true = collapse all
    for m in &mut self.committed_messages {
        m.content_collapsed = new_state;
        m.tool_collapsed = new_state;
    }
}
AppEvent::ToggleCollapseLatest => {
    if let Some(last) = self.committed_messages.last_mut() {
        let any_expanded = !last.content_collapsed || !last.tool_collapsed;
        let new_state = any_expanded;
        last.content_collapsed = new_state;
        last.tool_collapsed = new_state;
    }
}
```

- [ ] **Step 4: Check compilation**

Run: `cargo check --bin wgenty-code 2>&1 | head -30`
Expected: clean compile.

---

### Task 4: Render collapsed paragraphs in TUI chat

**Files:**
- Modify: `src/tui/components/chat.rs`

- [ ] **Step 1: Add collapsed rendering helper**

Add this function in `src/tui/components/chat.rs`:

```rust
/// Render a collapsed paragraph: first 3 lines + "... (N lines total, collapsed)" indicator.
fn render_collapsed(lines_buf: &mut Vec<Line<'static>>, content: &str, prefix: &str, prefix_color: Color, max_w: usize) {
    let preview_lines: Vec<&str> = content.lines().take(3).collect();
    let total_lines = content.lines().count();
    for line in &preview_lines {
        push_wrapped(lines_buf, line, prefix, prefix_color, DIM_COLOR, max_w);
    }
    let indicator = format!("   ... ({} lines total, collapsed)", total_lines);
    lines_buf.push(Line::from(Span::styled(indicator, Style::default().fg(DIM_COLOR))));
}
```

- [ ] **Step 2: Modify `message_to_lines` — Assistant branch**

Replace the assistant content rendering to respect `content_collapsed`:

```rust
MessageRole::Assistant => {
    let mut lines = vec![border_top(" Wgenty ", ASSISTANT_HEADER_COLOR, max_w)];
    if msg.content.is_empty() {
        lines.push(Line::from(Span::styled(
            "│ ",
            Style::default().fg(ASSISTANT_COLOR),
        )));
    } else if msg.content_collapsed {
        render_collapsed(&mut lines, &msg.content, "│ ", ASSISTANT_COLOR, max_w);
    } else {
        for line in msg.content.lines() {
            push_wrapped(&mut lines, line, "│ ", ASSISTANT_COLOR, TEXT_COLOR, max_w);
        }
    }
    lines.push(border_bottom(ASSISTANT_COLOR, max_w));
    lines.push(Line::raw(""));
    lines
}
```

- [ ] **Step 3: Modify `message_to_lines` — Tool branch**

Replace the tool rendering to respect `tool_collapsed`:

```rust
MessageRole::Tool => {
    if msg.content.is_empty() {
        // ToolStart placeholder — show "running..." indicator
        let label = msg.tool_name.as_deref().unwrap_or("tool");
        vec![Line::from(vec![Span::styled(
            format!("⚙ {}: running...", label),
            Style::default().fg(TOOL_COLOR),
        )])]
    } else if msg.tool_collapsed {
        let label = msg.tool_name.as_deref().unwrap_or("Tool");
        let mut lines = vec![border_top(&format!(" {} ", label), TOOL_COLOR, max_w)];
        render_collapsed(&mut lines, &msg.content, "│ ", TOOL_COLOR, max_w);
        lines.push(border_bottom(TOOL_COLOR, max_w));
        lines.push(Line::raw(""));
        lines
    } else {
        msg.content
            .lines()
            .map(|line| {
                Line::from(Span::styled(
                    line.to_string(),
                    Style::default().fg(DIM_COLOR),
                ))
            })
            .collect()
    }
}
```

- [ ] **Step 4: Check compilation**

Run: `cargo check --bin wgenty-code 2>&1 | head -30`
Expected: clean compile.

---

### Task 5: Build and verify TUI changes

**Files:**
- N/A (verification only)

- [ ] **Step 1: Full build**

Run: `cargo build --bin wgenty-code 2>&1 | tail -10`
Expected: build success.

- [ ] **Step 2: Commit TUI changes**

```bash
git add src/tui/app.rs src/tui/components/chat.rs
git commit -m "feat(tui): add message collapse/expand with Ctrl+E/Ctrl+O shortcuts"
```

---

### Task 6: Add content_collapsed to GUI ChatMessage

**Files:**
- Modify: `src/gui/chat.rs`

- [ ] **Step 1: Add `content_collapsed` field to `ChatMessage` struct**

```rust
pub struct ChatMessage {
    pub id: String,
    pub role: MessageRole,
    pub content: String,
    pub timestamp: DateTime<Utc>,
    pub is_streaming: bool,
    pub tool_calls: Vec<ToolCall>,
    pub attachments: Vec<Attachment>,
    pub thinking: Option<String>,
    pub thinking_expanded: bool,
    pub content_collapsed: bool,  // NEW
}
```

- [ ] **Step 2: Initialize in `ChatMessage::new`**

```rust
pub fn new(role: MessageRole, content: impl Into<String>) -> Self {
    let content_str = content.into();
    let line_count = content_str.lines().count();
    let content_collapsed = matches!(role, MessageRole::Assistant) && line_count > 50;
    Self {
        id: uuid::Uuid::new_v4().to_string(),
        role,
        content: content_str,
        timestamp: Utc::now(),
        is_streaming: false,
        tool_calls: Vec::new(),
        attachments: Vec::new(),
        thinking: None,
        thinking_expanded: false,
        content_collapsed,
    }
}
```

- [ ] **Step 3: Check compilation**

Run: `cargo check --bin wgenty-code 2>&1 | head -30`
Expected: clean compile.

---

### Task 7: Render collapsed body text in GUI chat

**Files:**
- Modify: `src/gui/chat.rs`

- [ ] **Step 1: Modify `render_claude_message_content` to respect `content_collapsed`**

Replace the body content rendering section:

```rust
// Main content - after thinking block
if message.content_collapsed {
    // Collapsed preview
    let preview_lines: Vec<&str> = message.content.lines().take(3).collect();
    let total_lines = message.content.lines().count();
    Frame::NONE
        .fill(theme.surface_color())
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::same(12))
        .stroke(Stroke::new(1.0, theme.border_color()))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            for line in &preview_lines {
                ui.label(RichText::new(*line).size(14.0).color(theme.muted_text_color()));
            }
            ui.label(
                RichText::new(format!("... ({} lines total, collapsed)", total_lines))
                    .size(12.0)
                    .color(theme.muted_text_color())
                    .italics(),
            );
        });
} else {
    let content = if message.is_streaming {
        format!("{}▌", message.content)
    } else {
        message.content.clone()
    };
    self.render_markdown_content(ui, &content, theme);
}
```

- [ ] **Step 2: Check compilation**

Run: `cargo check --bin wgenty-code 2>&1 | head -30`
Expected: clean compile.

---

### Task 8: Add tool result auto-collapse threshold in GUI

**Files:**
- Modify: `src/gui/tool_calls.rs`

- [ ] **Step 1: Add auto-collapse on tool result in `render_tool_call_card`**

In the `render_tool_call_card` method, after a result is set and expanded content section, add threshold logic. Find where `call.expanded` is initially `true` in `ToolCall::new()`:

In `ToolCall::new()`:
```rust
pub fn new(name: impl Into<String>, arguments: impl Into<String>) -> Self {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    Self {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.into(),
        arguments: arguments.into(),
        status: ToolCallStatus::Pending,
        result: None,
        timestamp,
        expanded: true,
    }
}
```

Modify `with_result` and `with_error` to auto-collapse when result exceeds threshold:

```rust
pub fn with_result(mut self, result: impl Into<String>) -> Self {
    let result_str = result.into();
    let line_count = result_str.lines().count();
    self.result = Some(result_str);
    self.status = ToolCallStatus::Success;
    self.expanded = line_count <= 10; // collapse if > 10 lines
    self
}

pub fn with_error(mut self, error: impl Into<String>) -> Self {
    let error_str = error.into();
    let line_count = error_str.lines().count();
    self.result = Some(error_str);
    self.status = ToolCallStatus::Error;
    self.expanded = line_count <= 10; // collapse if > 10 lines
    self
}
```

- [ ] **Step 2: Check compilation**

Run: `cargo check --bin wgenty-code 2>&1 | head -30`
Expected: clean compile.

---

### Task 9: Add Ctrl+E / Ctrl+O keyboard shortcuts in GUI

**Files:**
- Modify: `src/gui/app.rs` (`WgentyCodeApp::update`)

- [ ] **Step 1: Add keyboard shortcut handling in `update` method**

In `WgentyCodeApp::update`, add a keyboard input check at the top (before processing messages):

```rust
fn update(&mut self, ctx: &Context, _frame: &mut Frame) {
    // Keyboard shortcuts (global)
    ctx.input(|i| {
        if i.key_pressed(egui::Key::E) && i.modifiers.ctrl && !i.modifiers.shift {
            // Ctrl+E: toggle collapse all messages
            let any_expanded = self.chat_panel.messages.iter().any(|m| {
                !m.content_collapsed || m.tool_calls.iter().any(|tc| tc.expanded)
            });
            let new_state = any_expanded; // true = collapse all
            for msg in &mut self.chat_panel.messages {
                msg.content_collapsed = new_state;
                msg.thinking_expanded = !new_state;
                for tc in &mut msg.tool_calls {
                    tc.expanded = !new_state;
                }
            }
        }
        if i.key_pressed(egui::Key::O) && i.modifiers.ctrl && !i.modifiers.shift {
            // Ctrl+O: toggle collapse latest message only
            if let Some(last) = self.chat_panel.messages.last_mut() {
                let any_expanded = !last.content_collapsed
                    || !last.thinking_expanded
                    || last.tool_calls.iter().any(|tc| tc.expanded);
                let new_state = any_expanded;
                last.content_collapsed = new_state;
                last.thinking_expanded = !new_state;
                for tc in &mut last.tool_calls {
                    tc.expanded = !new_state;
                }
            }
        }
    });

    // Process pending messages
    self.process_messages(ctx);
    // ... rest unchanged
}
```

- [ ] **Step 2: Check compilation**

Run: `cargo check --bin wgenty-code 2>&1 | head -30`
Expected: clean compile.

---

### Task 10: Build and final verification

**Files:**
- N/A (verify only)

- [ ] **Step 1: Full build**

Run: `cargo build --bin wgenty-code 2>&1 | tail -10`
Expected: build success.

- [ ] **Step 2: Commit GUI changes**

```bash
git add src/gui/chat.rs src/gui/tool_calls.rs src/gui/app.rs
git commit -m "feat(gui): add message collapse/expand with Ctrl+E/Ctrl+O shortcuts"
```

---

### TUI Verification Notes

To manually verify the TUI:
1. Run `cargo run --bin wgenty-code`
2. Send a message that produces a long assistant response (> 50 lines) → verify it's auto-collapsed
3. Send a message that produces a tool call with > 10 lines of output → verify it's auto-collapsed
4. Press `Ctrl+E` → verify all paragraphs collapse (if any expanded) or expand (if all collapsed)
5. Press `Ctrl+O` → verify only the latest message's paragraphs toggle

### GUI Verification Notes

To manually verify the GUI:
1. Run `cargo run --bin wgenty-code-gui`
2. Same verification steps as TUI above
3. Click individual paragraph headers to manually toggle
