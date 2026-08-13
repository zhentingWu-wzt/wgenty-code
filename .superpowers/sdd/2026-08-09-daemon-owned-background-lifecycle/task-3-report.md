# Task 3 Report: Make TUI an observer

## Outcome

The TUI now treats session-scoped background results as display-only daemon
notifications. A matching `background_result` global SSE event renders the
existing completion notification but does not enqueue input, start a local
turn, POST `/run`, retry a run claim, or acquire the server-side running gate.

Retained inbox snapshots are recovered by the global-event reader and mapped
to `BackgroundTaskRecovered` display events. They are no longer polled from
`AgentLoop`, appended to TUI-owned model history, or deduplicated through a
client delivery/display ledger. The daemon inbox and continuation scheduler
remain the only owners of model delivery and retry.

Session-event `sync_lost` now advances the rendering cursor and displays an
informational notice. It no longer cancels a daemon run or enters a client
realignment/retry state.

## Removed TUI lifecycle ownership

- Hidden background `PendingInput` constructors and payload fields.
- Background-triggered local model turns and server-side `/run` POSTs.
- The 409 retry/backoff loop and background claim task handle.
- Pending, delivered, and displayed background task-ID ledgers.
- Background acceptance/deferred/realignment application events.
- `/clear` preservation and restoration of a client-owned background claim.
- `AgentLoop` retained-result polling and model-history injection.

The existing task-group continuation path is unchanged; it handles the
separate unified subagent task-group lifecycle, not command background
results.

## TDD evidence

- RED: `cargo test tui::app::turn::tests::completed_background_result_renders_without_starting_hidden_turn_when_idle --lib`
  failed because receiving the result created `current_turn_handle`.
- RED: `cargo test tui::app::turn::tests::background_sse_renders_without_posting_hidden_run_or_owning_gate --lib`
  failed with two captured `/run` POST attempts instead of zero, proving the
  hidden POST and conflict retry path.
- RED: `cargo test tui::app::server_side::tests::retained_background_snapshot_maps_to_display_events_only_for_active_session --lib`
  failed to compile because the display-only snapshot mapper did not exist.
- GREEN: all three focused tests passed after the observer-only change.
- Focused regression: `cargo test tui::app --lib` passed 119 tests; `cargo test
  tui::agent --lib` passed 6 tests.

## Web and Desktop audit

No background lifecycle driver exists in Web or Desktop, so no changes were
needed there.

- Web has one run entry point: the Composer's explicit user `onSend` calls
  `runSessionTurn`, which calls `DaemonClient::runSession`.
- Web's retry callback retries only that explicit failed user submission. No
  code subscribes to `background_result` or starts a run from a background
  notification.
- Desktop hosts the same Web application and manages daemon startup/token
  injection only. Its Rust and JavaScript host code contains no background
  result handler, continuation claim, or run POST.

## Verification

- `cargo fmt -- --check` — passed.
- `CARGO_INCREMENTAL=0 cargo clippy --all-targets -- -D warnings` — passed.
- `CARGO_INCREMENTAL=0 cargo test --all` — passed:
  - library: 1540 passed, 1 ignored, 0 failed;
  - integration: 195 passed, 3 ignored, 0 failed;
  - doc tests: 0 failed.
- Residual ownership audit found no `server_background`,
  `pending_server_background`, `delivered_background`,
  `displayed_background`, or `inject_background_results` symbol under
  `src/tui`.

## Files changed

- `src/tui/agent/compaction.rs`
- `src/tui/agent/mod.rs`
- `src/tui/app/event.rs`
- `src/tui/app/event_key.rs`
- `src/tui/app/input.rs`
- `src/tui/app/mod.rs`
- `src/tui/app/server_side.rs`
- `src/tui/app/turn.rs`
- `src/tui/app/types.rs`
- `src/tui/util.rs`
- `.superpowers/sdd/2026-08-09-daemon-owned-background-lifecycle/task-3-report.md`

The pre-existing user modification to
`docs/superpowers/plans/2026-08-09-realtime-background-results.md` was
preserved and excluded from the commit.
