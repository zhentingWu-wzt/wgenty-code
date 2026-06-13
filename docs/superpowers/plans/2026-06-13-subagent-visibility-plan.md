# Subagent Execution Visibility — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give users real-time visibility into subagent execution — tree hierarchy, per-node status, round progress — via inline chat cards and a dedicated toggleable panel.

**Architecture:** Subagent loop emits `SubagentProgress` events through an optional callback. On the daemon side, the callback writes to a shared `SubagentProgressStore`. A new daemon polling endpoint exposes this store. On the TUI side, the agent loop polls progress while a task/delegate tool is running, converts results into `AppEvent::SubagentUpdate`, and renders them as an inline collapsible tree card + a dedicated monitor panel.

**Tech Stack:** Rust, tokio, ratatui, reqwest, serde, uuid, chrono

---

## File Impact Summary

| File | Change |
|------|--------|
| `src/agent/progress.rs` | **NEW** — `SubagentProgress`, `SubagentStatus`, `SubagentMetadata`, `ProgressCallback` |
| `src/agent/mod.rs` | **MODIFY** — export `progress` module |
| `src/teams/subagent_loop.rs` | **MODIFY** — add `on_progress` param, invoke at key points, update call sites |
| `src/tools/meta/task.rs` | **MODIFY** — accept `progress_store`, create callback, pass to subagent_loop/rlm |
| `src/tools/meta/run_script.rs` | **MODIFY** — pass `None` for `on_progress` |
| `src/tools/meta/rlm/pipeline.rs` | **MODIFY** — accept and forward `on_progress` to child subagent loops |
| `src/tools/meta/rlm/mod.rs` | **MODIFY** — accept `progress_store`, pass to pipeline |
| `src/daemon/state.rs` | **MODIFY** — add `subagent_progress` store, pass to tool constructors |
| `src/daemon/handlers.rs` | **MODIFY** — add `GET /api/v1/subagent/progress` handler |
| `src/daemon/routes.rs` | **MODIFY** — register new route |
| `src/tui/app/types.rs` | **MODIFY** — add `SubagentUpdate(SubagentProgress)` and `ToggleSubagentPanel` |
| `src/tui/app/event.rs` | **MODIFY** — handle new events |
| `src/tui/app/mod.rs` | **MODIFY** — add `subagent_tree`, `subagent_panel_visible` fields |
| `src/tui/components/subagent_tree.rs` | **NEW** — `SubagentTree` with upsert logic + tests |
| `src/tui/components/subagent_panel.rs` | **NEW** — dedicated panel widget |
| `src/tui/components/chat.rs` | **MODIFY** — inline card rendering below tool messages |
| `src/tui/components/mod.rs` | **MODIFY** — export new modules |
| `src/tui/client.rs` | **MODIFY** — add `poll_subagent_progress()` |
| `src/tui/agent/tool_dispatch.rs` | **MODIFY** — poll progress during task/delegate execution |
| `src/tui/agent/core.rs` | **MODIFY** — pass `event_tx` through to tool dispatch |

---

### Task 1: Define Core Progress Types

**Files:**
- Create: `src/agent/progress.rs`
- Modify: `src/agent/mod.rs`

- [ ] **Step 1: Create `src/agent/progress.rs`**

```rust
//! Subagent progress types for real-time execution visibility.
//!
//! These types are standalone — they do NOT depend on AppEvent or TUI types.
//! The subagent loop emits `SubagentProgress` events through an optional
//! `ProgressCallback`. The daemon stores them in a shared store; the TUI polls
//! the store and converts updates into `AppEvent::SubagentUpdate` for rendering.

use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// A progress update emitted by a subagent at key lifecycle points.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentProgress {
    pub node_id: String,
    pub parent_id: Option<String>,
    pub label: String,
    pub status: SubagentStatus,
    pub round: Option<usize>,
    pub max_rounds: Option<usize>,
    pub current_tool: Option<String>,
    pub started_at: i64,
    pub elapsed_ms: u64,
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
    pub depends_on: Vec<String>,
}

pub type ProgressCallback = Arc<dyn Fn(SubagentProgress) + Send + Sync>;
```

- [ ] **Step 2: Update `src/agent/mod.rs`**

Read current `src/agent/mod.rs`. Add `pub mod progress;` alongside the existing module declarations. Also add the re-export:

```rust
pub mod core;
pub mod events;
pub mod progress;

pub use core::StreamProcessor;
pub use events::{StreamEvent, StreamResult};
```

- [ ] **Step 3: Build check**

```bash
cargo check -p wgenty_code 2>&1 | head -20
```

Expected: compiles (types unused yet).

- [ ] **Step 4: Commit**

```bash
git add src/agent/progress.rs src/agent/mod.rs
git commit -m "feat(agent): define SubagentProgress types and ProgressCallback

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 2: Add on_progress to run_subagent_loop + Update Call Sites

**Files:**
- Modify: `src/teams/subagent_loop.rs`
- Modify: `src/tools/meta/task.rs`
- Modify: `src/tools/meta/run_script.rs`
- Modify: `src/tools/meta/rlm/pipeline.rs`
- Modify: `src/tools/meta/rlm/mod.rs`

- [ ] **Step 1: Add import and param to run_subagent_loop**

In `src/teams/subagent_loop.rs`, add import:
```rust
use crate::agent::progress::{ProgressCallback, SubagentProgress, SubagentStatus, SubagentMetadata};
```

Change signature (line ~40) to add `on_progress: Option<ProgressCallback>` as the last parameter.

- [ ] **Step 2: Add callback invocations in subagent_loop**

Six invocation points, each guarded by `if let Some(ref cb) = on_progress`:

1. **After loop entry** (after `let start = Instant::now();`): emit `SubagentProgress` with `status: Running, round: Some(0)`
2. **Each round start** (after the round tracing info): emit with `status: Running, round: Some(round+1), elapsed_ms` 
3. **Before each tool execution** (before `tracing::debug!(tool = %tool_name, ...)`): emit with `current_tool: Some(tool_name.clone())`
4. **On completion** (before `return Ok(choice.message.content...)`): emit with `status: Completed`
5. **On stuck abort** (before `return Err(msg)`): emit with `status: Failed, metadata.error: Some(msg.clone())`
6. **On max rounds exceeded** (before the Err return): emit with `status: Failed`
7. **On timeout** (before the Err return): emit with `status: Failed, metadata.error: Some("timed out...")`

Each emission includes `node_id: trace_id.to_string()`, `parent_id: None`, `label: String::new()` (these get overridden by the wrapper callback in Task 4).

- [ ] **Step 3: Update task.rs call sites**

In `src/tools/meta/task.rs`, add `None` as the final argument to both `run_subagent_loop(...)` calls (background mode and synchronous mode).

- [ ] **Step 4: Update run_script.rs call site**

In `src/tools/meta/run_script.rs`, add `None` as the final argument to `run_subagent_loop(...)`.

- [ ] **Step 5: Update RLM pipeline**

In `src/tools/meta/rlm/pipeline.rs`:
- Add import: `use crate::agent::progress::ProgressCallback;`
- Add `on_progress: Option<ProgressCallback>` parameter to `run_rlm_pipeline`
- Forward `on_progress.clone()` to each `run_subagent_loop(...)` call inside the executor

- [ ] **Step 6: Update RLM delegate tool**

In `src/tools/meta/rlm/mod.rs`, add `None` as the final argument to `run_rlm_pipeline(...)`.

- [ ] **Step 7: Build check**

```bash
cargo check -p wgenty_code 2>&1 | head -30
```

Expected: compiles. All call sites pass `None`.

- [ ] **Step 8: Commit**

```bash
git add src/teams/subagent_loop.rs src/tools/meta/task.rs src/tools/meta/run_script.rs src/tools/meta/rlm/pipeline.rs src/tools/meta/rlm/mod.rs
git commit -m "feat(subagent): add on_progress callback to run_subagent_loop and run_rlm_pipeline

Emits SubagentProgress at entry, each round, pre-tool-execute, completion,
and error/timeout paths. All call sites pass None for now.

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 3: Add SubagentProgressStore to Daemon

**Files:**
- Modify: `src/daemon/state.rs`
- Modify: `src/daemon/handlers.rs`
- Modify: `src/daemon/routes.rs`

- [ ] **Step 1: Add store field to DaemonState**

Read `src/daemon/state.rs`. Add to the `DaemonState` struct:
```rust
pub subagent_progress: Arc<RwLock<HashMap<String, crate::agent::progress::SubagentProgress>>>,
```

- [ ] **Step 2: Initialize store and pass to tool constructors**

In `DaemonState::new()` (line ~47), BEFORE the `Arc::new_cyclic` block:
```rust
let progress_store: Arc<RwLock<HashMap<String, crate::agent::progress::SubagentProgress>>> =
    Arc::new(RwLock::new(HashMap::new()));
```

Update `TaskTool::new(...)` call to pass `progress_store.clone()` as the new 4th argument.

Update `RlmDelegateTool::new(...)` call to pass `progress_store.clone()` as the new 3rd argument.

In the `Self { ... }` constructor, add:
```rust
subagent_progress: progress_store,
```

- [ ] **Step 3: Add GET /api/v1/subagent/progress handler**

In `src/daemon/handlers.rs`, add:
```rust
use crate::agent::progress::SubagentProgress;
use std::collections::HashMap;

/// GET /api/v1/subagent/progress
pub async fn get_subagent_progress(
    State(state): State<Arc<DaemonState>>,
) -> Json<HashMap<String, SubagentProgress>> {
    let store = state.subagent_progress.read().await;
    Json(store.clone())
}
```

- [ ] **Step 4: Register route**

In `src/daemon/routes.rs`, add the route:
```rust
.route(
    "/api/v1/subagent/progress",
    axum::routing::get(handlers::get_subagent_progress),
)
```

- [ ] **Step 5: Build check**

```bash
cargo check -p wgenty_code 2>&1 | head -40
```

Expected: may fail because `TaskTool::new()` and `RlmDelegateTool::new()` signatures changed but aren't updated yet. Fix them in Task 4.

- [ ] **Step 6: Commit**

```bash
git add src/daemon/state.rs src/daemon/handlers.rs src/daemon/routes.rs
git commit -m "feat(daemon): add SubagentProgressStore and polling endpoint

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 4: Update TaskTool and RlmDelegateTool to Accept Progress Store

**Files:**
- Modify: `src/tools/meta/task.rs`
- Modify: `src/tools/meta/rlm/mod.rs`

- [ ] **Step 1: Update TaskTool struct and constructor**

In `src/tools/meta/task.rs`, add import:
```rust
use crate::agent::progress::{ProgressCallback, SubagentProgress, SubagentStatus, SubagentMetadata};
use std::sync::Arc;
use tokio::sync::RwLock;
use std::collections::HashMap;
```

Add field to `TaskTool`:
```rust
progress_store: Arc<RwLock<HashMap<String, SubagentProgress>>>,
```

Update `TaskTool::new()`:
```rust
pub fn new(
    settings: Settings,
    tool_registry: std::sync::Weak<crate::tools::ToolRegistry>,
    background_manager: std::sync::Arc<crate::tools::execution::background::BackgroundManager>,
    progress_store: Arc<RwLock<HashMap<String, SubagentProgress>>>,
) -> Self {
    Self {
        settings,
        tool_registry,
        background_manager,
        active_count: Arc::new(AtomicUsize::new(0)),
        progress_store,
    }
}
```

- [ ] **Step 2: Add make_progress_callback helper**

```rust
impl TaskTool {
    fn make_progress_callback(
        store: Arc<RwLock<HashMap<String, SubagentProgress>>>,
        node_id: String,
        parent_id: Option<String>,
        label: String,
    ) -> ProgressCallback {
        Arc::new(move |mut progress: SubagentProgress| {
            progress.node_id = node_id.clone();
            progress.parent_id = parent_id.clone();
            progress.label = label.clone();
            let store = store.clone();
            let node_id = node_id.clone();
            tokio::spawn(async move {
                let mut store = store.write().await;
                store.insert(node_id, progress);
            });
        })
    }
}
```

- [ ] **Step 3: Wire callback in execute() method**

At the start of `execute()`, register a root node and generate `root_node_id`. For each subagent path (background `tokio::spawn` and synchronous), create a node_id + label + callback via `make_progress_callback`, register initial Pending state in the store, and pass `Some(cb)` instead of `None` to `run_subagent_loop` / `run_rlm_pipeline`.

- [ ] **Step 4: Update RlmDelegateTool similarly**

Add `progress_store` field, update constructor, pass callback to `run_rlm_pipeline` in `execute()`.

- [ ] **Step 5: Build check**

```bash
cargo check -p wgenty_code 2>&1 | head -40
```

Expected: compiles. All tool constructor call sites in `daemon/state.rs` already updated from Task 3.

- [ ] **Step 6: Commit**

```bash
git add src/tools/meta/task.rs src/tools/meta/rlm/mod.rs
git commit -m "feat(tools): wire progress callback in TaskTool and RlmDelegateTool

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 5: Add SubagentUpdate and ToggleSubagentPanel to AppEvent

**Files:**
- Modify: `src/tui/app/types.rs`

- [ ] **Step 1: Add new event variants**

Read `src/tui/app/types.rs`. Add import:
```rust
use crate::agent::progress::SubagentProgress;
```

Add to the `AppEvent` enum (before closing `}`):
```rust
/// A subagent progress update from daemon polling.
SubagentUpdate(SubagentProgress),
/// Toggle the subagent monitor panel.
ToggleSubagentPanel,
```

- [ ] **Step 2: Build check**

```bash
cargo check -p wgenty_code 2>&1 | head -30
```

Expected: compiler warns about unmatched variants in `handle_event` match (fixed in Task 9).

- [ ] **Step 3: Commit**

```bash
git add src/tui/app/types.rs
git commit -m "feat(tui): add SubagentUpdate and ToggleSubagentPanel to AppEvent

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 6: Implement SubagentTree State

**Files:**
- Create: `src/tui/components/subagent_tree.rs`

- [ ] **Step 1: Create SubagentTree with upsert logic**

```rust
//! SubagentTree — in-memory tree state for subagent execution progress.

use crate::agent::progress::{SubagentProgress, SubagentStatus};
use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct SubagentTree {
    pub root_id: Option<String>,
    pub nodes: HashMap<String, SubagentNode>,
}

#[derive(Debug, Clone)]
pub struct SubagentNode {
    pub progress: SubagentProgress,
    pub children: Vec<String>,
}

impl SubagentTree {
    pub fn upsert(&mut self, progress: SubagentProgress) {
        let node_id = progress.node_id.clone();
        let parent_id = progress.parent_id.clone();

        if parent_id.is_none() && self.root_id.is_none() {
            self.root_id = Some(node_id.clone());
        }

        if let Some(ref pid) = parent_id {
            if let Some(parent) = self.nodes.get_mut(pid) {
                if !parent.children.contains(&node_id) {
                    parent.children.push(node_id.clone());
                }
            }
        }

        match self.nodes.get_mut(&node_id) {
            Some(existing) => { existing.progress = progress; }
            None => {
                self.nodes.insert(node_id, SubagentNode { progress, children: Vec::new() });
            }
        }
    }

    pub fn count_by_status(&self, status: SubagentStatus) -> usize {
        self.nodes.values().filter(|n| n.progress.status == status).count()
    }

    pub fn is_complete(&self) -> bool {
        self.nodes.values().all(|n| matches!(
            n.progress.status,
            SubagentStatus::Completed | SubagentStatus::Failed | SubagentStatus::Cancelled
        ))
    }

    pub fn clear(&mut self) {
        self.root_id = None;
        self.nodes.clear();
    }
}
```

- [ ] **Step 2: Add unit tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn make_progress(node_id: &str, parent_id: Option<&str>, status: SubagentStatus) -> SubagentProgress {
        SubagentProgress {
            node_id: node_id.to_string(),
            parent_id: parent_id.map(String::from),
            label: format!("Node {}", node_id),
            status,
            round: None, max_rounds: None, current_tool: None,
            started_at: 0, elapsed_ms: 0, metadata: None,
        }
    }

    #[test]
    fn test_upsert_creates_tree() {
        let mut tree = SubagentTree::default();
        tree.upsert(make_progress("root", None, SubagentStatus::Running));
        assert_eq!(tree.root_id.as_deref(), Some("root"));
        assert_eq!(tree.nodes.len(), 1);
        tree.upsert(make_progress("child1", Some("root"), SubagentStatus::Running));
        assert_eq!(tree.nodes.len(), 2);
        assert_eq!(tree.nodes["root"].children, vec!["child1"]);
        tree.upsert(make_progress("child1", Some("root"), SubagentStatus::Completed));
        assert_eq!(tree.nodes["child1"].progress.status, SubagentStatus::Completed);
    }

    #[test]
    fn test_is_complete() {
        let mut tree = SubagentTree::default();
        tree.upsert(make_progress("root", None, SubagentStatus::Completed));
        tree.upsert(make_progress("a", Some("root"), SubagentStatus::Completed));
        assert!(tree.is_complete());
        tree.upsert(make_progress("b", Some("root"), SubagentStatus::Running));
        assert!(!tree.is_complete());
    }

    #[test]
    fn test_count_by_status() {
        let mut tree = SubagentTree::default();
        tree.upsert(make_progress("root", None, SubagentStatus::Completed));
        tree.upsert(make_progress("a", Some("root"), SubagentStatus::Completed));
        tree.upsert(make_progress("b", Some("root"), SubagentStatus::Running));
        tree.upsert(make_progress("c", Some("root"), SubagentStatus::Pending));
        assert_eq!(tree.count_by_status(SubagentStatus::Completed), 2);
        assert_eq!(tree.count_by_status(SubagentStatus::Running), 1);
        assert_eq!(tree.count_by_status(SubagentStatus::Pending), 1);
    }
}
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p wgenty_code subagent_tree -- --nocapture 2>&1
```

Expected: 3 tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/tui/components/subagent_tree.rs
git commit -m "feat(tui): add SubagentTree state with upsert logic and tests

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 7: Add SubagentProgress Polling to DaemonClient

**Files:**
- Modify: `src/tui/client.rs`

- [ ] **Step 1: Add poll_subagent_progress method**

Read `src/tui/client.rs`. Add import:
```rust
use std::collections::HashMap;
```

After the existing `get_background_results` method (~line 158), add:
```rust
/// GET /api/v1/subagent/progress — poll subagent execution progress.
pub async fn poll_subagent_progress(
    &self,
) -> anyhow::Result<HashMap<String, crate::agent::progress::SubagentProgress>> {
    let url = format!("{}/api/v1/subagent/progress", self.base_url);
    let resp = self.http_tools.get(&url).send().await?;
    if !resp.status().is_success() {
        return Ok(HashMap::new());
    }
    Ok(resp.json().await?)
}
```

- [ ] **Step 2: Build check**

```bash
cargo check -p wgenty_code 2>&1 | head -20
```

Expected: compiles.

- [ ] **Step 3: Commit**

```bash
git add src/tui/client.rs
git commit -m "feat(client): add poll_subagent_progress to DaemonClient

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 8: Wire Progress Polling in Agent Loop

**Files:**
- Modify: `src/tui/agent/tool_dispatch.rs`
- Modify: `src/tui/agent/core.rs`

- [ ] **Step 1: Add progress polling to execute_tool_static**

Read `src/tui/agent/tool_dispatch.rs`. Modify `execute_tool_static` to accept an optional `event_tx`:

```rust
pub(super) async fn execute_tool_static(
    client: &DaemonClient,
    name: &str,
    args: serde_json::Value,
    session_id: &str,
    event_tx: Option<mpsc::UnboundedSender<AppEvent>>,
) -> String {
```

After the guardian check, before the main tool execution, add polling for `task` and `delegate` tools:

```rust
let is_long_running = name == "task" || name == "delegate";
let poll_handle = if is_long_running && event_tx.is_some() {
    let tx = event_tx.as_ref().unwrap().clone();
    let client_clone = client.clone();
    Some(tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(500)).await;
            match client_clone.poll_subagent_progress().await {
                Ok(map) => {
                    for (_id, progress) in map {
                        let _ = tx.send(AppEvent::SubagentUpdate(progress));
                    }
                }
                Err(_) => break,
            }
        }
    }))
} else {
    None
};

// Execute tool (existing logic — uses client.execute_tool(...))
let result = match client.execute_tool(name, args, session_id).await {
    Ok(resp) => { /* existing formatting code, unchanged */ },
    Err(e) => format!(r#"{{"success":false,"error":"{}"}}"#, e),
};

// Stop poller
if let Some(handle) = poll_handle {
    handle.abort();
}

result
```

- [ ] **Step 2: Update call sites in core.rs**

In `src/tui/agent/core.rs`, find all calls to `Self::execute_tool_static(...)` and add `Some(self.event_tx.clone())` as the new last argument. There are two locations:
- Sequential execution path (~line 250): `Self::execute_tool_static(&self.client, &name, args.clone(), &self.session_id)`
- Parallel execution path (~line 166): `Self::execute_tool_static(&client, &name, args.clone(), &session_id)`

Both need `Some(event_tx.clone())` added.

- [ ] **Step 3: Build check**

```bash
cargo check -p wgenty_code 2>&1 | head -40
```

Expected: compiles.

- [ ] **Step 4: Commit**

```bash
git add src/tui/agent/tool_dispatch.rs src/tui/agent/core.rs
git commit -m "feat(tui): poll subagent progress during task/delegate execution

Polls GET /api/v1/subagent/progress every 500ms while a long-running tool
executes, emitting AppEvent::SubagentUpdate for real-time UI updates.

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 9: Handle New Events in App + Wire State

**Files:**
- Modify: `src/tui/app/event.rs`
- Modify: `src/tui/app/mod.rs`

- [ ] **Step 1: Add subagent fields to App struct**

Read `src/tui/app/mod.rs`. Add to the `App` struct:
```rust
subagent_tree: crate::tui::components::subagent_tree::SubagentTree,
subagent_panel_visible: bool,
```

Initialize in `App::new()`:
```rust
subagent_tree: SubagentTree::default(),
subagent_panel_visible: false,
```

- [ ] **Step 2: Handle events in event.rs**

Read `src/tui/app/event.rs`. In the `handle_event` match, add:

```rust
AppEvent::SubagentUpdate(progress) => {
    self.subagent_tree.upsert(progress);
}
AppEvent::ToggleSubagentPanel => {
    self.subagent_panel_visible = !self.subagent_panel_visible;
}
```

In the `Submit` handler (new turn start), add:
```rust
self.subagent_tree.clear();
```

- [ ] **Step 3: Build check**

```bash
cargo check -p wgenty_code 2>&1 | head -30
```

Expected: compiles.

- [ ] **Step 4: Commit**

```bash
git add src/tui/app/event.rs src/tui/app/mod.rs
git commit -m "feat(tui): handle SubagentUpdate and ToggleSubagentPanel events

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 10: Render Inline Subagent Tree Card in Chat

**Files:**
- Modify: `src/tui/components/chat.rs`

- [ ] **Step 1: Add status_icon helper and render_subagent_card function**

In `src/tui/components/chat.rs`, add:

```rust
use crate::agent::progress::SubagentStatus;

fn status_icon(status: &SubagentStatus) -> &'static str {
    match status {
        SubagentStatus::Pending => "⏳",
        SubagentStatus::Running => "🔄",
        SubagentStatus::Completed => "✅",
        SubagentStatus::Failed => "❌",
        SubagentStatus::Cancelled => "🚫",
    }
}

pub fn render_subagent_card(
    lines: &mut Vec<Line>,
    tree: &crate::tui::components::subagent_tree::SubagentTree,
    width: u16,
    is_executing: bool,
    spinner_frame: u8,
) {
    if tree.nodes.is_empty() { return; }
    let done = tree.count_by_status(SubagentStatus::Completed);
    let total = tree.nodes.len();
    let indent = 4u16;

    // Header
    let spinner = if is_executing {
        SPINNER_CHARS[(spinner_frame as usize) % SPINNER_CHARS.len()]
    } else { ' ' };
    lines.push(Line::from(vec![
        Span::styled("  🌳 Subagent Tree", Style::default().fg(Color::Rgb(203, 166, 247)).add_modifier(Modifier::BOLD)),
        Span::styled(format!("  {} {}/{} done", spinner, done, total), Style::default().fg(DIM_COLOR)),
    ]));

    if is_executing {
        render_tree_nodes(lines, tree, tree.root_id.as_deref(), 0, indent);
    } else {
        let failed = tree.count_by_status(SubagentStatus::Failed);
        let icon = if failed > 0 { "⚠️" } else { "✅" };
        lines.push(Line::from(vec![
            Span::styled(format!("    {} {} · {}/{} done", icon, "task", done, total), Style::default().fg(DIM_COLOR)),
        ]));
    }
}

fn render_tree_nodes(
    lines: &mut Vec<Line>,
    tree: &crate::tui::components::subagent_tree::SubagentTree,
    node_id: Option<&str>,
    depth: u16,
    base_indent: u16,
) {
    let Some(nid) = node_id else { return };
    let Some(node) = tree.nodes.get(nid) else { return };
    let indent = base_indent + depth * 2;
    let prefix = if depth == 0 { "┌─" } else { "├─" };
    let indent_str = " ".repeat(indent as usize);
    let icon = status_icon(&node.progress.status);
    let color = match node.progress.status {
        SubagentStatus::Running => Color::Rgb(249, 226, 175),
        SubagentStatus::Completed => Color::Rgb(166, 227, 161),
        SubagentStatus::Failed | SubagentStatus::Cancelled => Color::Rgb(243, 139, 168),
        SubagentStatus::Pending => Color::Rgb(108, 112, 134),
    };

    let detail = format!("{} {}", node.progress.label, match node.progress.status {
        SubagentStatus::Running => {
            match (node.progress.round, node.progress.max_rounds) {
                (Some(r), Some(mr)) => format!("round {}/{}", r, mr),
                _ => String::new(),
            }
        }
        SubagentStatus::Completed => {
            node.progress.round.map(|r| format!("{} rounds", r)).unwrap_or_default()
        }
        _ => String::new(),
    });

    lines.push(Line::from(vec![
        Span::styled(format!("{}{} ", indent_str, prefix), Style::default().fg(DIM_COLOR)),
        Span::styled(icon, Style::default().fg(color)),
        Span::styled(format!(" {}", detail), Style::default().fg(color)),
    ]));

    if node.progress.status == SubagentStatus::Running {
        if let Some(ref tool) = node.progress.current_tool {
            let tool_indent = " ".repeat((indent + 4) as usize);
            lines.push(Line::from(vec![
                Span::styled(format!("{}└─ 🛠 executing: {}", tool_indent, tool),
                    Style::default().fg(Color::Rgb(137, 180, 250))),
            ]));
        }
    }

    for child_id in &node.children {
        render_tree_nodes(lines, tree, Some(child_id), depth + 1, base_indent);
    }
}
```

- [ ] **Step 2: Integrate into render function**

Update the `render` function signature to accept:
```rust
subagent_tree: Option<&crate::tui::components::subagent_tree::SubagentTree>,
subagent_is_executing: bool,
```

In the tool message rendering section, after rendering a tool message with name `"task"` or `"delegate"`, call:
```rust
if let Some(tree) = subagent_tree {
    if !tree.nodes.is_empty() {
        render_subagent_card(&mut lines, tree, area.width, subagent_is_executing, spinner_frame);
    }
}
```

- [ ] **Step 3: Update call site**

Find where `chat::render()` is called (in the main render function in `src/tui/app/`). Pass:
```rust
subagent_tree: Some(&self.subagent_tree),
subagent_is_executing: self.subagent_tree.count_by_status(SubagentStatus::Running) > 0,
```

- [ ] **Step 4: Build check**

```bash
cargo check -p wgenty_code 2>&1 | head -30
```

Expected: compiles.

- [ ] **Step 5: Commit**

```bash
git add src/tui/components/chat.rs
git commit -m "feat(tui): render inline subagent tree card in chat messages

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 11: Add Dedicated Subagent Monitor Panel

**Files:**
- Create: `src/tui/components/subagent_panel.rs`
- Modify: `src/tui/components/mod.rs`

- [ ] **Step 1: Create panel widget**

```rust
//! Subagent Monitor Panel — toggleable panel (Ctrl+Shift+T) showing full tree.

use crate::agent::progress::SubagentStatus;
use super::subagent_tree::SubagentTree;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

pub fn render(f: &mut Frame, area: Rect, tree: &SubagentTree, is_executing: bool) {
    let panel = Block::default()
        .title(format!(
            " 🌳 Subagent Monitor — {} agents · {} active — Ctrl+Shift+T toggle ",
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
        .constraints([Constraint::Length(1), Constraint::Min(0), Constraint::Length(1)])
        .split(inner);

    // Summary bar
    let done = tree.count_by_status(SubagentStatus::Completed);
    let running = tree.count_by_status(SubagentStatus::Running);
    let pending = tree.count_by_status(SubagentStatus::Pending);
    let failed = tree.count_by_status(SubagentStatus::Failed);
    f.render_widget(Paragraph::new(Line::from(vec![
        Span::styled(format!(" ✅ {} done  ", done), Style::default().fg(Color::Rgb(166, 227, 161))),
        Span::styled(format!("🔄 {} running  ", running), Style::default().fg(Color::Rgb(249, 226, 175))),
        Span::styled(format!("⏳ {} pending  ", pending), Style::default().fg(Color::Rgb(108, 112, 134))),
        Span::styled(format!("❌ {} failed", failed), Style::default().fg(Color::Rgb(243, 139, 168))),
    ])), chunks[0]);

    // Tree body
    let mut tree_lines: Vec<Line> = Vec::new();
    super::chat::render_subagent_card(&mut tree_lines, tree, inner.width, is_executing, 0);
    f.render_widget(Paragraph::new(ratatui::text::Text::from(tree_lines)).wrap(Wrap { trim: false }), chunks[1]);

    // Info line
    let info = tree.root_id.as_ref()
        .and_then(|rid| tree.nodes.get(rid))
        .map(|root| match &root.progress.status {
            SubagentStatus::Running => format!(" ℹ️  {} — elapsed {}s", root.progress.label, root.progress.elapsed_ms / 1000),
            SubagentStatus::Completed => format!(" ✅ Completed — {} nodes in {}s", tree.nodes.len(), root.progress.elapsed_ms / 1000),
            _ => format!(" {} subagents tracked", tree.nodes.len()),
        })
        .unwrap_or_else(|| " No active subagent execution".to_string());
    f.render_widget(
        Paragraph::new(Span::styled(info, Style::default().fg(Color::Rgb(108, 112, 134)))),
        chunks[2],
    );
}
```

- [ ] **Step 2: Export modules**

In `src/tui/components/mod.rs`, add:
```rust
pub mod subagent_tree;
pub mod subagent_panel;
```

- [ ] **Step 3: Integrate into main render**

In the main app render function (in `src/tui/app/`), when `self.subagent_panel_visible`:
```rust
if self.subagent_panel_visible {
    let panel_area = crate::tui::util::centered_rect(60, 70, f.size());
    crate::tui::components::subagent_panel::render(
        f, panel_area, &self.subagent_tree,
        self.subagent_tree.count_by_status(SubagentStatus::Running) > 0,
    );
}
```

If `centered_rect` doesn't exist in `util.rs`, add it:
```rust
pub fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);
    Layout::default()
        .direction(ratatui::layout::Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
```

- [ ] **Step 4: Register Ctrl+Shift+T keybinding**

In the key event handler (`event.rs`), find where keyboard shortcuts are processed. Add:
```rust
// Ctrl+Shift+T toggles subagent monitor panel
if key.modifiers.contains(KeyModifiers::CONTROL)
    && key.modifiers.contains(KeyModifiers::SHIFT)
    && (key.code == KeyCode::Char('T') || key.code == KeyCode::Char('t'))
{
    let _ = self.event_tx.send(AppEvent::ToggleSubagentPanel);
    return;
}
```

- [ ] **Step 5: Build check**

```bash
cargo check -p wgenty_code 2>&1 | head -30
```

Expected: compiles.

- [ ] **Step 6: Commit**

```bash
git add src/tui/components/subagent_panel.rs src/tui/components/mod.rs
git commit -m "feat(tui): add dedicated Subagent Monitor panel (Ctrl+Shift+T)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 12: History Persistence — Store Tree Snapshot per Turn

**Files:**
- Modify: `src/tui/app/mod.rs`
- Modify: `src/tui/app/event.rs`

- [ ] **Step 1: Add history map to App**

In `src/tui/app/mod.rs`, add to `App` struct:
```rust
subagent_history: HashMap<String, crate::tui::components::subagent_tree::SubagentTree>,
```

Initialize in `App::new()`:
```rust
subagent_history: HashMap::new(),
```

- [ ] **Step 2: Store snapshot on TurnComplete**

In the `TurnComplete` handler in `event.rs`, BEFORE clearing:
```rust
let snapshot = self.subagent_tree.clone();
self.subagent_history.insert(self.current_turn_id.clone(), snapshot);
```

- [ ] **Step 3: Render historical snapshots**

In the chat `render` function, when rendering historical messages (not the current turn), look up the tree from `subagent_history` by `turn_id` and render the static (collapsed) summary card.

- [ ] **Step 4: Build check**

```bash
cargo check -p wgenty_code 2>&1 | head -30
```

- [ ] **Step 5: Commit**

```bash
git add src/tui/app/mod.rs src/tui/app/event.rs src/tui/components/chat.rs
git commit -m "feat(tui): persist subagent tree snapshots for history review

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Summary

| Task | Description | Key Files |
|------|-------------|-----------|
| 1 | Define `SubagentProgress` types + `ProgressCallback` | `agent/progress.rs` (NEW) |
| 2 | Add `on_progress` to `run_subagent_loop` + update call sites | `teams/subagent_loop.rs`, `tools/meta/{task,run_script,rlm}` |
| 3 | Daemon `SubagentProgressStore` + polling endpoint | `daemon/{state,handlers,routes}.rs` |
| 4 | Wire callback in `TaskTool` + `RlmDelegateTool` | `tools/meta/{task,rlm/mod}.rs` |
| 5 | Add events to `AppEvent` | `tui/app/types.rs` |
| 6 | `SubagentTree` state with upsert + tests | `tui/components/subagent_tree.rs` (NEW) |
| 7 | `poll_subagent_progress()` in `DaemonClient` | `tui/client.rs` |
| 8 | Polling loop in `execute_tool_static` | `tui/agent/{tool_dispatch,core}.rs` |
| 9 | Handle events + wire state into App | `tui/app/{event,mod}.rs` |
| 10 | Inline subagent card in chat rendering | `tui/components/chat.rs` |
| 11 | Dedicated monitor panel (Ctrl+Shift+T) | `tui/components/{subagent_panel,mod}.rs` |
| 12 | History persistence | `tui/app/{event,mod}.rs`, `chat.rs` |
