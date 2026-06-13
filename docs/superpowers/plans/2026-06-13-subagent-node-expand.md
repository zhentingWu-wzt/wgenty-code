# Subagent Node Expand — Interactive Full History

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add keyboard navigation and node-expand to the subagent monitor panel so users can see a subagent's full think→call→think→call history.

**Architecture:** Replace `SubagentAction` (tool calls only) with `SubagentEvent` enum (Thought + Action), bump action_log cap to 50. Rewrite `subagent_panel.rs` as a stateful component with selection and expand/collapse. Wire keyboard events from `App` into the panel state.

**Tech Stack:** Rust, ratatui, existing SubagentTree/SubagentProgress types

---

### File Map

| File | Role |
|------|------|
| `agent/progress.rs` | `SubagentEvent` enum + updated `SubagentProgress` |
| `teams/subagent_loop.rs` | Emit Thought + Action events into action_log |
| `tui/components/chat.rs` | Render events from action_log (Thought vs Action) |
| `tui/components/subagent_panel.rs` | Stateful interactive panel component |
| `tui/app/mod.rs` | Add `SubagentPanelState` to `App` |
| `tui/app/types.rs` | Add `SubagentPanelKey` event |
| `tui/app/event.rs` | Keyboard dispatch to panel |

---

### Task 1: Data model — SubagentAction → SubagentEvent

**Files:**
- Modify: `src/agent/progress.rs` (lines 12-44)

- [ ] **Step 1: Replace SubagentAction with SubagentEvent**

Replace the `SubagentAction` struct with the `SubagentEvent` enum, and update `action_log` type + comment:

```rust
// Delete lines 12-21 (SubagentAction struct) and replace with:

/// An event in a subagent's execution timeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentEvent {
    pub event_type: SubagentEventType,
    /// Milliseconds elapsed since subagent started when this event occurred.
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SubagentEventType {
    /// The model output text (analysis, planning, conclusion).
    /// Text is truncated to 200 chars before storage.
    Thought { text: String },
    /// The model called a tool.
    Action {
        tool_name: String,
        params_summary: String,
    },
}
```

- [ ] **Step 2: Update action_log field in SubagentProgress**

```rust
// Replace line 37:
    /// Execution event timeline (earliest → latest), max 50 entries.
    pub action_log: Vec<SubagentEvent>,
```

- [ ] **Step 3: Compile check**

```bash
cargo check 2>&1
```

Expected: many compile errors (all sites that reference `SubagentAction`). This is expected — we fix them in subsequent tasks.

- [ ] **Step 4: Commit**

```bash
git add src/agent/progress.rs
git commit -m "refactor: replace SubagentAction with SubagentEvent enum for full event timeline"
```

---

### Task 2: Update all SubagentProgress construction sites

**Files:**
- Modify: `src/tui/components/subagent_tree.rs:105-124` (test helper)
- Modify: `src/tools/meta/task.rs` (3 construction sites)
- Modify: `src/tools/meta/rlm/mod.rs` (1 construction site)
- Modify: `src/teams/subagent_loop.rs` (2 construction sites)

All sites currently set `action_log: Vec::new()`. The type changes but initial value stays empty `Vec<SubagentEvent>`.

- [ ] **Step 1: Update subagent_tree.rs test helper**

```rust
// src/tui/components/subagent_tree.rs line 118, change:
            action_log: Vec::new(),
// stays the same — Vec::new() works for Vec<SubagentEvent>
```

No code change needed — `Vec::new()` infers correctly. Just verify.

- [ ] **Step 2: Verify compile**

```bash
cargo check 2>&1
```

Expected: errors only in `subagent_loop.rs` and `chat.rs` where `SubagentAction` is explicitly referenced. Other sites should be fine.

- [ ] **Step 3: Commit**

```bash
git add -u
git commit -m "chore: update SubagentProgress construction sites for SubagentEvent"
```

---

### Task 3: Update subagent_loop.rs — emit Thought + Action events

**Files:**
- Modify: `src/teams/subagent_loop.rs` (imports, state vars, emit closure, action log append)

- [ ] **Step 1: Update imports**

```rust
// Line 7, change:
use crate::agent::progress::{ProgressCallback, SubagentAction, SubagentMetadata, SubagentProgress, SubagentStatus};
// to:
use crate::agent::progress::{ProgressCallback, SubagentEvent, SubagentEventType, SubagentMetadata, SubagentProgress, SubagentStatus};
```

- [ ] **Step 2: Update state variable type**

```rust
// Line ~155, change:
        let action_log: Mutex<Vec<SubagentAction>> = Mutex::new(Vec::new());
// to:
        let action_log: Mutex<Vec<SubagentEvent>> = Mutex::new(Vec::new());
```

- [ ] **Step 3: Update emit closure — use action_log and text_snapshot from state**

The emit closure at lines 158-184 already reads `action_log.lock().unwrap().clone()` and `text_snapshot.lock().unwrap().clone()`. No change needed — types match.

- [ ] **Step 4: Emit Thought event after API response text is captured**

After the text_snapshot is captured (~line 193), append a Thought event to action_log:

```rust
// After: *text_snapshot.lock().unwrap() = Some(snapshot);
// Add:
                {
                    let mut log = action_log.lock().unwrap();
                    let elapsed = start.elapsed().as_millis() as u64;
                    log.push(SubagentEvent {
                        event_type: SubagentEventType::Thought {
                            text: snapshot.clone(),
                        },
                        elapsed_ms: elapsed,
                    });
                    if log.len() > 50 {
                        log.remove(0); // drop oldest
                    }
                }
```

- [ ] **Step 5: Update tool-call Action event — replace SubagentAction with SubagentEvent**

Replace the block at lines ~329-339:

```rust
// Replace:
                {
                    let mut log = action_log.lock().unwrap();
                    log.insert(0, SubagentAction {
                        tool_name: tool_name.clone(),
                        params_summary,
                        timestamp_ms: chrono::Utc::now().timestamp_millis(),
                    });
                    log.truncate(10);
                }

// With:
                {
                    let mut log = action_log.lock().unwrap();
                    let elapsed = start.elapsed().as_millis() as u64;
                    log.push(SubagentEvent {
                        event_type: SubagentEventType::Action {
                            tool_name: tool_name.clone(),
                            params_summary,
                        },
                        elapsed_ms: elapsed,
                    });
                    if log.len() > 50 {
                        log.remove(0); // drop oldest
                    }
                }
```

- [ ] **Step 6: Compile check**

```bash
cargo check 2>&1
```

Expected: errors only in `chat.rs` (references `SubagentAction` directly).

- [ ] **Step 7: Commit**

```bash
git add src/teams/subagent_loop.rs
git commit -m "feat: emit Thought+Action SubagentEvent timeline from subagent loop"
```

---

### Task 4: Update chat.rs — render SubagentEvent instead of SubagentAction

**Files:**
- Modify: `src/tui/components/chat.rs` (lines 188-202)

- [ ] **Step 1: Update action log rendering**

Replace lines 188-202 with event-type-aware rendering:

```rust
        // ── Recent action log (collapsed: last 3 events) ──────────────────
        let recent_events: Vec<&SubagentEvent> = node.progress.action_log.iter().rev().take(3).collect();
        if !recent_events.is_empty() {
            for event in recent_events.iter().rev() {
                match &event.event_type {
                    crate::agent::progress::SubagentEventType::Action { tool_name, params_summary } => {
                        let action_str = if params_summary.is_empty() {
                            format!("{}", tool_name)
                        } else {
                            format!("{}(\"{}\")", tool_name, params_summary)
                        };
                        lines.push(Line::from(vec![Span::styled(
                            format!("{}   ▸ {}", snapshot_indent, action_str),
                            Style::default().fg(Color::Rgb(108, 112, 134)),
                        )]));
                    }
                    crate::agent::progress::SubagentEventType::Thought { .. } => {
                        // Thoughts are rendered via text_snapshot above; skip here.
                    }
                }
            }
        }
```

- [ ] **Step 2: Compile check**

```bash
cargo check 2>&1
```

Expected: clean compile (no errors).

- [ ] **Step 3: Run tests**

```bash
cargo test --lib 2>&1
```

Expected: 96 tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/tui/components/chat.rs
git commit -m "feat: render SubagentEvent timeline in tree nodes (collapsed mode)"
```

---

### Task 5: Add SubagentPanelState to App

**Files:**
- Create: `src/tui/components/subagent_panel_state.rs`
- Modify: `src/tui/app/mod.rs`
- Modify: `src/tui/components/mod.rs`

- [ ] **Step 1: Create SubagentPanelState**

```rust
// src/tui/components/subagent_panel_state.rs

use super::subagent_tree::SubagentTree;
use std::collections::HashSet;

/// Interactive state for the subagent monitor panel.
#[derive(Debug, Clone, Default)]
pub struct SubagentPanelState {
    /// Index into the flattened node list (0-based).
    pub selected_index: usize,
    /// Node IDs that are currently expanded.
    pub expanded_nodes: HashSet<String>,
    /// Vertical scroll offset for the panel body.
    pub scroll_offset: u16,
}

impl SubagentPanelState {
    /// Build a depth-first flattened list of node IDs from the tree.
    pub fn node_list(tree: &SubagentTree) -> Vec<String> {
        let mut list = Vec::new();
        fn walk(tree: &SubagentTree, node_id: &str, list: &mut Vec<String>) {
            list.push(node_id.to_string());
            if let Some(node) = tree.nodes.get(node_id) {
                for child in &node.children {
                    walk(tree, child, list);
                }
            }
        }
        if let Some(ref root) = tree.root_id {
            walk(tree, root, &mut list);
        }
        list
    }

    /// Move selection to the previous node (wrap-around).
    pub fn move_up(&mut self, tree: &SubagentTree) {
        let list = Self::node_list(tree);
        if list.is_empty() { return; }
        if self.selected_index == 0 {
            self.selected_index = list.len() - 1;
        } else {
            self.selected_index -= 1;
        }
        self.scroll_offset = 0;
    }

    /// Move selection to the next node (wrap-around).
    pub fn move_down(&mut self, tree: &SubagentTree) {
        let list = Self::node_list(tree);
        if list.is_empty() { return; }
        self.selected_index = (self.selected_index + 1) % list.len();
        self.scroll_offset = 0;
    }

    /// Jump to first node.
    pub fn move_first(&mut self, _tree: &SubagentTree) {
        self.selected_index = 0;
        self.scroll_offset = 0;
    }

    /// Jump to last node.
    pub fn move_last(&mut self, tree: &SubagentTree) {
        let list = Self::node_list(tree);
        if !list.is_empty() {
            self.selected_index = list.len() - 1;
        }
        self.scroll_offset = 0;
    }

    /// Toggle expand/collapse for the currently selected node.
    pub fn toggle_expand(&mut self, tree: &SubagentTree) {
        let list = Self::node_list(tree);
        if let Some(node_id) = list.get(self.selected_index) {
            if self.expanded_nodes.contains(node_id) {
                self.expanded_nodes.remove(node_id);
            } else {
                self.expanded_nodes.insert(node_id.clone());
            }
        }
    }

    /// Whether a given node is currently expanded.
    pub fn is_expanded(&self, node_id: &str) -> bool {
        self.expanded_nodes.contains(node_id)
    }

    /// Get the currently selected node_id, if any.
    pub fn selected_node_id<'a>(&self, tree: &'a SubagentTree) -> Option<&'a str> {
        let list = Self::node_list(tree);
        list.get(self.selected_index).map(|s| s.as_str())
    }

    /// Reset all state (called when panel closes or new turn starts).
    pub fn reset(&mut self) {
        self.selected_index = 0;
        self.expanded_nodes.clear();
        self.scroll_offset = 0;
    }
}
```

- [ ] **Step 2: Register module**

In `src/tui/components/mod.rs`, add:
```rust
pub mod subagent_panel_state;
```

- [ ] **Step 3: Add field to App**

In `src/tui/app/mod.rs`, add after `subagent_panel_visible`:
```rust
    /// Interactive state for the subagent monitor panel.
    pub subagent_panel_state: SubagentPanelState,
```

Add the import at top of mod.rs:
```rust
use crate::tui::components::subagent_panel_state::SubagentPanelState;
```

Initialize in `App::new()`:
```rust
            subagent_panel_state: SubagentPanelState::default(),
```

- [ ] **Step 4: Reset on new turn**

In `src/tui/app/event.rs`, in the `AppEvent::Submit(text)` handler (line ~216), add:
```rust
            AppEvent::Submit(text) => {
                self.subagent_tree.clear();
                self.subagent_panel_state.reset();
                // ... rest of existing code
```

- [ ] **Step 5: Compile check**

```bash
cargo check 2>&1
```

Expected: clean compile.

- [ ] **Step 6: Commit**

```bash
git add src/tui/components/subagent_panel_state.rs src/tui/components/mod.rs src/tui/app/mod.rs src/tui/app/event.rs
git commit -m "feat: add SubagentPanelState for interactive node selection/expansion"
```

---

### Task 6: Add SubagentPanelKey event + keyboard dispatch

**Files:**
- Modify: `src/tui/app/types.rs`
- Modify: `src/tui/app/event.rs`

- [ ] **Step 1: Add event variant**

In `src/tui/app/types.rs`, add to the `AppEvent` enum (after `ToggleSubagentPanel`):
```rust
    /// A key event when the subagent panel is visible.
    SubagentPanelKey(ratatui::crossterm::event::KeyEvent),
```

- [ ] **Step 2: Wire keyboard dispatch in event.rs**

In `src/tui/app/event.rs`, in the `AppEvent::KeyEvent(key)` handler (line ~27), add a branch BEFORE the general key handling. After the `Ctrl+Shift+T` check at line ~64:

```rust
                // If subagent panel is visible, route keys to it
                if self.subagent_panel_visible {
                    match key.code {
                        KeyCode::Esc => {
                            self.subagent_panel_visible = false;
                            return;
                        }
                        KeyCode::Char('j') | KeyCode::Down => {
                            self.subagent_panel_state.move_down(&self.subagent_tree);
                            return;
                        }
                        KeyCode::Char('k') | KeyCode::Up => {
                            self.subagent_panel_state.move_up(&self.subagent_tree);
                            return;
                        }
                        KeyCode::Enter => {
                            self.subagent_panel_state.toggle_expand(&self.subagent_tree);
                            return;
                        }
                        KeyCode::Char('g') => {
                            self.subagent_panel_state.move_first(&self.subagent_tree);
                            return;
                        }
                        KeyCode::Char('G') => {
                            self.subagent_panel_state.move_last(&self.subagent_tree);
                            return;
                        }
                        _ => {} // pass through
                    }
                }
```

- [ ] **Step 3: Compile check**

```bash
cargo check 2>&1
```

Expected: clean compile.

- [ ] **Step 4: Commit**

```bash
git add src/tui/app/types.rs src/tui/app/event.rs
git commit -m "feat: add keyboard navigation for subagent panel (j/k/Enter/Esc/g/G)"
```

---

### Task 7: Rewrite subagent_panel.rs — stateful rendering with expand

**Files:**
- Modify: `src/tui/components/subagent_panel.rs` (full rewrite)
- Modify: `src/tui/app/render.rs` (line 87-93)

- [ ] **Step 1: Rewrite subagent_panel.rs**

Replace the entire file with:

```rust
//! Subagent Monitor Panel — interactive panel (Ctrl+Shift+T) with node expand.

use super::subagent_panel_state::SubagentPanelState;
use super::subagent_tree::SubagentTree;
use crate::agent::progress::{SubagentEventType, SubagentStatus};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

pub fn render(
    f: &mut Frame,
    area: Rect,
    tree: &SubagentTree,
    state: &SubagentPanelState,
    is_executing: bool,
) {
    let panel = Block::default()
        .title(format!(
            " 🌳 Subagent Monitor — {} agents · {} active — Esc close ",
            tree.nodes.len(),
            tree.count_by_status(SubagentStatus::Running),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(203, 166, 247)))
        .style(Style::default().bg(Color::Rgb(26, 26, 46)));

    let inner = panel.inner(area);
    f.render_widget(panel, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),  // summary bar
            Constraint::Min(0),     // tree body
            Constraint::Length(1),  // help bar
        ])
        .split(inner);

    // Summary bar
    let done = tree.count_by_status(SubagentStatus::Completed);
    let running = tree.count_by_status(SubagentStatus::Running);
    let pending = tree.count_by_status(SubagentStatus::Pending);
    let failed = tree.count_by_status(SubagentStatus::Failed);
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!(" ✅ {} done  ", done), Style::default().fg(Color::Rgb(166, 227, 161))),
            Span::styled(format!("🔄 {} running  ", running), Style::default().fg(Color::Rgb(249, 226, 175))),
            Span::styled(format!("⏳ {} pending  ", pending), Style::default().fg(Color::Rgb(108, 112, 134))),
            Span::styled(format!("❌ {} failed", failed), Style::default().fg(Color::Rgb(243, 139, 168))),
        ])),
        chunks[0],
    );

    // Tree body with expand support
    let mut tree_lines: Vec<Line> = Vec::new();
    let selected_id = state.selected_node_id(tree).map(|s| s.to_string());
    render_tree_with_expand(
        &mut tree_lines,
        tree,
        state,
        tree.root_id.as_deref(),
        0,
        4u16,
        selected_id.as_deref(),
    );
    f.render_widget(
        Paragraph::new(ratatui::text::Text::from(tree_lines)).wrap(Wrap { trim: false }),
        chunks[1],
    );

    // Help bar
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" ↑↓ navigate  ", Style::default().fg(Color::Rgb(108, 112, 134))),
            Span::styled("Enter expand  ", Style::default().fg(Color::Rgb(108, 112, 134))),
            Span::styled("g/G top/bottom  ", Style::default().fg(Color::Rgb(108, 112, 134))),
            Span::styled("Esc close", Style::default().fg(Color::Rgb(108, 112, 134))),
        ])),
        chunks[2],
    );
}

/// Recursively render tree nodes with selection highlight and expand.
fn render_tree_with_expand(
    lines: &mut Vec<Line>,
    tree: &SubagentTree,
    state: &SubagentPanelState,
    node_id: Option<&str>,
    depth: u16,
    base_indent: u16,
    selected_id: Option<&str>,
) {
    let Some(nid) = node_id else { return };
    let Some(node) = tree.nodes.get(nid) else { return };

    let indent = base_indent + depth * 2;
    let is_selected = selected_id == Some(nid);
    let is_expanded = state.is_expanded(nid);
    let prefix = if depth == 0 { if is_expanded { "▶" } else { "▸" } } else { if is_expanded { "▶" } else { "▸" } };
    let indent_str = " ".repeat(indent as usize);

    let icon = match node.progress.status {
        SubagentStatus::Pending => "⏳",
        SubagentStatus::Running => "🔄",
        SubagentStatus::Completed => "✅",
        SubagentStatus::Failed => "❌",
        SubagentStatus::Cancelled => "🚫",
    };

    let color = match node.progress.status {
        SubagentStatus::Running => Color::Rgb(249, 226, 175),
        SubagentStatus::Completed => Color::Rgb(166, 227, 161),
        SubagentStatus::Failed | SubagentStatus::Cancelled => Color::Rgb(243, 139, 168),
        SubagentStatus::Pending => Color::Rgb(108, 112, 134),
    };

    let select_style = if is_selected {
        Style::default().fg(Color::Rgb(203, 166, 247)).add_modifier(ratatui::style::Modifier::BOLD)
    } else {
        Style::default().fg(color)
    };

    // ── Node header line ─────────────────────────────────────────────
    let elapsed_secs = node.progress.elapsed_ms as f64 / 1000.0;
    let status_detail = match node.progress.status {
        SubagentStatus::Running => match (node.progress.round, node.progress.max_rounds) {
            (Some(r), Some(mr)) => format!("round {}/{} · {:.1}s", r, mr, elapsed_secs),
            _ => format!("{:.1}s", elapsed_secs),
        },
        SubagentStatus::Completed => {
            let mut s = node.progress.round.map(|r| format!("{} rounds", r)).unwrap_or_default();
            if !s.is_empty() { s.push_str(" · "); }
            s.push_str(&format!("{:.1}s", elapsed_secs));
            if let Some(ref meta) = node.progress.metadata {
                if let Some(tc) = meta.token_count {
                    if tc >= 1000 {
                        s.push_str(&format!(" · {:.1}k tokens", tc as f64 / 1000.0));
                    } else {
                        s.push_str(&format!(" · {} tokens", tc));
                    }
                }
            }
            s
        }
        _ => String::new(),
    };

    let label = if status_detail.is_empty() {
        format!(" {}", node.progress.label)
    } else {
        format!(" {} — {}", node.progress.label, status_detail)
    };

    lines.push(Line::from(vec![
        Span::styled(format!("{}{} ", indent_str, prefix), Style::default().fg(Color::Rgb(108, 112, 134))),
        Span::styled(icon, select_style),
        Span::styled(label, select_style),
    ]));

    // ── Expanded: show full event timeline ──────────────────────────
    if is_expanded {
        let event_indent = " ".repeat((indent + 2) as usize);
        let events: Vec<&crate::agent::progress::SubagentEvent> =
            node.progress.action_log.iter().collect();

        if events.is_empty() {
            lines.push(Line::from(vec![Span::styled(
                format!("{}│", event_indent),
                Style::default().fg(Color::Rgb(80, 80, 100)),
            )]));
            lines.push(Line::from(vec![Span::styled(
                format!("{}💭 thinking…", event_indent),
                Style::default().fg(Color::Rgb(108, 112, 134)),
            )]));
            lines.push(Line::from(vec![Span::styled(
                format!("{}▼", event_indent),
                Style::default().fg(Color::Rgb(80, 80, 100)),
            )]));
        } else {
            for (i, event) in events.iter().enumerate() {
                let connector = if i == events.len() - 1 { "▼" } else { "│" };
                match &event.event_type {
                    SubagentEventType::Thought { text } => {
                        let preview: String = text.chars().take(140).collect();
                        let display = if text.len() > 140 {
                            format!("{}…", preview)
                        } else {
                            preview
                        };
                        lines.push(Line::from(vec![Span::styled(
                            format!("{}{}", event_indent, connector),
                            Style::default().fg(Color::Rgb(80, 80, 100)),
                        )]));
                        lines.push(Line::from(vec![Span::styled(
                            format!("{}💭 {}  ({:.1}s)", event_indent, display, event.elapsed_ms as f64 / 1000.0),
                            Style::default().fg(Color::Rgb(180, 180, 200)),
                        )]));
                    }
                    SubagentEventType::Action { tool_name, params_summary } => {
                        let action_str = if params_summary.is_empty() {
                            format!("{}", tool_name)
                        } else {
                            format!("{}(\"{}\")", tool_name, params_summary)
                        };
                        lines.push(Line::from(vec![Span::styled(
                            format!("{}{}", event_indent, connector),
                            Style::default().fg(Color::Rgb(80, 80, 100)),
                        )]));
                        lines.push(Line::from(vec![Span::styled(
                            format!("{}▸ {}  ({:.1}s)", event_indent, action_str, event.elapsed_ms as f64 / 1000.0),
                            Style::default().fg(Color::Rgb(137, 180, 250)),
                        )]));
                    }
                }
            }
        }
        lines.push(Line::default()); // blank line after expanded section
    }

    // ── Collapsed: show compact info ────────────────────────────────
    if node.progress.status == SubagentStatus::Running {
        let detail_indent = " ".repeat((indent + 2) as usize);
        // Current tool
        if let Some(ref tool) = node.progress.current_tool {
            let tool_label = if let Some(ref params) = node.progress.current_params {
                if params.is_empty() {
                    format!("executing: {}", tool)
                } else {
                    format!("executing: {}(\"{}\")", tool, params)
                }
            } else {
                format!("executing: {}", tool)
            };
            lines.push(Line::from(vec![Span::styled(
                format!("{}└─ 🛠 {}", detail_indent, tool_label),
                Style::default().fg(Color::Rgb(137, 180, 250)),
            )]));
        }

        // Text snapshot
        if let Some(ref snapshot) = node.progress.text_snapshot {
            let preview: String = snapshot.chars().take(100).collect();
            let display = if snapshot.len() > 100 {
                format!("{}…", preview)
            } else {
                preview
            };
            lines.push(Line::from(vec![Span::styled(
                format!("{}   💬 {}", detail_indent, display),
                Style::default().fg(Color::Rgb(150, 150, 165)),
            )]));
        } else {
            lines.push(Line::from(vec![Span::styled(
                format!("{}   💭 thinking…", detail_indent),
                Style::default().fg(Color::Rgb(150, 150, 165)),
            )]));
        }

        // Recent 3 actions
        let recent: Vec<_> = node.progress.action_log.iter()
            .filter(|e| matches!(e.event_type, SubagentEventType::Action { .. }))
            .rev().take(3).collect();
        for event in recent.iter().rev() {
            if let SubagentEventType::Action { tool_name, params_summary } = &event.event_type {
                let action_str = if params_summary.is_empty() {
                    format!("{}", tool_name)
                } else {
                    format!("{}(\"{}\")", tool_name, params_summary)
                };
                lines.push(Line::from(vec![Span::styled(
                    format!("{}   ▸ {}", detail_indent, action_str),
                    Style::default().fg(Color::Rgb(108, 112, 134)),
                )]));
            }
        }
    }

    // Recurse into children
    for child_id in &node.children {
        render_tree_with_expand(lines, tree, state, Some(child_id), depth + 1, base_indent, selected_id);
    }
}
```

- [ ] **Step 2: Update render.rs call site**

In `src/tui/app/render.rs`, update lines 87-93:

```rust
// Change:
            components::subagent_panel::render(
                f,
                panel_area,
                &self.subagent_tree,
                self.subagent_tree.count_by_status(SubagentStatus::Running) > 0,
            );
// To:
            let is_executing = self.subagent_tree.count_by_status(SubagentStatus::Running) > 0;
            components::subagent_panel::render(
                f,
                panel_area,
                &self.subagent_tree,
                &self.subagent_panel_state,
                is_executing,
            );
```

- [ ] **Step 3: Compile check**

```bash
cargo check 2>&1
```

Expected: clean compile. Fix any issues.

- [ ] **Step 4: Run tests**

```bash
cargo test --lib 2>&1
```

Expected: all existing tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/tui/components/subagent_panel.rs src/tui/app/render.rs
git commit -m "feat: rewrite subagent panel with keyboard nav, node selection, expand for full event timeline"
```

---

### Task 8: Final verification

- [ ] **Step 1: Run full test suite**

```bash
cargo test --lib 2>&1
```

Expected: all tests pass.

- [ ] **Step 2: Check formatting**

```bash
cargo fmt
```

- [ ] **Step 3: Manual verification**

```bash
cargo run -- repl
```

1. Trigger a task that uses subagents
2. Press Ctrl+Shift+T to open the panel
3. Use j/k to navigate between nodes (selected node should be highlighted)
4. Press Enter on a node to expand full history
5. Verify Thought and Action events display with timestamps
6. Press Enter again to collapse
7. Press Esc to close the panel

- [ ] **Step 4: Commit final cleanup**

```bash
git add -u
git commit -m "chore: formatting and final cleanup for subagent node expand"
```

---

## Self-Review

- [x] Spec coverage: All design doc requirements covered — SubagentEvent enum (Task 1), Thought emit (Task 3), 50-entry cap (Task 3), keyboard nav (Task 6), expand/collapse rendering (Task 7), SubagentPanelState (Task 5)
- [x] No placeholders: Every step has exact code or commands
- [x] Type consistency: `SubagentEvent` used consistently across Tasks 1-7; `SubagentPanelState` methods match call sites in Tasks 6-7
