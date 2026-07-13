# Scoped Subagent Hierarchy and Chat Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every subagent focus window show `main + current agent + direct children`, support recursive capability navigation, and keep the conversation area synchronized with the selected agent.

**Architecture:** Preserve daemon-enforced local views rather than rebuilding a global tree. Enrich the local self projection with UI transcript data, resolve navigation targets from server-side capability grants, and make the TUI navigation stack own root caching, child refresh, breadcrumb state, selector identity, and conversation selection.

**Tech Stack:** Rust, Tokio, Axum, Ratatui, Serde, existing `CapabilityService`, `AgentCoordinator`, `DaemonClient`, and Cargo test tooling.

---

## File Map

- `src/daemon/models.rs`: wire representation for complete self and child UI projections.
- `src/daemon/handlers.rs`: build scoped display projections and resolve recursive navigation targets.
- `src/agent/capability.rs`: safely resolve a stored capability without accepting a raw target ID.
- `src/agent/store.rs`: trusted crate-private lookup used after capability verification.
- `src/agent/coordinator.rs`: narrow trusted-UI adapter from a verified target to a local view.
- `src/tui/client.rs`: root-view and capability-view request methods.
- `src/tui/app/types.rs`: navigation frames, selection identity, refresh events, and breadcrumb data.
- `src/tui/app/event.rs`: apply root updates, navigation responses, scoped refreshes, and back/main restoration atomically.
- `src/tui/app/event_key.rs`: selector movement updates the displayed conversation immediately; Enter changes scope.
- `src/tui/components/subagent_tree.rs`: explicit self/direct-child projection without grouping-node filtering.
- `src/tui/components/subagent_focus_view.rs`: selector rows, selected-agent conversation, and breadcrumbs.
- `src/tui/agent/core.rs`: sequential task poller continues emitting root snapshots.
- `src/tui/agent/tool_dispatch.rs`: parallel task poller continues emitting root snapshots.
- `tests/strict_subagent_isolation.rs`: recursive navigation and non-leakage integration contracts.
- `CHANGELOG.md`: user-visible fix entry.

### Task 1: Complete the Self Projection

**Files:**
- Modify: `src/daemon/models.rs:204-239`
- Modify: `src/daemon/handlers.rs:620-674`
- Modify: `src/tui/components/subagent_tree.rs:55-129`
- Test: `src/tui/components/subagent_tree.rs`

- [ ] **Step 1: Write the failing self-projection tree test**

Add a helper that builds a complete self response and a test proving navigation does not erase the selected agent's messages:

```rust
fn self_response(id: &str, label: &str, messages: Vec<ChatMessage>) -> SelfAgentResponse {
    SelfAgentResponse {
        agent_id: id.to_string(),
        status: AgentLifecycleStatus::Running,
        label: label.to_string(),
        text_snapshot: Some(format!("snapshot-{id}")),
        cumulative_tokens: 42,
        messages,
    }
}

#[test]
fn replace_local_populates_self_conversation_and_metadata() {
    let messages = vec![ChatMessage::assistant("child answer")];
    let mut tree = SubagentTree::default();
    tree.replace_local(LocalAgentViewResponse {
        self_view: self_response("child", "Child task", messages.clone()),
        children: Vec::new(),
    });

    let self_node = &tree.nodes["child"].progress;
    assert_eq!(self_node.label, "Child task");
    assert_eq!(self_node.messages, messages);
    assert_eq!(self_node.text_snapshot.as_deref(), Some("snapshot-child"));
    assert_eq!(self_node.cumulative_tokens, 42);
}
```

- [ ] **Step 2: Run the test and verify RED**

Run:

```bash
cargo test tui::components::subagent_tree::tests::replace_local_populates_self_conversation_and_metadata --lib
```

Expected: compilation fails because `SelfAgentResponse` lacks the display fields.

- [ ] **Step 3: Add complete fields to `SelfAgentResponse`**

Implement:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfAgentResponse {
    pub agent_id: String,
    pub status: AgentLifecycleStatus,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub text_snapshot: Option<String>,
    #[serde(default)]
    pub cumulative_tokens: u64,
    #[serde(default)]
    pub messages: Vec<crate::api::ChatMessage>,
}
```

In `build_local_view`, cross-populate the self projection from `session_progress` exactly as children are populated:

```rust
let self_progress = session_progress.get(view.self_view.agent_id.as_str());
let self_record = state
    .coordinator
    .trusted_ui_record(&caller.session_id, &view.self_view.agent_id)
    .await
    .map_err(|_| StatusCode::NOT_FOUND)?;

self_view: SelfAgentResponse {
    agent_id: view.self_view.agent_id.as_str().to_string(),
    status: view.self_view.status,
    label: self_record.label,
    text_snapshot: self_progress.and_then(|p| p.text_snapshot.clone()),
    cumulative_tokens: self_progress.map(|p| p.cumulative_tokens).unwrap_or(0),
    messages: self_progress.map(|p| p.messages.clone()).unwrap_or_default(),
}
```

Add `AgentCoordinator::trusted_ui_record` as `pub(crate)` and document that callers must verify trusted UI authority before using it:

```rust
pub(crate) async fn trusted_ui_record(
    &self,
    session: &SessionId,
    agent: &AgentId,
) -> Result<AgentRecord, CoordinatorError> {
    self.store.record_for_trusted_ui(session, agent).await.map_err(Into::into)
}
```

Add the matching crate-private store lookup:

```rust
pub(crate) async fn record_for_trusted_ui(
    &self,
    session: &SessionId,
    agent: &AgentId,
) -> Result<AgentRecord, StoreError> {
    self.state
        .read()
        .await
        .records
        .get(&(session.clone(), agent.clone()))
        .cloned()
        .ok_or(StoreError::NotVisible)
}
```

Populate the self node directly from `view.self_view` in `replace_local`; do not clone an arbitrary previous node after `self.nodes.clear()`.

- [ ] **Step 4: Run focused tests and verify GREEN**

Run:

```bash
cargo test tui::components::subagent_tree --lib
cargo test agent::store::tests --lib
```

Expected: all selected tests pass.

- [ ] **Step 5: Commit the projection fix**

```bash
git add src/daemon/models.rs src/daemon/handlers.rs src/agent/store.rs src/agent/coordinator.rs src/tui/components/subagent_tree.rs
git commit -m "fix(tui): preserve scoped agent conversation data"
```

### Task 2: Resolve Recursive Navigation Capabilities

**Files:**
- Modify: `src/agent/capability.rs:221-368`
- Modify: `src/daemon/handlers.rs:708-769`
- Test: `src/agent/capability.rs`
- Test: `tests/strict_subagent_isolation.rs`

- [ ] **Step 1: Write failing capability-resolution tests**

Add a resolved target projection and tests that the token itself selects the target while viewer/session/operation remain enforced:

```rust
#[tokio::test]
async fn resolve_navigation_returns_bound_target_only() {
    let service = CapabilityService::with_clock(test_secret(), fixed_clock());
    let token = service
        .issue(&CapabilityGrant::navigate("viewer", "session", "grandchild", 7))
        .await;

    let resolved = service
        .resolve_navigation(&token, "viewer", "session")
        .await
        .unwrap();
    assert_eq!(resolved.target.as_str(), "grandchild");
    assert_eq!(resolved.generation, 7);
    assert_eq!(
        service.resolve_navigation(&token, "other-viewer", "session").await,
        Err(CapabilityError::NotVisible)
    );
}
```

Also issue a `Transcript` capability and assert `resolve_navigation` rejects it.

- [ ] **Step 2: Run the test and verify RED**

Run:

```bash
cargo test agent::capability::tests::resolve_navigation --lib
```

Expected: compilation fails because `ResolvedCapability` and `resolve_navigation` do not exist.

- [ ] **Step 3: Implement server-side target resolution**

Add:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCapability {
    pub target: AgentId,
    pub generation: u64,
}

pub async fn resolve_navigation(
    &self,
    token: &str,
    viewer: impl Into<String>,
    session: impl Into<String>,
) -> Result<ResolvedCapability, CapabilityError> {
    let viewer = ViewerId::new(viewer);
    let session_id = SessionId::new(session);
    let digest = self.digest(token);
    let now = self.clock.now();
    let grants = self.grants.read().await;
    let stored = grants.get(&digest).ok_or(CapabilityError::NotVisible)?;
    if stored.expires_at <= now
        || stored.viewer != viewer
        || stored.session_id != session_id
        || stored.operation != CapabilityOperation::Navigate
    {
        return Err(CapabilityError::NotVisible);
    }
    Ok(ResolvedCapability {
        target: stored.target.clone(),
        generation: stored.generation,
    })
}
```

Keep the existing exact-target `verify` API for transcript/cancel callers.

Replace the root-child scan in `navigate_agent_view` with `resolve_navigation`, then load the verified target through the trusted coordinator adapter. Do not accept an agent ID from the URL or query string.

- [ ] **Step 4: Add the recursive navigation integration assertion**

Seed `root -> child -> grandchild`, issue the child capability, resolve/load the child view, take the freshly issued grandchild capability, and assert the second navigation returns `grandchild` as self. Assert the root local view still excludes `grandchild`.

```rust
assert_eq!(root_view.self_view.agent_id, "root");
assert!(root_view.children.iter().all(|child| child.agent_id != "grandchild"));
assert_eq!(child_view.self_view.agent_id, "child");
assert_eq!(grandchild_view.self_view.agent_id, "grandchild");
```

- [ ] **Step 5: Run capability and isolation tests**

Run:

```bash
cargo test agent::capability --lib
cargo test --test strict_subagent_isolation navigation_
```

Expected: all tests pass; forged and mismatched capabilities remain `NotVisible`.

- [ ] **Step 6: Commit recursive capability navigation**

```bash
git add src/agent/capability.rs src/daemon/handlers.rs tests/strict_subagent_isolation.rs
git commit -m "fix(agent): resolve recursive navigation capabilities"
```

### Task 3: Build Explicit Scoped Selector Entries

**Files:**
- Modify: `src/tui/components/subagent_tree.rs:131-292`
- Modify: `src/tui/components/subagent_focus_view.rs:21-46`
- Test: `src/tui/components/subagent_tree.rs`
- Test: `src/tui/components/subagent_focus_view.rs`

- [ ] **Step 1: Write failing root and child selector tests**

Introduce an explicit selector entry type:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopedSelectorEntry {
    Main,
    SelfAgent(String),
    DirectChild(String),
}
```

Add tests:

```rust
#[test]
fn root_selector_is_main_plus_direct_children_without_duplicate_self() {
    let tree = local_tree("root", &["child-a", "child-b"]);
    assert_eq!(
        tree.scoped_selector_entries("root"),
        vec![
            ScopedSelectorEntry::Main,
            ScopedSelectorEntry::DirectChild("child-a".into()),
            ScopedSelectorEntry::DirectChild("child-b".into()),
        ]
    );
}

#[test]
fn child_selector_is_main_self_and_direct_children() {
    let tree = local_tree("child", &["grandchild"]);
    assert_eq!(
        tree.scoped_selector_entries("root"),
        vec![
            ScopedSelectorEntry::Main,
            ScopedSelectorEntry::SelfAgent("child".into()),
            ScopedSelectorEntry::DirectChild("grandchild".into()),
        ]
    );
}
```

- [ ] **Step 2: Run the tests and verify RED**

Run:

```bash
cargo test tui::components::subagent_tree::tests::root_selector --lib
cargo test tui::components::subagent_tree::tests::child_selector --lib
```

Expected: compilation fails because scoped selector entries do not exist.

- [ ] **Step 3: Implement explicit selector construction**

Add `root_agent_id` as an argument owned by navigation state, not inferred from `parent_id`:

```rust
pub fn scoped_selector_entries(&self, root_agent_id: &str) -> Vec<ScopedSelectorEntry> {
    let Some(view) = &self.local_view else { return Vec::new(); };
    let mut entries = vec![ScopedSelectorEntry::Main];
    if view.self_view.agent_id != root_agent_id {
        entries.push(ScopedSelectorEntry::SelfAgent(view.self_view.agent_id.clone()));
    }
    entries.extend(
        view.children
            .iter()
            .map(|child| ScopedSelectorEntry::DirectChild(child.agent_id.clone())),
    );
    entries
}
```

Replace scoped focus-view use of `real_node_list()` with these explicit entries. Retain `real_node_list()` only for legacy status/count paths that still require it.

- [ ] **Step 4: Verify the current self cannot be filtered as a grouping node**

Add a regression test where self has one child and zero messages, then assert `SelfAgent("child")` remains present.

- [ ] **Step 5: Run component tests and commit**

Run:

```bash
cargo test tui::components::subagent_tree --lib
cargo test tui::components::subagent_focus_view --lib
```

Expected: all tests pass.

```bash
git add src/tui/components/subagent_tree.rs src/tui/components/subagent_focus_view.rs
git commit -m "fix(tui): render explicit scoped agent hierarchy"
```

### Task 4: Make Selection Own the Conversation

**Files:**
- Modify: `src/tui/app/types.rs:396-414`
- Modify: `src/tui/app/event_key.rs:18-99`
- Modify: `src/tui/components/subagent_focus_view.rs:177-292`
- Test: `src/tui/components/subagent_focus_view.rs`

- [ ] **Step 1: Write failing selected-conversation tests**

Replace numeric-only selection with an identity-bearing selection:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentSelection {
    Main,
    Agent(String),
}
```

Add tests that resolve messages by selected identity:

```rust
#[test]
fn selected_child_uses_child_messages() {
    let main = vec![ChatMessage::assistant("main")];
    let tree = tree_with_messages("child", "child chat", "grandchild", "grandchild chat");
    assert_eq!(
        conversation_for_selection(&AgentSelection::Agent("grandchild".into()), &tree, &main),
        vec![ChatMessage::assistant("grandchild chat")]
    );
}

#[test]
fn selected_main_uses_main_history() {
    let main = vec![ChatMessage::assistant("main")];
    assert_eq!(
        conversation_for_selection(&AgentSelection::Main, &SubagentTree::default(), &main),
        main
    );
}
```

- [ ] **Step 2: Run tests and verify RED**

Run:

```bash
cargo test tui::components::subagent_focus_view::tests::selected_ --lib
```

Expected: compilation fails because `AgentSelection` and `conversation_for_selection` do not exist.

- [ ] **Step 3: Implement atomic selection resolution**

Add:

```rust
pub fn conversation_for_selection(
    selection: &AgentSelection,
    tree: &SubagentTree,
    main_history: &[ChatMessage],
) -> Vec<ChatMessage> {
    match selection {
        AgentSelection::Main => main_history.to_vec(),
        AgentSelection::Agent(id) => tree
            .nodes
            .get(id)
            .map(|node| node.progress.messages.clone())
            .unwrap_or_default(),
    }
}
```

Store `selection: AgentSelection` on `FocusViewState` and `AgentViewFrame`. Keep `selector_index` only as a derived render/navigation position, or replace it with helpers that calculate the index from the current entry list.

On Up/Down:

1. calculate the next `ScopedSelectorEntry`;
2. update `focus.selection`;
3. copy the selected agent's label/status/metrics/messages into the focus display state in one method call;
4. render immediately without waiting for Enter.

On Enter:

- `Main`: restore the root frame;
- `SelfAgent`: keep the current scope;
- `DirectChild`: dispatch capability navigation.

- [ ] **Step 4: Add refresh fallback behavior**

When the selected child is absent after a refresh, select the current scoped self. At root, fall back to `Main`.

```rust
let fallback = if current_scope_id == root_agent_id {
    AgentSelection::Main
} else {
    AgentSelection::Agent(current_scope_id.to_string())
};
```

- [ ] **Step 5: Run focus-view and key handling tests**

Run:

```bash
cargo test tui::components::subagent_focus_view --lib
cargo test tui::app --lib
```

Expected: selector movement changes conversation content and all tests pass.

- [ ] **Step 6: Commit selection/chat synchronization**

```bash
git add src/tui/app/types.rs src/tui/app/event_key.rs src/tui/components/subagent_focus_view.rs
git commit -m "fix(tui): sync selected agent conversation"
```

### Task 5: Preserve and Refresh the Active Navigation Frame

**Files:**
- Modify: `src/tui/app/types.rs:168-188,396-414`
- Modify: `src/tui/app/event.rs:609-715`
- Modify: `src/tui/client.rs:130-165`
- Test: `src/tui/app/event.rs`

- [ ] **Step 1: Write failing navigation-state tests**

Add pure state transition tests proving a root poll does not replace an active child frame:

```rust
#[test]
fn root_update_is_cached_while_child_scope_is_active() {
    let mut navigation = navigation_at_child();
    let child_id = navigation.current.as_ref().unwrap().view.self_view.agent_id.clone();
    navigation.apply_root_view(root_view_with_child_status(AgentLifecycleStatus::Completed));

    assert_eq!(navigation.current.as_ref().unwrap().view.self_view.agent_id, child_id);
    assert_eq!(navigation.root.as_ref().unwrap().view.children[0].status,
        AgentLifecycleStatus::Completed);
}
```

Add a second test proving a scoped refresh replaces only the current frame and preserves back-stack depth and selection.

- [ ] **Step 2: Run tests and verify RED**

Run:

```bash
cargo test tui::app::event::tests::root_update_is_cached --lib
cargo test tui::app::event::tests::scoped_refresh --lib
```

Expected: compilation fails because root/current refresh transitions are not separated.

- [ ] **Step 3: Extend navigation frames and events**

Use:

```rust
pub struct AgentViewFrame {
    pub view: LocalAgentViewResponse,
    pub selection: AgentSelection,
    pub breadcrumb_label: String,
    pub entry_capability: Option<String>,
}

pub struct AgentNavigationState {
    pub root_agent_id: Option<String>,
    pub root: Option<AgentViewFrame>,
    pub current: Option<AgentViewFrame>,
    pub back_stack: Vec<AgentViewFrame>,
}
```

Add distinct events:

```rust
AgentViewNavigated {
    capability: String,
    view: Box<LocalAgentViewResponse>,
},
AgentViewRefreshed(Box<LocalAgentViewResponse>),
RefreshCurrentAgentView,
NavigateToMain,
```

- [ ] **Step 4: Apply root updates without replacing a child frame**

For `AgentLocalView`:

- initialize or update `navigation.root`;
- if current is root, replace the displayed tree and rebuild focus;
- if current is non-root, retain the displayed tree and dispatch
  `RefreshCurrentAgentView` using `current.entry_capability`.

For `AgentViewRefreshed`:

- verify the returned self ID matches the current frame's self ID;
- replace only the current frame view;
- rebuild tree, selector, and conversation atomically;
- preserve back stack and entry capability.

Do not log the raw capability in warnings.

- [ ] **Step 5: Implement refresh through the existing capability URL**

Reuse `DaemonClient::navigate_agent_view` for refresh. A refresh response must emit `AgentViewRefreshed`, not `AgentViewNavigated`, so it never pushes another history frame.

- [ ] **Step 6: Run navigation-state tests and commit**

Run:

```bash
cargo test tui::app --lib
cargo test tui::components::subagent_focus_view --lib
```

Expected: all tests pass and root updates remain cached while a child scope is active.

```bash
git add src/tui/app/types.rs src/tui/app/event.rs src/tui/client.rs
git commit -m "fix(tui): preserve active scoped agent view"
```

### Task 6: Render Breadcrumbs and Restore Main/Back Navigation

**Files:**
- Modify: `src/tui/components/subagent_focus_view.rs:294-442,520-622`
- Modify: `src/tui/app/event_key.rs:49-97`
- Modify: `src/tui/app/event.rs:688-715`
- Test: `src/tui/components/subagent_focus_view.rs`

- [ ] **Step 1: Write failing breadcrumb tests**

Add a pure breadcrumb formatter:

```rust
#[test]
fn breadcrumb_shows_traversed_hierarchy() {
    assert_eq!(
        breadcrumb_text(&["main".into(), "child".into(), "grandchild".into()]),
        "main > child > grandchild"
    );
}
```

Add state tests proving Backspace restores the previous frame selection and Enter on `Main` clears the back stack and restores the root frame.

- [ ] **Step 2: Run tests and verify RED**

Run:

```bash
cargo test tui::components::subagent_focus_view::tests::breadcrumb --lib
cargo test tui::app::event::tests::navigate_to_main --lib
```

Expected: tests fail because breadcrumb and main restoration are not implemented.

- [ ] **Step 3: Render the trusted navigation path**

Add breadcrumb text to the focus header using labels from root, back stack, and current frame:

```rust
pub fn breadcrumb_text(labels: &[String]) -> String {
    labels.join(" > ")
}
```

Truncate the rendered breadcrumb to the available header width using the existing `truncate` helper. Do not derive breadcrumbs from agent messages or arbitrary IDs outside the navigation stack.

- [ ] **Step 4: Implement main and back restoration**

- Backspace pops exactly one frame and restores its selection/conversation.
- Enter on `Main` restores the cached root frame and clears `back_stack`.
- Esc continues to close the focus view without mutating the cached hierarchy.
- Enter on the current self is a no-op.

Update help text to distinguish `Enter open/main`, `Backspace parent`, and `Esc close`.

- [ ] **Step 5: Run UI component tests and commit**

Run:

```bash
cargo test tui::components --lib
cargo test tui::app --lib
```

Expected: all tests pass.

```bash
git add src/tui/components/subagent_focus_view.rs src/tui/app/event_key.rs src/tui/app/event.rs
git commit -m "feat(tui): show scoped agent breadcrumbs"
```

### Task 7: Lock Down Integration, Documentation, and Verification

**Files:**
- Modify: `tests/strict_subagent_isolation.rs`
- Modify: `CHANGELOG.md`
- Verify: all modified Rust files

- [ ] **Step 1: Add the end-to-end hierarchy/chat regression contract**

Cover this sequence in `tests/strict_subagent_isolation.rs`:

1. root view contains root and direct child, not grandchild;
2. child navigation returns child self messages and direct grandchild;
3. grandchild navigation returns grandchild self messages;
4. each response excludes parent, sibling, and other-branch records;
5. invalid capability navigation returns the same not-visible result;
6. selected-agent message assertions contain only that agent's seeded marker.

Use unique message markers:

```rust
assert_eq!(child_view.self_view.messages, vec![ChatMessage::assistant("child-only")]);
assert_eq!(
    grandchild_view.self_view.messages,
    vec![ChatMessage::assistant("grandchild-only")]
);
assert!(!format!("{:?}", grandchild_view).contains("sibling-only"));
```

- [ ] **Step 2: Run the strict-isolation regression tests**

Run:

```bash
cargo test --test strict_subagent_isolation -- --nocapture
```

Expected: all strict isolation, recursive navigation, and transcript assertions pass.

- [ ] **Step 3: Update the changelog**

Under the current unreleased section, add:

```markdown
- Fixed scoped subagent focus views so each level shows `main`, the current
  agent, and its direct children, supports recursive navigation breadcrumbs,
  and displays the conversation of the selected agent without root polling
  overwriting the active child view.
```

- [ ] **Step 4: Run formatting**

Run:

```bash
cargo fmt
cargo fmt -- --check
```

Expected: both commands succeed with no formatting diff.

- [ ] **Step 5: Run Clippy with warnings denied**

Run:

```bash
cargo clippy --all-targets -- -D warnings
```

Expected: exit code 0 and no warnings.

- [ ] **Step 6: Run the complete test suite**

Run:

```bash
cargo test --all
```

Expected: all tests pass.

- [ ] **Step 7: Check scope and security invariants**

Run:

```bash
rg -n "parent_id|grandchild|descendants" src/daemon/models.rs src/daemon/handlers.rs src/tui/components/subagent_tree.rs
rg -n "capability = %|navigation_capability = %" src/tui src/daemon
git diff --check
```

Expected:

- no parent or arbitrary descendant field is added to `LocalAgentViewResponse`;
- no raw capability value is logged;
- `git diff --check` produces no output.

- [ ] **Step 8: Commit verification and changelog**

```bash
git add CHANGELOG.md tests/strict_subagent_isolation.rs
git commit -m "test(tui): cover scoped hierarchy conversations"
```
