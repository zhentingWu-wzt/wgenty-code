# Task 8: 端到端测试 + 解耦不变式验证

## Scope
- Create `tests/integration/exec_session_e2e.rs` covering the full closed loop (turn chain + git refs + checkpoint capture + verify-gate + rollback + unverified fallback) plus the two cross-cutting invariants (decoupling, crash consistency).
- Register the new module in the consolidated integration binary (`tests/integration/main.rs`).
- Verify existing CheckpointStore / undo tests unaffected.
- `cargo test --all` + `cargo clippy --all-targets -- -D warnings` + `cargo fmt --check` green.

## Approach
Added the E2E module under `tests/integration/` (consolidated binary convention from `tests/integration/main.rs`) rather than a standalone `tests/exec_session_e2e.rs`. Rationale: the consolidated binary eliminates a redundant link pass (the convention this repo adopted); the plan's standalone path predates that consolidation. The module is registered with one `mod` line.

The unit tests in `src/exec_session/{coordinator,verify_gate,session}.rs` and the agent-loop tests in `src/agent/runtime/loop_tests.rs` (Task 7) cover each stage in isolation. Task 8's value is realistic multi-stage flows over a real git repo, combining subsystems in one test:

- **8.1** `full_closed_loop_three_turns_verify_pass` — 3 turns (file create via tombstone capture + write), verify pass, status=Completed, parent chain intact, verify_log 1 attempt completed.
- **8.2** `verify_fail_then_retry_passes` — `false` (exit 1) -> CommandFailed + AutoRetry{remaining:2}, status stays InProgress; `true` (exit 0) -> Completed. verify_log 2 attempts (failed, completed).
- **8.3** `verify_out_of_scope_then_adjusted_passes` — create c.txt outside expected set -> BoundaryViolation{unexpected_files=[c.txt]}; agent declares c.txt -> pass.
- **8.4** `rollback_restores_workspace_full` — turn 0 edit seed.txt (Saved), turn 1 commit (HEAD advances) + create untracked.txt; rollback_to(turn-0): git reset --hard sha0 + rewind seed.txt + delete untracked.txt; current_turn=turn-0.
- **8.5** `agent_skips_verify_fallback_unverified` — session ends InProgress -> mark_unverified_if_incomplete -> Unverified; idempotent (AlreadyTerminal on 2nd call); verify_log final_status=unverified.
- **8.6** `non_git_project_degraded_verify_and_rollback` — no git: git_refs=None, verify actual from source 1 only; rollback: no git reset, rewind deletes tombstone file.
- **8.7** `crash_consistency_stale_tmp_does_not_corrupt_load` + `crash_consistency_missing_session_json_with_only_tmp_errors` — stale `session.json.tmp` never corrupts load (committed session.json is sole source of truth); missing session.json + only tmp -> load errors (no half-read).
- **8.8** `exec_session_source_has_no_comet_dependency` — scans `src/exec_session/*.rs`, strips `//` comments + string literals (honoring `\"` escapes), asserts no lowercase `comet` in remaining code. `Comet` (PascalCase enum variant) and `"comet"` (kebab-case serde wire form) are the only allowed occurrences; both are caller-declared labels, not core-runtime branching.

## Decoupling invariant check (8.8)
The test enforces the spec §6 invariant at runtime (runs on every `cargo test`). Logic:
1. For each `.rs` file in `src/exec_session/`, process line by line.
2. Strip `//` line comments (safe: no `//` inside string literals in these files).
3. Strip Rust string literals (honoring `\"` escapes) from the code portion.
4. Assert the remaining code contains no lowercase `comet` substring. PascalCase `Comet` (the enum variant) does not match; the `"comet"` serde literal is already stripped.

Current state: zero violations. Allowed occurrences:
- `session.rs`: `SessionSource::Comet` enum variant + `"comet"` serde test assertion.
- `hooks.rs:68`, `mod.rs:11-12`: doc comments mentioning comet/plan (stripped as comments).

## Verification
- `cargo test --test integration exec_session_e2e` -> **9 passed**.
- `cargo test --test integration` -> **164 passed, 0 failed** (includes the 9 new tests).
- `cargo test --lib checkpoint` -> 21 passed; `cargo test --lib undo` -> 3 passed (CheckpointStore / undo unaffected).
- `cargo test --all` -> 1049 lib passed + 164 integration passed; 2 failed (`services::auto_dream`, pre-existing, unrelated: seatbelt "Operation not permitted" on `~/.wgenty-code/memory/`; `auto_dream.rs` not in diff).
- `cargo clippy --all-targets -- -D warnings` -> zero warnings.
- `cargo fmt --check` -> clean.

## Files
- **Create**: `tests/integration/exec_session_e2e.rs` (~370 lines, 9 tests).
- **Modify**: `tests/integration/main.rs` (+1 line: `mod exec_session_e2e;`).

## Invariants upheld
- Decoupling: `src/exec_session/` has no comet dependency (enforced by test 8.8).
- Crash consistency: stale tmp never corrupts load (enforced by tests 8.7).
- CheckpointStore / undo behavior unchanged (21 + 3 tests pass).
- All 8 plan scenarios (8.1-8.8) covered; 8.9 (existing tests unaffected) verified; 8.10 (full suite green) verified modulo pre-existing auto_dream env failures.
