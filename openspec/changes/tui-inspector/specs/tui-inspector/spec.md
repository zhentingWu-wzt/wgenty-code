# Delta Spec: tui-inspector

## ADDED: TUI Inspector Panel

### Requirement: Inspector Toggle

The TUI SHALL provide an Inspector side panel toggled by the `F2` key.
When open, the main chat panel SHALL resize to ~65% width and the Inspector SHALL occupy the remaining ~35% on the right.
When closed, the chat panel SHALL restore to full width.
On terminal widths below 80 columns, the Inspector SHALL auto-hide.

When `subagent_focus_view` is active, the Inspector SHALL auto-collapse with state preserved; when focus_view closes, the Inspector SHALL restore to its previous visible state.
Toggling with F2 while hidden SHALL restore the last active tab and turn index.

### Requirement: Inspector Tabs

The Inspector SHALL render 5 tabs accessible via `Tab`/`Shift+Tab`:
- **L** (Layers) — system prompt layered view
- **M** (Memories) — recalled memories list
- **Msg** (Messages) — full API messages array
- **H** (Hooks) — system reminder hook injections
- **T** (Tokens) — character/token statistics

The active tab SHALL be visually highlighted.

### Requirement: Layers Tab

The Layers tab SHALL display each system prompt layer as a collapsible row showing:
- Layer label (e.g., "Layer 1: base_instructions")
- Source badge (Builtin / ConfigFile / ProjectFile / MemoryRecall / SkillInventory / HookInjection)
- Expandable content body

Pressing `Enter` on a layer SHALL toggle its expand/collapse state.
Pressing `L` SHALL toggle expand/collapse all layers.

### Requirement: Memories Tab

The Memories tab SHALL list all recalled memories for the current turn with:
- Scope badge (project or global)
- Importance indicator (progress bar-style)
- Content preview (truncated to fit)

### Requirement: Messages Tab

The Messages tab SHALL render the complete API messages array in monospace font with role-tagged entries (system, user, assistant, tool).
Only visible lines SHALL be rendered (viewport clipping) to avoid performance degradation with large message arrays.

### Requirement: Hooks Tab

The Hooks tab SHALL display the `<system-reminder>` content from `ReminderOutput`, partitioned into:
- **to_model**: full reminder text injected into the user message
- **to_transcript**: UI-visible subset

### Requirement: Tokens Tab

The Tokens tab SHALL display a table with columns: layer/role, character count, estimated token count.
Token estimation SHALL use `ceil(chars / 4.0)`.
When the API response contains `usage.prompt_tokens`, the measured value SHALL be displayed alongside the estimate with visual distinction.

### Requirement: Turn History Navigation

The Inspector SHALL maintain snapshots of the last 50 turns (`TurnContext` ring buffer).
Pressing `↑`/`↓` in the Inspector SHALL navigate between historical turns.
The footer SHALL display "Turn N/M" indicating current and total available turns.
When a new turn completes while the user is browsing history, the Inspector SHALL NOT auto-jump to the latest turn; the user SHALL manually navigate back via `↓`.

### Requirement: Scroll Support

Each tab SHALL support scrolling via `J`/`K` or `↑`/`↓` (when not navigating turns in list mode) and `PgUp`/`PgDn`.
Only visible lines SHALL be rendered (viewport clipping).

## MODIFIED: Prompt Assembly Layer Metadata

### Requirement: LayerMeta in AssembledInstructions

`AssembledInstructions` SHALL carry a `layers: Vec<LayerMeta>` field alongside the existing `system_messages: Vec<ChatMessage>`.
Each `LayerMeta` SHALL contain:
- `label: String` — human-readable layer name
- `source: LayerSource` — origin tracking
- `content: String` — full layer text
- `char_count: usize` — character count for token estimation

`LayerSource` variants SHALL include at minimum:
- `Builtin` for hardcoded templates
- `ConfigSettings` for settings.json-derived instructions
- `ConfigFile(PathBuf)` for user-level config files (WGENTY.md, rules/*.md)
- `ProjectFile(PathBuf)` for project-level config files (AGENTS.md, WGENTY.md)
- `MemoryRecall { scope: String }` for recalled memories
- `SkillInventory` for the skills overview
- `Unknown` fallback

The `assemble_instructions()` function SHALL populate `layers` for every layer it builds.
Existing `system_messages` SHALL remain populated identically (backward compatible).

### Requirement: TurnContext Snapshot Capture

During `process_input_inner()`, after `assemble_instructions()` and `build_user_turn_reminder()`, a `PartialTurnContext` SHALL be stashed in `App` with the assembled layers, recalled memories, and system reminder.
After `run_agent_loop()` completes, the `PartialTurnContext` SHALL be finalized into a `TurnContext` containing:
- `assembled_layers: Vec<LayerMeta>` from the assembled instructions
- `recalled_memories: Vec<MemoryMeta>` with scope, importance, and score metadata
- `system_reminder: Option<ReminderOutput>` from `build_user_turn_reminder`
- `full_messages: Vec<ChatMessage>` — the full messages array sent to the LLM

The finalized snapshot SHALL be pushed to a ring buffer of max 50 entries in `App.turn_contexts`.
A previous round's `PartialTurnContext` SHALL be `take()`-cleared before a new round begins.

## MODIFIED: TUI Layout

### Requirement: Inspector Split Pane

`compute_layout()` SHALL return a right-side `Rect` for the Inspector when `inspector.visible == true`.
The main chat area SHALL be constrained to 65% of terminal width, with the Inspector occupying the remaining 35%.
When `inspector.visible == false`, layout SHALL be identical to current behavior (no right panel).
When `subagent_focus_view` is active, `inspector.visible` SHALL be treated as `false` with state preserved for restoration.
