# Verification Report: fix-token-budget-warning-channel

**Date:** 2026-07-02
**Verify mode:** full (tasks=10 > 3, delta spec=1 capability, changed files=8)
**Branch:** feature/20260702/fix-token-budget-warning-channel

## Summary

| Dimension    | Status |
|--------------|--------|
| Completeness | 10/10 tasks complete; 1 MODIFIED requirement, 5 scenarios |
| Correctness  | 5/5 scenarios mapped to implementation evidence; tests green |
| Coherence    | Design decisions D1–D3 followed; no spec/design contradiction |

## Completeness

- **Tasks:** 10/10 `[x]`, 0 incomplete (`grep -c '^- \[ \]'` = 0). The interactive-TUI smoke was correctly reclassified out of build-completable tasks (documented in tasks.md §4 note); its underlying invariants are unit-tested.
- **Spec coverage:** 1 MODIFIED requirement ("Token budget calculation and one-time warning") in delta `specs/system-reminder-injection/spec.md`. Requirement title matches the main spec exactly (whitespace-insensitive), so archive merge will patch in place.

## Correctness — scenario → implementation evidence

| Scenario | Evidence |
|---|---|
| Reminder block under threshold | `tracing::warn!` only fires `if reminder_token_estimate > 2000` (`src/tui/app/mod.rs:305`); under-threshold emits nothing. Covered by `reminder_under_threshold_estimate_stays_quiet`. |
| Reminder block exceeds threshold on first turn | `tracing::warn!` at `mod.rs:306` fires once at startup (session_init runs once). No `committed_messages` push (`grep committed_messages.push` → no match, exit 1). Covered by `budget_warning_is_dev_log_only_no_user_visible_notice`. |
| Threshold-exceeding on subsequent turns | Dev-log fires only in `App::new` (startup); no per-turn re-emit. Existing `second_turn_reminder_reappears` confirms reminder structure; warning-once semantics preserved by construction. |
| Threshold computed across all sources | Estimation uses `build_user_turn_reminder(&preview_ctx, &[])` over all 4 sources (`mod.rs:290-299`). Unchanged. Covered by `reminder_over_threshold_estimate_exceeds_2000`. |
| No user-visible surface for the budget warning | No `System` message pushed; welcome banner condition filters on non-`System` roles (`render.rs:73-77`). Guard test asserts `token_budget_notice` binding is absent. |

**Tests (fresh run):**
- `cargo test --lib` → 453 passed; 0 failed.
- `cargo test --test system_reminder` → 8 passed; 0 failed.
- `cargo build` → exit 0. `cargo clippy --lib --tests` → clean. `cargo fmt --check` → clean.

## Coherence — design adherence

- **D1 (dev-log-only, not status area):** Implemented — notice construction + push removed; `tracing::warn!` retained. ✓
- **D2 (filter on user/assistant/tool turns, not emptiness):** Implemented — `has_real_turn = committed_messages.iter().any(|m| !matches!(m.role, MessageRole::System))` (`render.rs:73-76`). ✓
- **D3 (MODIFIED delta):** Delta created with full requirement block + new "No user-visible surface" scenario. ✓
- **Spec/design contradiction (comet-verify check 6):** None. The design explicitly states the spec is downgraded to dev-log-only; the delta matches. No "Implementation Divergence" section needed.

## Issues

- **CRITICAL:** none.
- **WARNING:** none.
- **SUGGESTION:** The guard test `budget_warning_is_dev_log_only_no_user_visible_notice` uses an include_str!-based source-text check with a fragment-assembled forbidden token to avoid self-match. It is a pragmatic regression guard given `App::new` is too heavy to construct in a unit test. If a future refactor makes `App::new` testable, prefer a behavioral assertion. Not blocking.

## Final Assessment

All checks passed. Ready for archive. The interactive-TUI smoke (welcome banner renders, no ⚠ in chat) is the one item not exercised by automation — its preconditions are proven by the root-cause removal and the hardened render condition; a manual TUI launch remains recommended before merge but is not a blocker for this hotfix's spec contract.
