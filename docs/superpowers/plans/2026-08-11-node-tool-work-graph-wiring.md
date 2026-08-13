# Node Tool Work-Graph Wiring Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox syntax for tracking.

**Goal:** Route existing begin_node and verify_node tool calls through the static Work-Graph when the node declares compile or test anchors.

**Architecture:** Persist optional compile_commands and test_commands on NodeContract with serde defaults. BeginNodeTool parses and stores the optional arrays. VerifyNodeTool asks NodeRuntime to select the legacy verify path for empty arrays or the complete static work graph for declared anchors, returning structured status and next-step data.

**Tech Stack:** Rust, Tokio, Serde, existing exec_session tools, Cargo tests.

## Global Constraints

- Preserve NodeContract JSON backward compatibility with serde defaults.
- Use code-owned WorkGraphStep routing; do not parse agent text.
- Do not hold coordinator locks across asynchronous command execution.
- Maintain cargo fmt and clippy zero-warning requirements.

---

### Task 1: Persist optional compile and test command arrays

**Files:**

- Modify: src/exec_session/node.rs
- Modify: src/exec_session/node_runtime.rs
- Test: unit tests in src/exec_session/node.rs
- Test: tests/integration/exec_session_node_lifecycle.rs

**Interfaces:**

- NodeContract gains serde-defaulted compile_commands: Vec<String> and test_commands: Vec<String>.
- NodeRuntime gains begin_node_with_anchors(goal, compile_commands, test_commands, verify_commands, expected_files) -> Result<NodeId>.
- The existing begin_node signature delegates with empty compile/test arrays.

- [ ] **Step 1: Write a failing legacy-deserialization test**

```rust
#[test]
fn node_contract_missing_anchor_arrays_defaults_to_empty() {
    let contract: NodeContract = serde_json::from_str(
        r#"{"goal":"legacy","verify_commands":["cargo test"],"expected_files":[]}"#,
    ).unwrap();
    assert!(contract.compile_commands.is_empty());
    assert!(contract.test_commands.is_empty());
}
```

- [ ] **Step 2: Run the test and observe the missing fields**

Run: cargo test exec_session::node::tests::node_contract_missing_anchor_arrays_defaults_to_empty --lib

Expected: FAIL because NodeContract has no compile_commands or test_commands fields.

- [ ] **Step 3: Implement the backward-compatible fields and constructor**

Add the two fields with #[serde(default)], update all NodeContract constructors, and make existing begin_node call the new anchored constructor with empty vectors.

- [ ] **Step 4: Add and run an integration persistence test**

```rust
#[tokio::test]
async fn node_contract_persists_compile_and_test_commands() {
    let node_id = setup.runtime.begin_node_with_anchors(
        "goal".into(), vec!["cargo check".into()], vec!["cargo test".into()],
        vec!["cargo test --doc".into()], vec![],
    ).await.unwrap();
    assert_eq!(reloaded_node(&setup, &node_id).contract.compile_commands, vec!["cargo check"]);
}
```

Run: cargo test --test exec_session_node_lifecycle node_contract_persists_compile_and_test_commands

Expected: PASS.

### Task 2: Parse anchors in begin_node and route verify_node

**Files:**

- Modify: src/exec_session/node_tools.rs
- Modify: src/exec_session/node_runtime.rs
- Test: unit tests in src/exec_session/node_tools.rs
- Test: unit tests in src/exec_session/node_runtime.rs

**Interfaces:**

- BeginNodeTool accepts optional compile_commands and test_commands string arrays.
- NodeRuntime gains verify_current_node() -> Result<NodeVerificationOutcome>.
- NodeVerificationOutcome distinguishes Legacy(NodeVerifyResult) from WorkGraph(WorkGraphRunResult).
- VerifyNodeTool emits graph next_step in content and metadata for the anchored path.

- [ ] **Step 1: Write a failing BeginNodeTool schema and parsing test**

```rust
#[tokio::test]
async fn begin_node_tool_persists_optional_anchor_commands() {
    let output = tool.execute(json!({
        "goal":"goal", "compile_commands":["cargo check"],
        "test_commands":["cargo test"], "verify_commands":["cargo test --doc"]
    })).await.unwrap();
    assert_eq!(persisted_node(&output).contract.test_commands, vec!["cargo test"]);
}
```

- [ ] **Step 2: Run the test and observe that the optional arrays are ignored**

Run: cargo test exec_session::node_tools::tests::begin_node_tool_persists_optional_anchor_commands --lib

Expected: FAIL because the schema/parser does not expose the arrays.

- [ ] **Step 3: Implement optional-array parsing and anchored node creation**

Reuse a local optional-string-array parser that rejects non-string array entries. Update the tool description and JSON schema. Do not make either new field required.

- [ ] **Step 4: Write a failing runtime-routing test**

```rust
#[tokio::test]
async fn verify_current_node_runs_work_graph_when_anchors_are_declared() {
    setup.runtime.begin_node_with_anchors(
        "goal".into(), vec!["compile".into()], vec!["test".into()],
        vec!["verify".into()], vec![],
    ).await.unwrap();
    let result = setup.runtime.verify_current_node().await.unwrap();
    assert!(matches!(result, NodeVerificationOutcome::WorkGraph(_)));
    assert_eq!(scripted_executor_call_count(), 3);
}
```

- [ ] **Step 5: Run the test and observe the missing routing entry point**

Run: cargo test exec_session::node_runtime::tests::verify_current_node_runs_work_graph_when_anchors_are_declared --lib

Expected: FAIL with no method named verify_current_node.

- [ ] **Step 6: Implement NodeRuntime and VerifyNodeTool routing**

Clone the current node contract under a read lock, release the lock, then choose verify_node for two empty arrays or run_work_graph otherwise. Serialize legacy and graph outputs without changing existing legacy fields.

- [ ] **Step 7: Verify focused tests and formatting**

Run: cargo test exec_session::node_tools::tests --lib && cargo test exec_session::node_runtime::tests --lib && cargo fmt -- --check

Expected: all tests pass.

### Task 3: Verify end-to-end compatibility and commit

**Files:**

- Modify: tests/integration/exec_session_node_lifecycle.rs
- Modify: docs/superpowers/plans/2026-08-11-node-tool-work-graph-wiring.md

- [ ] **Step 1: Add a failing compatibility test**

```rust
#[tokio::test]
async fn verify_current_node_without_anchors_uses_legacy_verify_path() {
    setup.runtime.begin_node("goal".into(), vec!["verify".into()], vec![]).await.unwrap();
    assert!(matches!(setup.runtime.verify_current_node().await.unwrap(), NodeVerificationOutcome::Legacy(_)));
}
```

- [ ] **Step 2: Run it, implement any minimal compatibility correction, and re-run**

Run: cargo test --test exec_session_node_lifecycle verify_current_node_without_anchors_uses_legacy_verify_path

Expected: PASS.

- [ ] **Step 3: Run full verification**

Run: cargo fmt -- --check && cargo clippy --all-targets -- -D warnings && cargo test --all

Expected: all commands exit 0.

- [ ] **Step 4: Commit**

```bash
git add src/exec_session/node.rs src/exec_session/node_runtime.rs src/exec_session/node_tools.rs tests/integration/exec_session_node_lifecycle.rs docs/superpowers/plans/2026-08-11-node-tool-work-graph-wiring.md
git commit -m "feat(graph): wire node tools to static work graph"
```

