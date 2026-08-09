# Daemon-owned Background Lifecycle Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans. Steps use checkbox syntax.

**Goal:** Move background-result continuation ownership into the daemon.

**Architecture:** A session inbox retains deduplicated results. The daemon schedules continuation runs under `RunRegistry`; TUI only renders SSE.

**Tech Stack:** Rust, Tokio, Axum, broadcast SSE.

## Global Constraints

- Results require a session ID and are consumed exactly once per session.
- Only daemon code creates background continuation runs.
- Run final-save precedes the next continuation claim.
- Run `cargo fmt -- --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test --all`.

### Task 1: Daemon session inbox

**Files:** `src/daemon/state.rs`, `src/daemon/handlers.rs`, daemon tests.

- [ ] Write failing tests for per-session retention, duplicate task IDs, and foreign-session isolation.
- [ ] Run focused daemon tests and observe failure.
- [ ] Add a daemon-owned inbox keyed by session and task ID; enqueue before SSE notification.
- [ ] Verify focused tests pass and commit `feat(daemon): retain background results per session`.

### Task 2: Daemon continuation scheduler

**Files:** `src/daemon/run_loop.rs`, `src/daemon/state.rs`, `src/daemon/handlers.rs`, integration tests.

- [ ] Write failing tests for idle delivery, busy-to-idle scheduling, cancellation, and final-save ordering.
- [ ] Run tests and observe failure.
- [ ] Add scheduler hooks around run completion that drain the inbox into a structured continuation message under the existing run claim.
- [ ] Verify daemon/integration tests pass and commit `feat(daemon): schedule background continuations`.

### Task 3: Make TUI an observer

**Files:** `src/tui/app/{event.rs,input.rs,server_side.rs,turn.rs,mod.rs}`, TUI tests.

- [ ] Write failing tests proving `background_result` renders but never POSTs a hidden run or owns a retry/gate.
- [ ] Run tests and observe failure.
- [ ] Remove TUI background continuation claim/retry/ledger logic; retain session SSE rendering and snapshot display recovery only.
- [ ] Verify focused tests pass and commit `refactor(tui): observe daemon background lifecycle`.

### Task 4: End-to-end verification

- [ ] Add integration coverage for two-session isolation, reconnect, clear/cancel, duplicate delivery, and busy results.
- [ ] Run formatter, clippy, and full test suite.
- [ ] Review the diff and commit `test(daemon): cover background lifecycle ownership`.
