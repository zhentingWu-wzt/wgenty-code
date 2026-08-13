# Production Work-Graph Runtime Wiring Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Register the static Work-Graph node tools in normal daemon and headless runs through isolated, trusted per-session runtime state.

**Architecture:** An `ExecutionSessionRuntimeStore` lazily creates a `NodeRuntime` per trusted `AgentExecutionContext.session_id`. Context-aware node-tool adapters resolve that runtime and ensure its first graph turn exists. Daemon and headless own one store per process and register the adapters in their existing shared `ToolRegistry`.

**Tech Stack:** Rust, Tokio, `Arc`, `std::sync::RwLock`, existing `ToolContext`, `SessionCoordinator`, `NodeRuntime`, `ToolRegistry`, daemon state, and headless runtime.

## Global Constraints

- Runtime lookup must use only `ToolContext.agent.session_id`; tool JSON never contains a session or agent identifier.
- Context-free graph-tool execution fails closed and must not allocate a session.
- The global registry may serve many sessions; it must not hold a single mutable `SessionCoordinator`.
- A runtime must be initialized exactly once per session, including under concurrent lookup.
- Preserve existing tool names and JSON schemas.
- `begin_node` must ensure one active graph turn before calling the existing NodeRuntime API.
- Run `cargo fmt -- --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test --all` before the final commit.

---

### Task 1: Create an isolated ExecutionSession runtime store

**Files:**

- Create: `src/exec_session/runtime_store.rs`
- Modify: `src/exec_session/mod.rs`
- Test: unit tests in `src/exec_session/runtime_store.rs`

**Interfaces:**

- `ExecutionSessionRuntimeStore::new(project_root: PathBuf, checkpoint_store: Arc<CheckpointStore>, auto_retry_max: u32) -> Self`.
- `runtime_for(&self, session_id: &SessionId) -> anyhow::Result<Arc<NodeRuntime>>` creates or returns exactly one runtime for the trusted id.
- `ensure_turn(&self, session_id: &SessionId) -> anyhow::Result<Arc<NodeRuntime>>` starts a `SessionCoordinator` turn only if no active turn exists.
- The private map stores a `RuntimeEntry { runtime: Arc<NodeRuntime>, coordinator: Arc<RwLock<SessionCoordinator>>, gate: Arc<VerifyGate> }` so the store, not `NodeRuntime`, owns construction and turn initialization.

- [ ] **Step 1: Write failing session-isolation and same-session concurrency tests**

```rust
#[tokio::test]
async fn runtime_store_reuses_one_runtime_per_session_and_isolates_other_sessions() {
    let store = test_store();
    let first = store.runtime_for(&SessionId::new("a")).unwrap();
    let repeated = store.runtime_for(&SessionId::new("a")).unwrap();
    let other = store.runtime_for(&SessionId::new("b")).unwrap();
    assert!(Arc::ptr_eq(&first, &repeated));
    assert!(!Arc::ptr_eq(&first, &other));
}

#[tokio::test]
async fn ensure_turn_is_idempotent_for_one_session() {
    let store = test_store();
    store.ensure_turn(&SessionId::new("a")).unwrap();
    store.ensure_turn(&SessionId::new("a")).unwrap();
    assert_eq!(store.turn_count_for_test(&SessionId::new("a")), 1);
}
```

- [ ] **Step 2: Run the focused tests and observe the missing store**

Run: `cargo test exec_session::runtime_store::tests --lib`

Expected: compilation fails because the store does not exist.

- [ ] **Step 3: Implement minimal lazy runtime construction**

Construct `SessionCoordinator` using `SessionSource::AgentSelf`, then `VerifyGate` and `NodeRuntime::new_with_default_hooks` with `ProcessCommandExecutor`. Protect map lookup/insertion with one mutex; hold it only through construction and never while executing graph commands. Add a test-only `turn_count_for_test` helper rather than widening `NodeRuntime`'s API.

- [ ] **Step 4: Verify Task 1**

Run: `cargo test exec_session::runtime_store::tests --lib && cargo fmt -- --check`

Expected: store tests and formatting pass.

- [ ] **Step 5: Commit Task 1**

```bash
git add src/exec_session/runtime_store.rs src/exec_session/mod.rs
git commit -m "feat(exec-session): add per-session runtime store"
```

### Task 2: Convert node tools to trusted-context adapters

**Files:**

- Modify: `src/exec_session/node_tools.rs`
- Modify: `src/exec_session/verify_gate.rs`
- Test: unit tests in `src/exec_session/node_tools.rs`

**Interfaces:**

- `BeginNodeTool`, `VerifyNodeTool`, and `RollbackNodeTool` hold `Arc<ExecutionSessionRuntimeStore>`.
- `execute_with_context` resolves the runtime from `context.agent.session_id`; `BeginNodeTool` calls `ensure_turn` before beginning the node.
- `execute` returns `ToolError { code: Some("missing_tool_context") }` for every graph lifecycle tool.
- `VerifyAndCompleteTool` resolves the stored `VerifyGate` through `ExecutionSessionRuntimeStore::gate_for`; it uses the same trusted session selection rule.

- [ ] **Step 1: Write failing contextual-node-tool tests**

```rust
#[tokio::test]
async fn contextual_begin_and_verify_run_one_session_graph() {
    let setup = ToolSetup::with_store("session-a");
    let begin = setup.registry.execute_with_context(&setup.context, "begin_node", begin_json()).await.unwrap();
    let verified = setup.registry.execute_with_context(&setup.context, "verify_node", json!({})).await.unwrap();
    assert!(begin.content.contains("node_id"));
    assert!(verified.metadata.contains_key("next_step"));
}

#[tokio::test]
async fn context_free_begin_node_is_rejected_without_creating_a_runtime() {
    let setup = ToolSetup::with_store("session-a");
    assert_eq!(setup.begin.execute(begin_json()).await.unwrap_err().code.as_deref(), Some("missing_tool_context"));
    assert_eq!(setup.store.len(), 0);
}
```

- [ ] **Step 2: Run the focused test and observe the old per-runtime constructor fails the contract**

Run: `cargo test exec_session::node_tools::tests::contextual_begin_and_verify_run_one_session_graph --lib`

Expected: fail until node tools use the store/context.

- [ ] **Step 3: Implement adapters without changing schemas**

Extract existing input parsing and output formatting into helpers called by the contextual paths. Convert the legacy verification-completion tool to the same store resolution model, then leave the command executor and verification policy unchanged. Do not permit model input to override session identity.

- [ ] **Step 4: Verify Task 2**

Run: `cargo test exec_session::node_tools::tests --lib && cargo test exec_session::verify_gate::tests --lib && cargo clippy --all-targets -- -D warnings`

Expected: all node/verify gate tests and lint pass.

- [ ] **Step 5: Commit Task 2**

```bash
git add src/exec_session/node_tools.rs src/exec_session/verify_gate.rs
git commit -m "feat(exec-session): scope node tools to trusted sessions"
```

### Task 3: Register the store in daemon and headless startup

**Files:**

- Modify: `src/daemon/state.rs`
- Modify: `src/cli/headless_runtime.rs`
- Modify: `src/tools/mod.rs`
- Test: unit tests in `src/daemon/state.rs`, `src/daemon/handlers.rs`, and `src/cli/headless_runtime.rs`

**Interfaces:**

- `ToolRegistry::register_exec_session_tools(store: Arc<ExecutionSessionRuntimeStore>)` registers the graph lifecycle tools once in a global registry.
- `DaemonState` owns one `Arc<ExecutionSessionRuntimeStore>` built from its project root/checkpoint store and registers graph tools in `Arc::new_cyclic`.
- Headless creates a matching store before `RegistryToolPort` and exposes the same tool definitions.

- [ ] **Step 1: Write failing production-registration tests**

```rust
#[tokio::test]
async fn daemon_registers_contextual_work_graph_tools() {
    let state = Arc::new(DaemonState::new(test_app_state()).await);
    for name in ["begin_node", "verify_node", "rollback_node", "verify_and_complete"] {
        assert!(state.tool_registry.get(name).is_some(), "missing {name}");
    }
}

#[test]
fn headless_registry_exposes_work_graph_tools() {
    let registry = build_headless_registry(test_settings());
    assert!(registry.get("begin_node").is_some());
}
```

- [ ] **Step 2: Run the focused tests and observe absent production registration**

Run: `cargo test daemon::handlers::tests::daemon_registers_contextual_work_graph_tools --lib && cargo test cli::headless_runtime::tests::headless_registry_exposes_work_graph_tools --lib`

Expected: fail because no production path currently calls the graph registration helper.

- [ ] **Step 3: Wire stores into both entry points**

Create exactly one daemon store after checkpoint dependencies are available and pass it into registry construction. Factor headless registry construction into a testable helper if it does not already exist. Keep per-session runtime allocation lazy so startup work and memory remain bounded.

- [ ] **Step 4: Verify Task 3**

Run: `cargo test daemon::handlers::tests --lib && cargo test cli::headless_runtime::tests --lib && cargo fmt -- --check`

Expected: production construction tests and formatting pass.

- [ ] **Step 5: Commit Task 3**

```bash
git add src/daemon/state.rs src/daemon/handlers.rs src/cli/headless_runtime.rs src/tools/mod.rs
git commit -m "feat(graph): wire work graph runtime into entry points"
```

### Task 4: Verify real tool-path isolation and update architecture docs

**Files:**

- Modify: `WGENTY.md`
- Modify: `docs/superpowers/specs/2026-08-12-work-graph-runtime-wiring-design.md`
- Modify: `docs/superpowers/plans/2026-08-12-work-graph-runtime-wiring.md`
- Test: existing integration and library suites

- [ ] **Step 1: Add an end-to-end daemon tool-path test**

Use the real handler/tool executor with two trusted root contexts. Begin a node in each and assert their persisted sessions use separate snapshot directories and neither can observe the other node id or graph audit events.

- [ ] **Step 2: Run the test before final wiring**

Run: `cargo test daemon::handlers::tests::work_graph_sessions_are_isolated --lib`

Expected: fail until the store-backed production registration is complete.

- [ ] **Step 3: Document operational behavior**

Explain that Work-Graph tools bind to the trusted agent session and create a graph turn lazily on `begin_node`; external anchors remain code-owned. Include the runtime store in the graph architecture section.

- [ ] **Step 4: Run the complete quality gate**

Run: `cargo fmt -- --check && cargo clippy --all-targets -- -D warnings && cargo test --all`

Expected: all commands exit 0; report fresh pass/ignore counts.

- [ ] **Step 5: Commit Task 4**

```bash
git add WGENTY.md docs/superpowers/specs/2026-08-12-work-graph-runtime-wiring-design.md docs/superpowers/plans/2026-08-12-work-graph-runtime-wiring.md src
git commit -m "docs(graph): document production runtime wiring"
```
