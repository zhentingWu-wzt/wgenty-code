# Rust Verification Profile Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every new execution-session node in a Rust project use code-owned compile, test, and final verification anchors.

**Architecture:** Add a focused `VerificationProfile` resolver in `exec_session`. At node creation it detects `Cargo.toml` at the coordinator project root and resolves immutable Rust defaults before persisting `NodeContract`. Model commands become supplemental final verification only; missing profile fields in old sessions resolve to `none`.

**Tech Stack:** Rust, Tokio, Serde, tempfile, ExecutionSession Work-Graph and node tools.

## Global Constraints

- New Rust nodes run `cargo check`, `cargo test --all`, then `cargo clippy --all-targets -- -D warnings` before supplemental commands.
- Profile selection and WorkGraph routing are code-owned; no model report is an acceptance signal.
- Old `NodeContract` JSON without a profile field continues to deserialize and route as before.
- Do not hold coordinator locks during command execution.
- Full validation is `cargo fmt -- --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test --all`.

---

### Task 1: Add deterministic profile resolution and persisted profile state

**Files:**

- Create: `src/exec_session/verification_profile.rs`
- Modify: `src/exec_session/mod.rs`
- Modify: `src/exec_session/node.rs`
- Test: unit tests in `src/exec_session/verification_profile.rs` and `src/exec_session/node.rs`

**Interfaces:**

- `VerificationProfile::{None, Rust}` serializes as snake-case and defaults to `None`.
- `VerificationProfile::detect(project_root: &Path) -> Self` returns Rust only when `Cargo.toml` is a file.
- `VerificationProfile::resolve(self, compile_commands, test_commands, verify_commands) -> ResolvedVerificationCommands` prepends mandatory commands and exact-deduplicates strings in first-seen order.
- `NodeContract` gains `#[serde(default)] verification_profile: VerificationProfile`.

- [ ] **Step 1: Write failing resolver tests**

```rust
#[test]
fn rust_profile_prepends_required_anchors_and_deduplicates_final_commands() {
    let resolved = VerificationProfile::Rust.resolve(
        vec!["custom compile".into()], vec!["custom test".into()],
        vec!["cargo clippy --all-targets -- -D warnings".into(), "cargo test --doc".into()],
    );
    assert_eq!(resolved.compile_commands, ["cargo check", "custom compile"]);
    assert_eq!(resolved.test_commands, ["cargo test --all", "custom test"]);
    assert_eq!(resolved.verify_commands, [
        "cargo clippy --all-targets -- -D warnings", "cargo test --doc",
    ]);
}

#[test]
fn detect_rust_only_when_manifest_exists() {
    let dir = tempfile::tempdir().expect("temporary project");
    assert_eq!(VerificationProfile::detect(dir.path()), VerificationProfile::None);
    std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"p\"\n")
        .expect("write manifest");
    assert_eq!(VerificationProfile::detect(dir.path()), VerificationProfile::Rust);
}
```

- [ ] **Step 2: Run and observe expected missing-type failure**

Run: `cargo test exec_session::verification_profile::tests --lib`

Expected: fails to compile because `verification_profile` does not exist.

- [ ] **Step 3: Implement profile types, resolver, exports, and contract field**

Use a focused value object for the three resolved command vectors. Use a small helper with `HashSet<&str>` only to remove exact duplicate strings, retaining vector order. Add the serde-defaulted profile to every `exec_session::NodeContract` constructor, plus round-trip and legacy JSON tests.

- [ ] **Step 4: Verify Task 1**

Run: `cargo test exec_session::verification_profile::tests --lib && cargo test exec_session::node::tests --lib && cargo fmt -- --check`

Expected: all targeted tests and formatting pass.

### Task 2: Resolve the profile centrally at NodeRuntime creation

**Files:**

- Modify: `src/exec_session/node_runtime.rs`
- Test: unit tests in `src/exec_session/node_runtime.rs`

**Interfaces:**

- `NodeRuntime::begin_node_with_anchors(...) -> Result<NodeId>` detects, resolves, and persists the profile and command lists.
- Existing `NodeRuntime::begin_node(...)` remains source-compatible and delegates to the same path.

- [ ] **Step 1: Write a failing Rust-project node-creation test**

Extend `TestSetup` with a helper that writes `Cargo.toml`. Add:

```rust
#[tokio::test]
async fn begin_node_in_rust_project_persists_code_owned_profile_commands() {
    let setup = TestSetup::new(0);
    setup.write_cargo_manifest();
    setup.begin_turn();
    setup.runtime.begin_node_with_anchors(
        "goal".into(), vec![], vec![], vec!["cargo test --doc".into()], vec![],
    ).await.expect("begin node");
    let contract = &setup.coord.read().expect("coordinator").current_node()
        .expect("node").contract;
    assert_eq!(contract.verification_profile, VerificationProfile::Rust);
    assert_eq!(contract.compile_commands, ["cargo check"]);
    assert_eq!(contract.test_commands, ["cargo test --all"]);
    assert_eq!(contract.verify_commands, [
        "cargo clippy --all-targets -- -D warnings", "cargo test --doc",
    ]);
}
```

- [ ] **Step 2: Run and observe that empty anchors persist**

Run: `cargo test exec_session::node_runtime::tests::begin_node_in_rust_project_persists_code_owned_profile_commands --lib`

Expected: fails because no profile command injection exists.

- [ ] **Step 3: Implement central resolution**

Copy the project root under a short read lock, detect and resolve commands, then acquire the existing write lock for the node-state precondition and persistence. Do not put profile enforcement in `BeginNodeTool`; direct runtime callers must have identical protection.

- [ ] **Step 4: Add non-Rust compatibility test and verify runtime tests**

```rust
#[tokio::test]
async fn begin_node_in_non_rust_project_preserves_declared_anchors() {
    let setup = TestSetup::new(0);
    setup.begin_turn();
    setup.runtime.begin_node_with_anchors(
        "goal".into(), vec!["compile".into()], vec!["test".into()],
        vec!["verify".into()], vec![],
    ).await.expect("begin node");
    let contract = &setup.coord.read().expect("coordinator").current_node()
        .expect("node").contract;
    assert_eq!(contract.verification_profile, VerificationProfile::None);
    assert_eq!(contract.compile_commands, ["compile"]);
}
```

Run: `cargo test exec_session::node_runtime::tests --lib`

Expected: all runtime tests pass.

### Task 3: Restrict model-selected anchors and prove tool-path execution order

**Files:**

- Modify: `src/exec_session/node_tools.rs`
- Test: unit tests in `src/exec_session/node_tools.rs`
- Modify: `docs/superpowers/plans/2026-08-11-rust-verification-profile.md`

**Interfaces:**

- The BeginNodeTool schema no longer exposes `compile_commands` or `test_commands`.
- BeginNodeTool passes empty compile/test command lists to `NodeRuntime`; `verify_commands` are supplemental final verification.
- In a Rust temp project, VerifyNodeTool executes `[cargo check, cargo test --all, cargo clippy --all-targets -- -D warnings, supplemental]` through the injected executor.

- [ ] **Step 1: Write failing tool schema and end-to-end order test**

```rust
#[tokio::test]
async fn node_tools_use_rust_profile_anchors_in_order() {
    let directory = TempDir::new().expect("temporary project");
    std::fs::write(directory.path().join("Cargo.toml"), "[package]\nname = \"p\"\n")
        .expect("write manifest");
    // Build the existing coordinator, RecordingExecutor, runtime, and tools.
    begin.execute(json!({
        "goal": "tool e2e", "verify_commands": ["cargo test --doc"],
        "expected_files": []
    })).await.expect("begin node");
    verify.execute(json!({})).await.expect("verify node");
    assert_eq!(calls.lock().expect("calls").as_slice(), [
        "cargo check", "cargo test --all",
        "cargo clippy --all-targets -- -D warnings", "cargo test --doc",
    ]);
}
```

- [ ] **Step 2: Run and observe missing Rust profile commands**

Run: `cargo test exec_session::node_tools::tests::node_tools_use_rust_profile_anchors_in_order --lib`

Expected: fails before profile wiring exists.

- [ ] **Step 3: Update tool contract and route only supplemental final commands**

Remove compile/test properties and parser calls. Retain required `verify_commands` for compatibility, describe it as supplemental final verification, and call `begin_node_with_anchors(goal, Vec::new(), Vec::new(), verify_commands, expected_files)`.

- [ ] **Step 4: Run focused and full validation**

Run: `cargo test exec_session::node_tools::tests --lib && cargo fmt -- --check && cargo clippy --all-targets -- -D warnings && cargo test --all`

Expected: all commands exit 0.

- [ ] **Step 5: Commit the implementation**

Run `git add src/exec_session/verification_profile.rs src/exec_session/mod.rs src/exec_session/node.rs src/exec_session/node_runtime.rs src/exec_session/node_tools.rs docs/superpowers/plans/2026-08-11-rust-verification-profile.md` followed by `git commit -m "feat(graph): enforce rust verification profiles"`.
