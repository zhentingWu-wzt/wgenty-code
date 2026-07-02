## 1. Remove user-visible budget warning from App::new

- [x] 1.1 In `src/tui/app/mod.rs`, delete the `token_budget_notice: Option<String>` construction (the `if reminder_token_estimate > 2000 { ... Some(...) } else { None }` block) while keeping the `tracing::warn!` call fired when `reminder_token_estimate > 2000`.
- [x] 1.2 Remove the `if let Some(notice) = token_budget_notice { app.committed_messages.push(UIMessage { role: MessageRole::System, ... }) }` block after `let mut app = Self { ... };`. Restore `Self { ... }` as a direct return (drop the `let mut app =` binding if no longer needed).
- [x] 1.3 Confirm `reminder_token_estimate` is still computed and fed to `tracing::warn!`; remove now-unused imports/bindings (e.g. `UIMessage`, `MessageRole` only if unused elsewhere in this scope).

## 2. Harden welcome-banner render condition

- [x] 2.1 In `src/tui/app/render.rs:68`, replace `self.committed_messages.is_empty()` with a check that no user or assistant turn has been committed (banner shows when all committed messages are `System`, i.e. `committed_messages.iter().all(|m| matches!(m.role, MessageRole::System))`), keeping the `!self.streaming_active` guard.
- [x] 2.2 Verify the banner still hides once a real user/assistant turn exists (manual or existing test).

## 3. Tests

- [x] 3.1 In `src/tui/app/mod.rs::token_budget_tests`, keep `reminder_over_threshold_estimate_exceeds_2000` and `reminder_under_threshold_estimate_stays_quiet` (they test estimation, unaffected). Ensure neither depends on the removed push.
- [x] 3.2 Add a test asserting that `App::new` with an over-threshold reminder does NOT push any `System` message into `committed_messages` (covers spec scenario "No user-visible surface for the budget warning"). If `App::new` is too heavy to construct in a unit test, assert at the level of the removed code path (e.g. a helper that returns the notice is gone) — prefer a direct `App::new`-based assertion if feasible with a fake `$HOME`.
- [x] 3.3 Run `cargo test -p wgenty-code token_budget` and `cargo test --test system_reminder` — all green.

## 4. Spec sync & verification

- [x] 4.1 Confirm `openspec/changes/fix-token-budget-warning-channel/specs/system-reminder-injection/spec.md` MODIFIED delta matches the implemented behavior (dev-log-only, no `committed_messages` injection, banner preserved).
- [x] 4.2 `cargo build` clean; `cargo clippy` clean on touched files.
- [ ] 4.3 Manual TUI smoke: startup in this repo (WGENTY.md+AGENTS.md > 2000 tokens) shows welcome banner and no ⚠ in chat; submitting input enters agent flow. _(deferred to verify phase — interactive TUI)_
