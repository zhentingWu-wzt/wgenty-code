# Task 3 Fix 1 Report — TUI cancellation and daemon-owned history fencing

Date: 2026-08-09

## Scope

Resolved both P1 findings from `task-3-review.md` while preserving the Task 3
observer-only contract:

1. `/clear` and explicit session switches request cancellation for the old
   daemon session even when `server_side_turn_active` is false.
2. Once the daemon can mutate a session through a server-side run or background
   continuation, generic TUI persistence no longer PUTs local history over the
   daemon's final history.

No Web or Desktop lifecycle behavior changed. The Task 3 audit still holds:
Web starts runs only from explicit Composer submission, and Desktop only hosts
the Web UI / daemon process; neither contains an automatic background
continuation driver.

## TDD evidence

### RED

- `clear_cancels_daemon_continuation_without_a_local_run_gate`
  - cancellation count was `0`, expected `1`.
- `exit_snapshot_after_background_notice_does_not_put_stale_history`
  - exit snapshot issued `1` PUT, expected `0`.
- `queued_local_save_is_dropped_after_daemon_takes_history_ownership`
  - a save queued behind `session_save_lock` issued `1` PUT after daemon
    ownership became visible, expected `0`.
- `switching_sessions_requests_cancellation_for_invisible_daemon_work`
  - timed out because the old session received no cancellation request.

The fourth RED compile initially hit `No space left on device`; only the
current worktree's reproducible Cargo build cache was cleaned, then the RED was
rerun successfully.

### GREEN

- All four focused regression tests pass.
- `CARGO_INCREMENTAL=0 cargo test tui::app --lib`
  - 123 passed, 0 failed.

## Implementation

### Cancellation before reset/rebind

- `/clear` now fences old-session persistence and always calls daemon
  cancellation; it no longer branches on the local server-side run gate and no
  longer PUTs the old TUI snapshot.
- Explicit session adoption fences old-session persistence and asynchronously
  requests daemon cancellation before rebinding the SSE observers.
- Cancellation failure during `/clear` restores the old local history and does
  not create/adopt a new session.

### Daemon-owned history write fence

- Added a per-session shared, sticky `daemon_owns_session_history` flag.
- The flag is set when:
  - an explicit server-side run starts;
  - the local loop returns a `background` tool result;
  - a live or recovered background completion is rendered;
  - clear/session-switch begins fencing the old session.
- Both exit snapshot persistence and fire-and-forget persistence check the flag
  before scheduling/work and again after acquiring `session_save_lock`.
- Session adoption replaces the flag's `Arc` instead of clearing it, so already
  queued save tasks for the old session retain the sticky fence and cannot write
  into it after a switch.

This changes only persistence/cancellation safety. Background SSE events still
render notifications and do not enqueue input, POST `/run`, acquire a local turn
handle, retry a hidden turn, or claim lifecycle ownership.

## Verification

- `cargo fmt -- --check` — passed.
- `CARGO_INCREMENTAL=0 cargo clippy --all-targets -- -D warnings` — passed.
- `CARGO_INCREMENTAL=0 cargo test --all` — passed:
  - library: 1544 passed, 1 ignored;
  - integration: 195 passed, 3 ignored;
  - doc tests: 0 failed.

## Files changed

- `src/tui/app/input.rs`
- `src/tui/app/mod.rs`
- `src/tui/app/event.rs`
- `src/tui/app/event_key.rs`
- `src/tui/app/turn.rs`

The unrelated user modification in
`docs/superpowers/plans/2026-08-09-realtime-background-results.md` was preserved
and excluded from the commit.
