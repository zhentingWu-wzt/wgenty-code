# Daemon-owned Background Lifecycle Design

## Goal

Make the daemon the sole owner of background-result delivery, continuation scheduling, run claims, cancellation, and retry. The TUI only renders session SSE events and sends explicit user input.

## Problem

The current server-side run loop is daemon-owned, but the TUI still claims and retries background continuations. That crosses process boundaries with `RunRegistry` final-save, SSE reconnect, `/clear`, and session changes, creating unresolvable ownership races.

## Design

### Session inbox

Add a daemon-owned, session-scoped background-result inbox. A completed `BackgroundResult` is retained by its originating `session_id`, deduplicated by task ID, and published as an SSE status event only after storage succeeds. Legacy results without a session ID remain unowned and are never delivered.

### Continuation scheduler

When a result is enqueued, the daemon scheduler checks the session's `RunRegistry` claim. If idle, it atomically claims a continuation run, drains one or more pending results into a structured user message, and runs the existing daemon loop. If busy, it leaves the inbox untouched; the finishing run schedules the next continuation before releasing ownership. Cancellation and final persistence remain inside the same run task.

### TUI simplification

The TUI removes hidden background `/run` calls, local delivery/display ledgers, continuation claim handles, retry loops, and server-run gates used exclusively for background delivery. It consumes session SSE events for display. `/clear` only sends daemon cancel/clear semantics and adopts the returned session state.

### Recovery

The daemon inbox is the recovery source of truth. SSE is notification-only: reconnects can replay status, but replay cannot create a continuation. Session event sequence handling remains a rendering concern, not a lifecycle gate.

## Safety invariants

- Every result belongs to at most one session and is consumed at most once by that session's model history.
- Only daemon code creates continuation runs or modifies the run claim.
- A run persists its final history before the next continuation is claimed.
- Session clear/cancel is serialized with inbox delivery by the daemon session state machine.

## Verification

- Daemon integration tests cover busy-to-idle continuation, cancel/clear, duplicate result, reconnect, and two-session isolation.
- TUI tests prove background SSE only renders and never POSTs a hidden run.
- Full `cargo fmt -- --check`, clippy, and `cargo test --all` pass.
