---
change: exec-session-node-state-machine
design-doc: openspec/changes/exec-session-node-state-machine/design.md
base-ref: b0af032e740d99a189921ac7c89343d385bf5030
archived-with: 2026-07-20-exec-session-node-state-machine
---

# Implementation Plan: ExecutionSession Outer Layer -- Node State Machine

## Context

**Discovery**: The outer-layer implementation is **already complete** (3 prior commits, ~900 lines across `node.rs`, `node_runtime.rs`, `node_tools.rs`). It compiles cleanly, passes clippy, and includes 12 unit tests (5 serialization + 7 lifecycle). However:

1. **Unit tests cannot run** -- a pre-existing compile error in `src/agent/runtime/loop_tests.rs:641` (missing `system_messages` field in `RunLoopArgs`) blocks `cargo test --lib`.
2. **No integration tests** -- the spec defines 17 scenarios; 0 are covered by integration tests.
3. **No end-to-end verification** of the complete node lifecycle (begin -> work -> verify -> retry -> rollback).

This plan fills those gaps rather than implementing from scratch.

## Pre-Existing Implementation Audit

Already done (verified by code review + compile + clippy):

- `src/exec_session/node.rs` (170 lines): `NodeContract`, `NodeStatus`, `Node`, `NodeStates` + 5 serialization unit tests
- `src/exec_session/node_runtime.rs` (518 lines): `NodeRuntime` with `begin_node`/`verify_node`/`rollback_node` + 7 unit tests (create, reject-when-not-verified, verify-success, verify-failure-within-retry, rollback-to-verified, rollback-error-without-verified, begin-after-verified)
- `src/exec_session/node_tools.rs` (218 lines): `BeginNodeTool`, `VerifyNodeTool`, `RollbackNodeTool` implementing `Tool` trait with `is_read_only() = false`
- `src/tools/mod.rs`: `register_exec_session_tools` wires all 3 node tools + `VerifyGate`
- `src/config/agent.rs`: `ExecSessionSettings.auto_retry_max` (default 2) + config tests
- `src/exec_session/hooks.rs`: `pre_node`/`post_node` no-op stubs already present

archived-with: 2026-07-20-exec-session-node-state-machine
---

## Task 1: Fix pre-existing `loop_tests.rs:641` compile error

**Goal**: Unblock `cargo test --lib` so all existing unit tests (including node layer's 12 tests) can run.

**Problem**: `src/agent/runtime/loop_tests.rs:641` constructs `RunLoopArgs` without the `system_messages` field that was added to the struct. This is a pre-existing issue unrelated to the node work, but it blocks all `--lib` tests.

**Steps**:
1. Read `RunLoopArgs` struct definition to identify the exact missing field(s)
2. Read `loop_tests.rs:641` call site to see what's being constructed
3. Add the missing field(s) with appropriate default value (likely `Vec::new()` or `None`)
4. Verify: `cargo test --lib --no-run` compiles successfully

**Verification**:
- `cargo check --lib` passes
- `cargo test --lib --no-run` compiles (tests can now be discovered)

archived-with: 2026-07-20-exec-session-node-state-machine
---

## Task 2: Run and verify all existing unit tests pass

**Goal**: Confirm the pre-existing implementation (12 node tests + inner L1 tests + config tests) all pass once unblocked.

**Steps**:
1. Run `cargo test --lib exec_session` -- verifies node + inner layer unit tests
2. Run `cargo test --lib config` -- verifies `auto_retry_max` config tests
3. Review any failures against the design doc acceptance criteria (§11)

**Verification**:
- All `exec_session` unit tests pass (node.rs: 5 serialization + node_runtime.rs: 7 lifecycle)
- `auto_retry_max` config tests pass (defaults to 2, serde default when omitted)

archived-with: 2026-07-20-exec-session-node-state-machine
---

## Task 3: Write integration tests covering the 17 spec scenarios

**Goal**: Create `tests/integration/exec_session_node_lifecycle.rs` with integration tests that exercise the complete node state machine through `NodeRuntime` (not through the tool layer -- that's covered by unit tests in `node_tools.rs`).

**Test file**: `tests/integration/exec_session_node_lifecycle.rs`

**Test groups** (mapped to spec requirements):

### 3a: Node contract schema (2 scenarios)
- `node_contract_persisted_across_turns`: begin_node in turn-0, advance to turn-1, verify node still in `running` status in session.json
- `node_contract_without_expected_files`: begin_node with empty expected_files, verify_node skips boundary check

### 3b: Node state machine (4 scenarios)
- `node_transitions_to_verified_on_success`: begin_node -> verify_node (exit 0) -> status=verified, session.json updated
- `node_transitions_to_failed_on_failure`: begin_node -> verify_node (exit 1) -> status=failed, workspace preserved, failure reason returned
- `failed_node_self_correction_within_retry`: begin_node -> verify (fail, retry=1) -> verify (fail, retry=2) -> verify (pass) -> verified
- `failed_node_exceeds_retry_limit`: begin_node -> verify (fail x3, auto_retry_max=2) -> session.status=failed

### 3c: Node-level verify-gate (2 scenarios)
- `verify_node_delegates_to_inner_verify_gate`: verify_node executes commands via VerifyGate (mock executor), guardian/sandbox path verified
- `out_of_bounds_detection_scoped_to_node_span`: begin_node with expected_files, make changes outside expected, verify_node fails with boundary violation

### 3d: Node rollback (3 scenarios)
- `rollback_to_last_verified_node`: n1(verified) + n2(running) -> rollback_node -> workspace restored, n2 removed, current=n1
- `rollback_without_verified_node`: rollback_node when no verified node -> error, no workspace changes
- `rollback_preserves_verified_node_state`: rollback -> target verified node remains verified, node_states reflects removal

### 3e: Decoupling (2 scenarios)
- `runtime_code_has_no_orchestration_skill_references`: grep `src/exec_session/` for "comet" (excluding SessionSource::Comet enum variant + comments) -- automated via build script or test
- `verify_failure_returned_to_agent`: verify_node failure returns NodeVerifyResult with failure_reason, no orchestration-skill API called

### 3f: Tool registration (2 scenarios)
- `node_tools_available_when_enabled`: ExecSessionSettings.enabled=true -> 3 tools registered in ToolRegistry
- `node_tools_absent_when_disabled`: ExecSessionSettings.enabled=false -> 0 node tools registered

### 3g: Full lifecycle e2e (2 scenarios)
- `full_lifecycle_begin_verify_retry_rollback`: begin_node(n1) -> verify(pass) -> begin_node(n2) -> verify(fail) -> retry(pass) -> begin_node(n3) -> verify(fail x3) -> rollback_node -> current=n2(verified)
- `hooks_pre_post_node_called`: custom SessionHooks impl records pre_node/post_node calls through a full lifecycle

**Verification**:
- `cargo test --test integration exec_session_node` -- all new tests pass
- Each test maps to a spec scenario (traceable)

archived-with: 2026-07-20-exec-session-node-state-machine
---

## Task 4: Full verification suite

**Goal**: Run the complete CI-equivalent verification before marking build complete.

**Steps**:
1. `cargo fmt --check` -- formatting clean
2. `cargo clippy --all-targets -- -D warnings` -- zero warnings (note: may still hit pre-existing loop_tests issue if Task 1 didn't fully fix; fix any remaining)
3. `cargo test --all` -- all tests pass (unit + integration)
4. Verify decoupling invariant: `grep -rn "comet" src/exec_session/ | grep -v "SessionSource::Comet" | grep -v "//" | grep -v "//!"` returns empty

**Verification**:
- fmt: clean
- clippy: zero warnings
- tests: all pass
- decoupling: no "comet" references in exec_session source (except enum variant + comments)

archived-with: 2026-07-20-exec-session-node-state-machine
---

## Task 5: Update tasks.md and commit

**Goal**: Record task completion in `tasks.md` and commit.

**Steps**:
1. Update `openspec/changes/exec-session-node-state-machine/tasks.md` with checkmarks for all completed tasks
2. Commit with conventional commit message: `test(exec-session): add node lifecycle integration tests + fix loop_tests compile`

**Verification**:
- tasks.md updated
- git commit created
- `git status` clean
