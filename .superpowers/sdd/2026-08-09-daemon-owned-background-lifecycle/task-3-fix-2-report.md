# Task 3 Fix 2 Report — Transactional session switching and pre-run persistence fence

Date: 2026-08-09

## Scope

Resolved the Task 3 rereview P1 findings:

1. The session picker no longer adopts target session B after merely firing a
   cancellation request for session A. It keeps A active until the daemon's run
   claim has been released after final persistence.
2. Starting an explicit server-side turn on an existing daemon session no
   longer performs an unconditional pre-run PUT from stale local history.
3. A local PUT already in flight is serialized before the daemon run claim, so
   it cannot race the run's history ownership boundary.

The observer-only behavior remains unchanged: background SSE renders a notice
and never creates a hidden input, run, retry, claim, or local turn handle.

## Root-cause findings

- `POST /sessions/:id/cancel` returns 204 when it signals the cancellation
  token, not when the run has completed. `RunRegistry` deliberately retains the
  claim until final save and any daemon-owned continuation handoff complete.
- Repeating cancel returns 204 while a claim exists and 404 only once the
  session is idle. Therefore 404 is the available correlated final-release
  signal for a transactional picker switch.
- `start_server_side_run` called `save_session` directly after setting the
  daemon-history fence. That call bypassed the generic save helpers and their
  fence checks.
- The startup TUI session id is minted locally and legitimately requires one
  initial upsert, while picker-loaded and daemon-created ids already belong to
  the daemon and must never be recreated from local history.

## TDD evidence

### RED

- `session_switch_waits_for_old_daemon_run_to_release_before_adopting_target`
  - B was adopted while cancel still returned 204 for A.
- `session_switch_failure_keeps_the_old_session_active`
  - a cancel 500 produced no transactional failure event and B had already
    been adopted.
- `explicit_run_on_existing_daemon_session_does_not_put_stale_local_history`
  - existing-session explicit input issued one stale PUT instead of zero.
- `server_run_waits_for_in_flight_local_put_before_claiming_daemon_ownership`
  - with the shared lock removed, POST `/run` occurred while the prior local
    PUT was still blocked (`1` instead of `0`).

### GREEN

- All four focused P1 tests pass.
- Additional regression coverage proves:
  - a completed switch event with a stale `from_id` is dropped;
  - an idle old session (immediate cancel 404) switches normally;
  - only the locally minted first session may use the initial upsert path;
  - the one-time upsert capability closes after successful persistence.
- `CARGO_INCREMENTAL=0 cargo test tui::app --lib`: 129 passed.

## Implementation

### Transactional picker switch

- Picker selection fences old-session TUI persistence but keeps the active
  session id and SSE readers bound to A.
- `DaemonClient::cancel_run_and_wait_for_release` polls cancellation:
  - 204: cancellation is signalled but the daemon still owns a claim; continue;
  - 404: final release is observed; switching may proceed;
  - error or three-second timeout: report failure and keep A active.
- Successful completion emits `SessionSwitched { from_id, id, name }`.
- The event handler adopts B only when `from_id` still equals the current
  session, preventing a late completion from overriding `/clear` or a newer
  switch. History is loaded only after adoption.

### Initial-only upsert and in-flight save fence

- Added shared `session_needs_initial_upsert` state:
  - true only for the locally minted startup id;
  - false for daemon-created and picker-adopted sessions;
  - set false after any successful persistence.
- Server-side start first marks daemon history ownership, then takes
  `session_save_lock`:
  - an already executing local PUT finishes before the daemon run is claimed;
  - queued local saves observe the sticky ownership fence and drop;
  - existing sessions skip PUT entirely;
  - only an unpersisted local startup id performs GET/404 followed by its
    one-time PUT upsert.
- The same save lock remains held through POST `/run`, making the transition
  from local persistence to daemon ownership ordered and explicit.

## Verification

- `cargo fmt -- --check` — passed.
- `CARGO_INCREMENTAL=0 cargo clippy --all-targets -- -D warnings` — passed.
- `CARGO_INCREMENTAL=0 cargo test --all` — passed:
  - library: 1550 passed, 1 ignored;
  - integration: 195 passed, 3 ignored;
  - doc tests: 0 failed.

## Files changed

- `src/tui/client.rs`
- `src/tui/app/mod.rs`
- `src/tui/app/event_key.rs`
- `src/tui/app/event.rs`
- `src/tui/app/turn.rs`
- `src/tui/app/types.rs`
- `src/tui/util.rs`

Web and Desktop were not changed; the prior audit remains valid and neither
contains a background lifecycle driver. The unrelated user modification in
`docs/superpowers/plans/2026-08-09-realtime-background-results.md` remains
unstaged.
