# Message Collapse/Expand Design

## Summary

Add fold/collapse capability for long tool call outputs and LLM responses in the chat interface, with global keyboard shortcuts for expanding/collapsing all messages and the latest message.

## Motivation

Tool call results and long assistant responses flood the chat view with excessive output. Currently:
- **TUI (ratatui)**: All content is rendered fully expanded with no folding.
- **GUI (egui)**: Thinking blocks and tool calls have per-item expand/collapse but no global shortcuts; assistant text content cannot be collapsed.

## Design

### Collapse Granularity (Mixed Mode)

Each paragraph (thinking, assistant body text, tool call) folds independently. Keyboard shortcuts control all paragraphs globally.

### Auto-Collapse Thresholds

| Paragraph type | Threshold | Behavior |
|---------------|-----------|----------|
| Assistant body text | > 50 lines | Initially collapsed, shows 3-line preview + line count |
| Tool call result | > 10 lines | Initially collapsed, shows 3-line preview + line count |
| Thinking block / short content | below threshold | Initially expanded |

### Data Structures

**TUI** — extend `UIMessage`:

```rust
pub struct UIMessage {
    pub role: MessageRole,
    pub content: String,
    pub tool_name: Option<String>,
    // NEW
    pub content_collapsed: bool,   // body text fold state
    pub tool_collapsed: bool,      // tool result fold state
}
```

**GUI** — extend `ChatMessage` (already has `thinking_expanded`):

```rust
pub content_collapsed: bool,  // body text fold state
```

`ToolCall.expanded` already exists in `src/gui/tool_calls.rs`.

### Collapsed Rendering

A collapsed paragraph shows: paragraph type label, first 3 lines as preview, line count indicator, and a fold icon (`▶`).

```
┌ Tool: bash ──────────────────────────────────────────────────┐
│ $ cargo build --release                                      │
│ Compiling claude-code v0.1.0                                 │
│ ... (342 lines, collapsed)                                    │
└──────────────────────────────────────────────────────────────┘
```

### Keyboard Shortcuts

| Shortcut | Behavior | Scope |
|----------|----------|-------|
| `Ctrl+E` | Toggle all collapsible paragraphs: if any paragraph is expanded, collapse all; otherwise expand all | All committed messages + streaming content |
| `Ctrl+O` | Toggle only the latest message's paragraphs: if any of its paragraphs is expanded, collapse all; otherwise expand all | Last message in the list only |

Behavior notes:
- `Ctrl+E` and `Ctrl+O` do not interfere with each other.
- On a fresh conversation with auto-collapsed long content, first `Ctrl+E` expands all (since some are collapsed); second `Ctrl+E` collapses all.
- Per-paragraph manual click/click toggle is still available.
- The two shortcuts operate independently: `Ctrl+O` may leave older messages in a different state than `Ctrl+E`.

### Files to Modify

| File | Changes |
|------|---------|
| `src/tui/app.rs` | Add `content_collapsed` and `tool_collapsed` fields to `UIMessage`; add `Ctrl+E` and `Ctrl+O` key handlers; initialize collapse state based on threshold on message commit |
| `src/tui/components/chat.rs` | Render collapsed paragraphs with 3-line preview + line count; support fold toggle via click/keyboard |
| `src/gui/chat.rs` | Add `content_collapsed` to `ChatMessage`; render collapsed body text; add `Ctrl+E`/`Ctrl+O` key handlers in `gui/app.rs` |
| `src/gui/app.rs` | Keyboard shortcut dispatch for `Ctrl+E`/`Ctrl+O` |
| `src/gui/tool_calls.rs` | Add 10-line threshold for auto-collapse on tool results |

### Auto-Collapse Logic

When a new message is committed (TUI `StreamDone`, GUI message append):
1. Count lines in content.
2. For assistant body text: if > 50 lines, set `content_collapsed = true`.
3. For tool results: if > 10 lines, set `tool_collapsed = true`.
4. For thinking blocks in GUI: existing behavior preserved (default expanded).

### Shortcut Implementation Detail

**Ctrl+E** (toggle all):
```
fn handle_collapse_all(messages: &mut [UIMessage]) {
    let any_expanded = messages.iter().any(|m| !m.content_collapsed || !m.tool_collapsed);
    let new_state = any_expanded; // true = collapse all
    for m in messages {
        m.content_collapsed = new_state;
        m.tool_collapsed = new_state;
    }
}
```

**Ctrl+O** (toggle latest only):
```
fn handle_collapse_latest(messages: &mut [UIMessage]) {
    if let Some(last) = messages.last_mut() {
        let any_expanded = !last.content_collapsed || !last.tool_collapsed;
        let new_state = any_expanded;
        last.content_collapsed = new_state;
        last.tool_collapsed = new_state;
    }
}
```

### Scope

- TUI (ratatui) frontend: full implementation.
- GUI (egui) frontend: full implementation.
- No changes to backend, daemon, API, or web templates needed.

### Edge Cases

- Streaming content: considered a paragraph; `Ctrl+E` includes it; `Ctrl+O` treats it as part of the latest message.
- Empty messages: collapse state is irrelevant; skip in toggle logic.
- Messages below threshold: can still be collapsed via shortcut even though they start expanded.
