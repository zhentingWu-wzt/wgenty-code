---
comet_change: subagent-visualization
role: technical-design
canonical_spec: openspec
---

# Subagent Visualization — Technical Design

## Architecture

```
┌─ subagent_trace.rs ─────────────────────────────────────────────┐
│                                                                  │
│  render_html_report(session_id)                                  │
│    ├─ build_trace_tree(session_id) → Vec<TraceNode>             │
│    ├─ nodes_to_json(&roots) → serde_json::Value                 │
│    ├─ SubagentHealthAnalyzer::compute_health() → SubagentHealth │
│    └─ build_html_report(&tree_json, &health_json, session_id)   │
│         ├─ <style> Catppuccin Mocha CSS (~200 lines)            │
│         ├─ <script> DATA = {tree: ..., health: ...}             │
│         ├─ <script> tab switching + tree expand/collapse JS     │
│         └─ return String (self-contained HTML)                   │
│                                                                  │
└──────────────────────────────────────────────────────────────────┘

┌─ task_panel.rs ─────────────────────────────────────────────────┐
│                                                                  │
│  render(f, area, state)                                          │
│    for item in state.items:                                      │
│      match item.subagent:                                        │
│        Some(meta) → 🤖 type · Nr · N.Ns · N.Nk tokens           │
│        None → existing ✓/●/○ + label rendering                  │
│                                                                  │
└──────────────────────────────────────────────────────────────────┘

┌─ client.rs ─────────────────────────────────────────────────────┐
│                                                                  │
│  TodoItem {                                                      │
│    content: String,                                              │
│    status: String,           // "pending"|"in_progress"|"done"  │
│    active_form: String,                                          │
│    subagent: Option<SubagentTodoMeta>,  // NEW                   │
│  }                                                               │
│                                                                  │
│  SubagentTodoMeta {                                              │
│    subagent_type: String,    // "explore"|"plan"|"general"      │
│    token_usage: u64,                                            │
│    rounds: u32,                                                 │
│    duration_ms: u64,                                            │
│  }                                                               │
│                                                                  │
└──────────────────────────────────────────────────────────────────┘
```

## Key Decisions

### HTML Report
- **Self-contained single file**: CSS/JS inlined, no CDN, no network dependency
- **Catppuccin Mocha theme**: matches TUI color scheme, dark background
- **3-tab layout**: Call Tree (default) | Health Dashboard | Error Timeline
- **Data embedding**: `<script>const DATA = {tree: [...], health: {...}};</script>`
- **JS interactivity**: tree expand/collapse (default depth 3), tab switching, ~150 lines vanilla JS

### String Truncation Safety
- Use `is_char_boundary()` loop to adjust slice indices before `&s[..N]`
- Pattern: `fn floor_char_boundary(s, idx) → idx` — decrement until valid boundary
- Applied at: `subagent_trace.rs:131` (tool params), `subagent_trace.rs:254` (error msg)

### Task Panel Enhancement
- `SubagentTodoMeta` as optional field on `TodoItem`, `#[serde(default)]` for backward compat
- Subagent tasks rendered with 🤖 icon + `{type} · {N}r · {N.N}s · {N.N}k tokens`
- Regular tasks unchanged
- Daemon-side: fill `subagent` field when task source is a subagent

## Data Flow

```
Subagent spawn → TaskCreate tool → TodoItem { subagent: Some(meta) }
  → Daemon stores → TUI fetches via GET /todos
  → TaskPanel renders with subagent styling
```

## Testing
- `nodes_to_json`: unit test with empty tree, single node, nested tree, multi-byte UTF-8
- `build_html_report`: verify output contains `<script>`, tab containers, health metrics
- Task Panel: verify subagent task line vs regular task line rendering
- Byte-index safety: test with strings containing `─` (3-byte) at truncation boundary
