# Subagent Execution Visibility — Design Spec

**Date**: 2026-06-13
**Status**: Design approved
**Scope**: All subagent execution paths (delegate/RLM, task, Agent tool, Workflow)

## Problem

When users trigger subagent-heavy operations (e.g., "分析这个项目的RLM架构"), the delegate/task tool can run for minutes with zero visible progress. The status bar only shows "Executing tool: task" — users cannot see:

1. How many subagents were spawned
2. Their hierarchy/dependency structure
3. Real-time status per subagent (pending/running/completed/failed)
4. What each subagent is currently doing (round, tool name)
5. When/if the operation is making meaningful progress

This creates a "black box" experience that erodes user trust and makes debugging stuck agents impossible.

## Design Goals

- **Coverage**: All subagent paths — `delegate` (RLM pipeline), `task` tool, parallel task execution, Agent tool, Workflow
- **Visibility depth**: Tree view with per-node status (B-level), NOT round-by-round "livestream" (D-level)
- **Display**: Dual mode — inline collapsible card in chat + dedicated toggleable panel
- **History**: Completed subagent trees can be reviewed in chat history as summary snapshots

## Architecture

### Data Flow

```
subagent_loop (callback)  ──→  tool dispatch layer  ──→  TUI
        ↑                          ↑                       ↑
   SubagentProgress          AppEvent::              SubagentTree
   (standalone type)         SubagentUpdate          (Arc<RwLock<>>)
```

The key design principle: **subagent loop does NOT know about AppEvent or TUI**. It only calls an optional callback with a standalone `SubagentProgress` struct. The tool dispatch layer (in `core.rs`) converts this to an `AppEvent` and sends it through the existing `event_tx` channel.

### Core Types

#### `SubagentProgress` — standalone, in `src/teams/` or `src/agent/`

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentProgress {
    pub node_id: String,              // uuid, unique per subagent instance
    pub parent_id: Option<String>,    // None = root node (the delegate/task itself)
    pub label: String,                // e.g. "Sub 2: 分析 pipeline.rs"
    pub status: SubagentStatus,
    pub round: Option<usize>,         // current round number
    pub max_rounds: Option<usize>,    // max rounds configured
    pub current_tool: Option<String>, // e.g. "grep", "file_read"
    pub started_at: i64,              // Unix timestamp
    pub elapsed_ms: u64,              // wall-clock elapsed
    pub metadata: Option<SubagentMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SubagentStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentMetadata {
    pub token_count: Option<usize>,
    pub error: Option<String>,
    pub depends_on: Vec<String>,      // node_ids this node waits for
}
```

#### Callback type

```rust
pub type ProgressCallback = Arc<dyn Fn(SubagentProgress) + Send + Sync>;
```

#### `SubagentTree` — TUI-side state in `src/tui/`

```rust
pub struct SubagentTree {
    pub root_id: Option<String>,
    pub nodes: HashMap<String, SubagentNode>,
}

pub struct SubagentNode {
    pub progress: SubagentProgress,
    pub children: Vec<String>,        // child node_ids
}
```

### run_subagent_loop Signature

```rust
// Before
pub async fn run_subagent_loop(
    api_client: &ApiClient,
    tool_registry: &ToolRegistry,
    system_prompt: &str,
    user_prompt: &str,
    allowed_tools: &[String],
    max_rounds: usize,
    timeout_secs: u64,
) -> Result<String, String>

// After — single optional parameter added
pub async fn run_subagent_loop(
    api_client: &ApiClient,
    tool_registry: &ToolRegistry,
    system_prompt: &str,
    user_prompt: &str,
    allowed_tools: &[String],
    max_rounds: usize,
    timeout_secs: u64,
    on_progress: Option<ProgressCallback>,  // NEW
) -> Result<String, String>
```

### Callback Invocation Points in subagent_loop

1. **On entry**: Emit `SubagentProgress` with `status: Running`, `round: 0`
2. **Each round start**: Emit with updated `round`, `elapsed_ms`
3. **Before tool execution**: Emit with `current_tool` set to the tool name
4. **On completion**: Emit with `status: Completed`, final `round` count
5. **On error/timeout**: Emit with `status: Failed`, `metadata.error` set

### AppEvent Addition

Only ONE new variant:

```rust
pub enum AppEvent {
    // ... all existing variants unchanged ...
    SubagentUpdate(SubagentProgress),
}
```

### Tool Dispatch Layer Changes (`src/tui/agent/core.rs`)

In `execute_tool_static` (or equivalent), when dispatching `task` or `delegate`:

1. Create a `SubagentTree` (or get existing one for this turn)
2. Build a `ProgressCallback` closure that:
   - Calls `tree.upsert(progress)` to update the tree state
   - Sends `AppEvent::SubagentUpdate(progress)` through `event_tx`
3. Pass the callback to `run_subagent_loop` (or RLM pipeline)

## TUI Component Design

### 1. Inline Card (in chat flow)

**Location**: Below the tool call message (task/delegate) in the conversation pane.

**States**:
- **Executing** (any node Running/Pending): Card shows full tree, auto-expanded. Updates live on each `SubagentUpdate` event.
- **Completed** (all nodes Completed/Failed): Card auto-collapses to single-line summary:
  ```
  ✅ delegate · 5/5 done · 2m 34s
  ```
- **User interaction**: Press Enter on a selected tool call message to toggle expand/collapse of its subagent card.

**Tree rendering** (inside card):
```
🌳 Subagent Tree                                    3/5 done · collapse ▾
┌─ 🔄 delegate                                      executing · 12s
├─ ✅ Planner                                        3 tasks · 2s
├─ ✅ Sub 1: 分析 RLM mod.rs                         8 rounds · 45s
├─ 🔄 Sub 2: 分析 pipeline.rs                        round 3/20 · grep ...
├─ ⏳ Sub 3: 分析 subagent_loop                      waiting: Sub 1 & 2
└─ ⏳ Sub 4: 分析 mailbox.rs                         waiting: Sub 3
```

**Status indicators**:
- ⏳ Pending (gray)
- 🔄 Running (yellow, with spinner animation)
- ✅ Completed (green)
- ❌ Failed (red)
- 🚫 Cancelled (dim red)

### 2. Dedicated Panel (toggleable)

**Trigger**: Hotkey `Ctrl+Shift+T` (alongside existing `Ctrl+T` for task panel). Adds `ToggleSubagentPanel` to `AppEvent`.

**Layout** (full-height side panel or overlay):
```
┌─ 🌳 Subagent Monitor ───── 5 agents · 3 active ────── Ctrl+Shift+T toggle ─┐
│                                                                             │
│  ✅ 2 done    🔄 2 running    ⏳ 2 pending    ⏱ 18s elapsed                 │
│                                                                             │
│  🔄 delegate                     RLM pipeline · 18s               expand ▾  │
│  ├─ ✅ Planner                   3 sub-tasks · 2s · 120 tokens              │
│  ├─ 🔄 Executor                  level 0/2 · 2 parallel                     │
│  │  ├─ ✅ Sub 1: 分析 RLM mod.rs  8 rounds · 45s · 2.3k tokens              │
│  │  ├─ 🔄 Sub 2: 分析 pipeline.rs round 3/20                                │
│  │  │  └─ 🛠 executing: grep "run_rlm_pipeline" src/                        │
│  │  ├─ ⏳ Sub 3: 分析 subagent_loop  waiting: Sub 1 & 2                     │
│  │  └─ ⏳ Sub 4: 分析 mailbox.rs     waiting: Sub 3                         │
│  └─ ⏳ Aggregator                  pending results                          │
│                                                                             │
│  ℹ️ Sub 3 & 4 depend on Sub 1 & 2 — waiting for dependency level            │
└─────────────────────────────────────────────────────────────────────────────┘
```

**Features**:
- Scrollable tree view
- Running nodes highlighted with accent color background
- Bottom info line shows details for hovered/selected node
- Panel stays live when open; state persists when panel is closed
- Summary bar at top shows aggregate counts + total elapsed

### 3. History Review

- Each turn's `SubagentTree` is stored keyed by `turn_id` in session state
- When scrolling to a historical tool call message:
  - The inline card renders a **static snapshot** (not live-updating)
  - Shows the final summary: each node's label + status + rounds + elapsed
- Dedicated panel shows the tree for the **current/latest** turn only
- Historical trees are serialized to the session JSON file for persistence across restarts

## Implementation Plan

### Phase 1: Core Types & Callback (no UI changes)

1. Define `SubagentProgress`, `SubagentStatus`, `SubagentMetadata`, `ProgressCallback` in `src/agent/events.rs` or new `src/agent/progress.rs`
2. Add `on_progress: Option<ProgressCallback>` parameter to `run_subagent_loop`
3. Add callback invocation at: loop entry, each round start, pre-tool-execute, completion, error
4. Update all call sites of `run_subagent_loop` to pass `None` for now
5. Update RLM pipeline to accept and forward `on_progress` to subagent loops

### Phase 2: AppEvent & Tree State

6. Add `SubagentUpdate(SubagentProgress)` to `AppEvent`
7. Implement `SubagentTree` with `upsert` logic (create/update nodes, maintain parent-child links)
8. Add `SubagentTree` to app state (or turn state)
9. Handle `SubagentUpdate` event in `handle_event`

### Phase 3: Inline Card Rendering

10. Implement inline card rendering in chat message renderer
11. Implement auto-collapse on completion
12. Implement expand/collapse toggle on Enter key

### Phase 4: Dedicated Panel

13. Add `ToggleSubagentPanel` event and keybinding (`Ctrl+Shift+T`)
14. Implement panel widget with tree rendering, scroll, highlight
15. Wire panel to consume `SubagentTree` state

### Phase 5: History Persistence

16. Store `SubagentTree` snapshot in turn/session data
17. Render static summary in historical tool call messages
18. Serialize/deserialize for session persistence

### Phase 6: Wiring (connect callback to events)

19. In tool dispatch (`core.rs`), create callback that bridges `SubagentProgress` → `AppEvent::SubagentUpdate`
20. Pass callback to `run_subagent_loop` / RLM pipeline for `task` and `delegate` tools
21. Also wire for Agent tool invocations (wgentCodeGuideAgent, exploreAgent, etc.)

## Non-Goals (for this spec)

- Round-by-round "livestream" of subagent LLM responses (D-level)
- Web/daemon API streaming of subagent progress (Rust TUI only for now)
- Subagent tree for Workflow orchestration (Workflow already has its own progress display)
- Modifying existing tracing/logging infrastructure

## File Impact Summary

| File | Change |
|------|--------|
| `src/agent/progress.rs` | NEW — `SubagentProgress` types and callback |
| `src/agent/mod.rs` | Add `progress` module |
| `src/teams/subagent_loop.rs` | Add `on_progress` param + callback invocations |
| `src/tools/meta/rlm/pipeline.rs` | Accept and forward `on_progress` |
| `src/tui/app/types.rs` | Add `SubagentUpdate` to `AppEvent` |
| `src/tui/app/event.rs` | Handle `SubagentUpdate` event |
| `src/tui/agent/core.rs` | Wire callback in tool dispatch |
| `src/tui/agent/tool_dispatch.rs` | Pass callback to subagent loops |
| `src/tui/components/subagent_tree.rs` | NEW — `SubagentTree` state + rendering |
| `src/tui/components/subagent_panel.rs` | NEW — dedicated panel widget |
| `src/tui/chat/render.rs` or equivalent | Inline card rendering in chat messages |
| `src/state/` | Optional: turn-level tree storage |

## Risks & Mitigations

| Risk | Mitigation |
|------|------------|
| High-frequency events (every round) flood event channel | Batch updates with min 200ms interval per node; only emit on state change |
| Subagent loop hangs without emitting completion | Timeout path already emits Failed; add heartbeat check in parent |
| Deeply nested subagents (subagent spawns subagent) require callback threading | ProgressCallback is `Arc<dyn Fn + Send + Sync>`, easily cloneable and passable |
| TUI performance with frequent redraws | Tree rendering is cheap (O(n) nodes); ratatui handles diff-based rendering |
