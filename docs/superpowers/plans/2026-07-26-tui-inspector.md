# Implementation Plan: TUI Request Context Inspector

> 2026-07-26 | Change: `tui-inspector` | Phase: build
>
> Based on: `docs/superpowers/specs/2026-07-26-tui-inspector-design.md`

---

## Phase 0: Delete Legacy task_panel (5 steps)

### Step 0.1 — Delete `src/tui/components/task_panel.rs`

Remove file entirely.

```bash
rm src/tui/components/task_panel.rs
```

### Step 0.2 — Remove from `src/tui/components/mod.rs`

Delete the `pub mod task_panel;` line.

### Step 0.3 — Remove from `src/tui/app/mod.rs`

- Remove `task_panel: TaskPanel` field from `App` struct (around line 154)
- Remove `task_panel: TaskPanel::new(),` from struct initialization (around line 295)

### Step 0.4 — Remove from `src/tui/app/event.rs`

Delete `ToggleTaskPanel` event handling block around line 923:

```rust
 ToggleTaskPanel => {
    self.task_panel.visible = !self.task_panel.visible;
    if self.task_panel.visible {
        ...
    }
 }
```

### Step 0.5 — Remove from `src/tui/app/render.rs`

- Remove the `// Task panel` block (right-side split pane) around lines 82-120
- Simplify `chat_area` to always be `let chat_area = main_area;` (no split)
- Remove `task_panel.render(f, task_area);`  call (around line 155)

**Verify at each step**:
```bash
cargo check 2>&1 | head -20
```

---

## Phase 1: Data Model Layer (4 steps)

### Step 1.1 — Add data structures to `src/prompts/mod.rs`

Add these types at the end of the module (before `#[cfg(test)]`):

```rust
/// Labels the origin of a system-prompt layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayerSource {
    Builtin,
    ConfigSettings,
    ConfigFile(std::path::PathBuf),
    ProjectFile(std::path::PathBuf),
    MemoryRecall { scope: String },
    SkillInventory,
    HookInjection,
    Unknown,
}

impl std::fmt::Display for LayerSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LayerSource::Builtin => write!(f, "Builtin"),
            LayerSource::ConfigSettings => write!(f, "Config Settings"),
            LayerSource::ConfigFile(p) => write!(f, "~/.wgenty-code/{}", p.display()),
            LayerSource::ProjectFile(p) => write!(f, "<project>/{}", p.display()),
            LayerSource::MemoryRecall { scope } => write!(f, "Memory ({})", scope),
            LayerSource::SkillInventory => write!(f, "Skills"),
            LayerSource::HookInjection => write!(f, "Hooks"),
            LayerSource::Unknown => write!(f, "Unknown"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LayerMeta {
    pub label: String,
    pub source: LayerSource,
    pub content: String,
    pub char_count: usize,
}

/// Metadata for a recalled memory
#[derive(Debug, Clone)]
pub struct MemoryMeta {
    pub scope: String,
    pub importance: f32,
    pub score: Option<f32>,
    pub memory_type: Option<String>,
    pub content_preview: String,
}
```

### Step 1.2 — Modify `AssembledInstructions` struct

Change `src/prompts/mod.rs` line 52-55 from:

```rust
pub struct AssembledInstructions {
    pub system_messages: Vec<ChatMessage>,
}
```

To:

```rust
pub struct AssembledInstructions {
    pub system_messages: Vec<ChatMessage>,
    pub layers: Vec<LayerMeta>,
}
```

Update all `AssembledInstructions { system_messages: ... }` constructors in `assemble_instructions()` to include `layers: vec![]` initially.

### Step 1.3 — Populate LayerMeta in `assemble_instructions()`

In `assemble_instructions()` (`src/prompts/mod.rs` line 423+), after each layer is built into a system message, simultaneously push a corresponding `LayerMeta`. For each builder:

```rust
// Example: base_instructions layer
instructions.push_layer(LayerMeta {
    label: "Layer 1: base_instructions".into(),
    source: LayerSource::Builtin,
    content: base_text.clone(),
    char_count: base_text.len(),
});
instructions.system_messages.push(ChatMessage::system(base_text));
```

Map each layer to its source:

| Layer | Source |
|-------|--------|
| 1. base_instructions | `LayerSource::Builtin` |
| 2. permissions | `LayerSource::Builtin` |
| 3. developer_instructions | `LayerSource::ConfigSettings` |
| 4. environment_context | `LayerSource::Builtin` |
| 5a. agents_md | `LayerSource::ConfigFile(...)` |
| 5b. project memories | `LayerSource::MemoryRecall { scope: "project" }` |
| 5c. global memories | `LayerSource::MemoryRecall { scope: "global" }` |
| 6. collaboration | `LayerSource::ConfigSettings` |
| 7. skills_inventory | `LayerSource::SkillInventory` |
| 8. user WGENTY.md | `LayerSource::ConfigFile(...)` |
| 9. user rules | `LayerSource::ConfigFile(...)` |
| 10. project docs | `LayerSource::ProjectFile(...)` |

### Step 1.4 — Capture ReminderOutput

In `src/prompts/mod.rs`, `process_input_inner()` returns the reminder. Trace how `ReminderOutput` is defined in `build_user_turn_reminder`. 

Add to `ReminderOutput`:
```rust
/// The to_model content of the system reminder
pub fn to_model_text(&self) -> &str {
    // Map to the exported content field
}
```

No struct changes needed if `ReminderOutput` already has the data — just expose a getter.

---

## Phase 2: Turn Snapshots (4 steps)

### Step 2.1 — Add `PartialTurnContext` and `TurnContext` to App

In `src/tui/app/mod.rs`, add imports:

```rust
use crate::prompts::{LayerMeta, LayerSource, MemoryMeta};
```

Add new types (can go in a new `src/tui/app/turn_context.rs` or inline in `mod.rs`):

```rust
#[derive(Debug, Clone)]
pub struct PartialTurnContext {
    pub layers: Vec<LayerMeta>,
    pub memories: Vec<MemoryMeta>,
    pub reminder: Option<crate::prompts::ReminderOutput>,
}

#[derive(Debug, Clone)]
pub struct TurnContext {
    pub turn_index: usize,
    pub layers: Vec<LayerMeta>,
    pub memories: Vec<MemoryMeta>,
    pub reminder: Option<crate::prompts::ReminderOutput>,
    pub full_messages: Vec<crate::api::ChatMessage>,
}

const TURN_CONTEXT_CAPACITY: usize = 50;
```

Add to `App` struct:
```rust
turn_contexts: Vec<TurnContext>,
pending_context: Option<PartialTurnContext>,
```

Initialize in `App::new()`:
```rust
turn_contexts: Vec::with_capacity(TURN_CONTEXT_CAPACITY),
pending_context: None,
```

### Step 2.2 — Change `App.assembled_system_messages` to carry layers

Currently `assembled_system_messages: Vec<ChatMessage>`. Change to carry a full `AssembledInstructions`:

```rust
// Before
assembled_system_messages: Vec<ChatMessage>,

// After
assembled_instructions: AssembledInstructions,
```

Update all references to `self.assembled_system_messages` → `self.assembled_instructions.system_messages`.

### Step 2.3 — Stash `PartialTurnContext` during `spawn_agent_turn`

In `src/tui/app/turn.rs`, `spawn_agent_turn()`:

After `agent.process_input(input_agent)` completes, extract the `PartialTurnContext` from the agent's context capture and store it in `self.pending_context`.

The cleanest approach: pass a `tokio::sync::mpsc::Sender` channel to the AgentLoop that can send back `PartialTurnContext`. Add to `TuiAgentLoop`:

```rust
pub context_tx: Option<tokio::sync::mpsc::UnboundedSender<PartialTurnContext>>,
```

Then in `process_input_inner()`, after building `assembled_instructions` and `reminder`:

```rust
if let Some(tx) = &self.ctx.context_tx {
    let context = PartialTurnContext {
        layers: assembled_instructions.layers.clone(),
        memories: self.extract_memory_meta(&ctx),
        reminder: reminder.clone(),
    };
    let _ = tx.send(context);
}
```

### Step 2.4 — Finalize `TurnContext` and push to ring buffer

After `agent.process_input()` returns in `spawn_agent_turn`, take `self.pending_context` and finalize:

```rust
if let Some(partial) = self.pending_context.take() {
    // Build full_messages: assembled layers + conversation history
    let sys_messages = partial.layers.iter()
        .map(|l| ChatMessage::system(l.content.clone()))
        .collect::<Vec<_>>();
    let history = self.conversation_history.blocking_lock().clone();
    let full_messages: Vec<ChatMessage> = sys_messages.into_iter().chain(history).collect();
    
    let context = TurnContext {
        turn_index: self.turn_contexts.len(),
        layers: partial.layers,
        memories: partial.memories,
        reminder: partial.reminder,
        full_messages,
    };
    
    if self.turn_contexts.len() >= TURN_CONTEXT_CAPACITY {
        self.turn_contexts.remove(0);
    }
    self.turn_contexts.push(context);
}
```

---

## Phase 3: Inspector Component (9 steps)

### Step 3.1 — Create `src/tui/components/inspector.rs` skeleton

```rust
use ratatui::{
    layout::Rect,
    Frame,
};
use crate::tui::traits::Component;
use crate::tui::app::TurnContext;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InspectorTab {
    Layers,
    Memories,
    Messages,
    Hooks,
    Tokens,
}

impl InspectorTab {
    fn label(&self) -> &'static str {
        match self {
            InspectorTab::Layers => "L",
            InspectorTab::Memories => "M",
            InspectorTab::Messages => "Msg",
            InspectorTab::Hooks => "H",
            InspectorTab::Tokens => "T",
        }
    }
    
    fn all() -> &'static [InspectorTab] {
        &[
            InspectorTab::Layers,
            InspectorTab::Memories,
            InspectorTab::Messages,
            InspectorTab::Hooks,
            InspectorTab::Tokens,
        ]
    }
}

pub struct InspectorComponent {
    pub visible: bool,
    pub active_tab: InspectorTab,
    pub selected_turn: usize,
    pub expanded_layers: std::collections::BTreeSet<usize>,
    pub scroll_offset: u16,
}

impl Default for InspectorComponent {
    fn default() -> Self {
        Self {
            visible: false,
            active_tab: InspectorTab::Layers,
            selected_turn: 0,
            expanded_layers: std::collections::BTreeSet::new(),
            scroll_offset: 0,
        }
    }
}
```

### Step 3.2 — Implement `Component` trait (render)

```rust
impl Component for InspectorComponent {
    fn render(&self, f: &mut Frame, area: Rect) {
        if !self.visible || area.width < 40 || area.height < 5 {
            return;
        }
        self.render_tabs_header(f, area);
        let body = self.body_area(area);
        match self.active_tab {
            InspectorTab::Layers => self.render_layers_tab(f, body),
            InspectorTab::Memories => self.render_memories_tab(f, body),
            InspectorTab::Messages => self.render_messages_tab(f, body),
            InspectorTab::Hooks => self.render_hooks_tab(f, body),
            InspectorTab::Tokens => self.render_tokens_tab(f, body),
        }
        self.render_footer(f, area);
    }
}
```

### Step 3.3 — Tab header rendering

```rust
fn render_tabs_header(&self, f: &mut Frame, area: Rect) {
    let tabs: Vec<Span> = InspectorTab::all().iter().enumerate().flat_map(|(i, tab)| {
        let style = if tab == &self.active_tab {
            Style::default().fg(TAB_ACTIVE).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(TAB_DIM)
        };
        if i > 0 {
            vec![Span::raw(" | "), Span::styled(tab.label(), style)]
        } else {
            vec![Span::styled(tab.label(), style)]
        }
    }).collect();
    
    let header = Block::default()
        .title("Inspector [F2]")
        .borders(Borders::TOP);
    f.render_widget(header, area);
    f.render_widget(
        Line::from(tabs),
        Rect { y: area.y, x: area.x + 2, width: area.width - 4, height: 1 },
    );
}
```

### Step 3.4 — Layers tab

Iterate `TurnContext.layers`, show each with label + source badge. Enter toggles expand. Content rendered in a `Paragraph` with wrapping.

### Step 3.5 — Memories tab

Iterate `TurnContext.memories`, show scope badge + importance bar (ratatui `Gauge`) + content preview.

### Step 3.6 — Messages tab

Iterate `TurnContext.full_messages`, render role-colored headers with monospace content. Viewport clipping: only render lines in `[scroll_offset, scroll_offset + body.height]`.

### Step 3.7 — Hooks tab

Show `TurnContext.reminder` content:
- "## to_model" header → full reminder text  
- "## to_transcript" header → transcript text

### Step 3.8 — Tokens tab

Table: layer | chars | est_tokens. Formula: `ceil(chars / 4.0)`. Total row at bottom.

### Step 3.9 — History navigation + scroll

- `↑`/`↓`: when Inspector focused, change `selected_turn`
- `J`/`K` or `PgUp`/`PgDn`: adjust `scroll_offset`
- Footer: `"Turn N/M"` bar

---

## Phase 4: Layout Integration (5 steps)

### Step 4.1 — Split pane in `render.rs`

```rust
// Determine Inspector area
let inspector_visible = app.inspector.visible
    && app.focused_subagent.is_none()
    && frame.size().width >= 80;

let (chat_area, inspector_area) = if inspector_visible {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(65),
            Constraint::Percentage(35),
        ])
        .split(main_area);
    (chunks[0], Some(chunks[1]))
} else {
    (main_area, None)
};
```

### Step 4.2 — Register in component system

In `src/tui/components/mod.rs`:
```rust
pub mod inspector;
pub use inspector::InspectorComponent;
```

In `src/tui/app/mod.rs`:
```rust
inspector: InspectorComponent::default(),
```

### Step 4.3 — F2 event handler

In `src/tui/app/event.rs`, key event handler:
```rust
KeyCode::F(2) => {
    self.inspector.visible = !self.inspector.visible;
    if self.inspector.visible {
        // Auto-select latest turn
        self.inspector.selected_turn = self.turn_contexts.len().saturating_sub(1);
        self.inspector.scroll_offset = 0;
    }
}
```

### Step 4.4 — Auto-fold logic

When `focused_subagent.is_some()`, treat inspector as hidden (already in step 4.1).
When terminal width < 80, treat inspector as hidden.
When subagent focus closes (`focused_subagent = None`), inspector restores if it was open before.

### Step 4.5 — Inspector event dispatch

In `event.rs`, add event handling block for Inspector keyboard events when `inspector.visible`:

```rust
if self.inspector.visible && self.focused_subagent.is_none() {
    match key {
        KeyCode::Tab => self.inspector.next_tab(),
        KeyCode::Up => self.inspector.prev_turn_or_scroll_up(),
        KeyCode::Down => self.inspector.next_turn_or_scroll_down(),
        // ... etc
    }
}
```

---

## Phase 5: Theme & Registration (2 steps)

### Step 5.1 — Theme colors

In `src/tui/theme.rs`:
```rust
pub const TAB_ACTIVE: Color = Color::Rgb(100, 200, 255);
pub const TAB_DIM: Color = Color::Rgb(80, 80, 80);
pub const SOURCE_BADGE_BUILTIN: Color = Color::Rgb(60, 120, 200);
pub const SOURCE_BADGE_FILE: Color = Color::Rgb(200, 160, 60);
pub const SOURCE_BADGE_MEMORY: Color = Color::Rgb(100, 200, 120);
pub const IMPORTANCE_HIGH: Color = Color::Rgb(200, 80, 80);
pub const IMPORTANCE_LOW: Color = Color::Rgb(80, 80, 80);
```

### Step 5.2 — Verify compilation

```bash
cargo check
```

---

## Phase 6: Tests & Validation (6 steps)

### Step 6.1 — `inspector.rs` unit tests

Test:
- `test_tab_cycling` — Tab/Shift+Tab cycles correctly
- `test_layer_expand_collapse` — Enter toggles, L toggles all
- `test_turn_history_bounds` — 0-turn edge case, max 50
- `test_no_auto_jump_on_new_turn` — selected_turn unchanged
- `test_auto_fold_under_80_cols` — hidden when area < 40

### Step 6.2 — `prompts/mod.rs` tests

Test:
- `test_layers_each_have_source` — Every layer in assembled output has correct LayerSource
- `test_layer_count_matches_system_messages` — layers.len() == system_messages.len()

### Step 6.3 — TurnContext capture test

Test (integration or manual):
- After one user input, `App.turn_contexts` has 1 entry
- TurnContext.layers not empty
- TurnContext.full_messages contains system + user messages

### Step 6.4 — Confirm task_panel removal

```bash
grep -r "task_panel" src/tui/ --include="*.rs" 
# Should return nothing
```

### Step 6.5 — Format + clippy

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

### Step 6.6 — Full test suite

```bash
cargo test --all
```

---

## Rollback Plan

Each phase is self-contained and can be rolled back independently:
- Phase 0 (task_panel removal): revert deleted file + git checkout the 4 edited files
- Phase 1-2 (data model): if data flow breaks but no UI yet, revert `mod.rs` and `turn.rs` changes
- Phase 3 (Inspector): remove `inspector.rs` + revert `mod.rs` registration
- Phase 4 (layout): revert `render.rs` + `event.rs` changes
- Phase 5-6: trivial revert

## Commit Plan

```
feat(tui): add request context Inspector panel
```

Body with bullet list of changes per phase.
