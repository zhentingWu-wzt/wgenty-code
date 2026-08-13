# Final Fix Report: Work-Graph State Isolation

## Finding resolved

`WorkState` is persisted for the active turn, but compile, test, verify, generated
diff, and retry-budget values are node/pass scoped. Previously a new work-graph
pass overwrote only `compile_result`; `next_step` could therefore route through a
stale `test_result` or `verify_result`. A second Rust node in the same turn also
inherited the first node's successful anchors and could be marked verified after
running only `cargo check`.

The fix adds two coordinator-owned lifecycle resets:

- New node: clear generated diff, compile, test, verify, and retry budget while
  retaining the turn requirement and audit log. Persist the reset beside the
  active checkpoint after the node is added.
- New work-graph pass/retry: clear generated diff and all three anchor results,
  retain the current node's retry budget, initialize it when absent, and persist
  the fresh pass state before executing compile commands.

No coordinator lock is held across an `.await`, and field write permissions are
unchanged.

## Strict TDD evidence

The following production mutations are caught: omitting pass reset, omitting
new-node reset, or failing to persist/reapply fresh anchors after checkpoint
restore.

### RED (before production changes)

1. `cargo test exec_session::node_runtime::tests::retry_after_test_failure_runs_all_anchors_from_compile_again --lib -- --exact`
   - Failed: retry returned `Implement` instead of `Complete` after executing
     only the retry compile anchor.
2. `cargo test exec_session::node_runtime::tests::two_rust_nodes_in_one_turn_each_run_the_complete_anchor_chain --lib -- --exact`
   - Failed: recorded commands were the first node's full chain plus only the
     second node's `cargo check` (4 calls rather than 6).
3. `cargo test exec_session::node_runtime::tests::retry_after_restoring_failed_verify_checkpoint_reruns_every_anchor --lib -- --exact`
   - Failed: after restoring the persisted failed verify outcome, retry returned
     `Implement` instead of `Complete` after only compile.

### GREEN (after minimal implementation)

The same three exact commands passed individually: 1 passed, 0 failed each.

Focused suites:

- `cargo test exec_session::node_runtime::tests --lib`: 22 passed, 0 failed.
- `cargo test exec_session::work_graph::tests --lib`: 3 passed, 0 failed.
- `cargo test org_graph::work_state::tests --lib`: 18 passed, 0 failed.

## Final verification

- `cargo fmt -- --check`: passed.
- `git diff --check`: passed.
- `cargo clippy --all-targets -- -D warnings`: passed with zero warnings.
- `cargo test --all`: library 1487 passed / 1 ignored; integration 183 passed /
  3 ignored; binary and doc tests passed; 0 failures.

## Concerns

None. The reset is intentionally strongest at pass boundaries: a resumed retry
reruns compile, test, and verify rather than trying to resume mid-chain from
persisted anchor results.
