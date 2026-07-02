## Why

Verify-phase commit `006945f` (PR `0069259`, "test+feat(tui): close verify-phase minor gaps") routed the per-session token-budget warning into `committed_messages` (the chat log) instead of a transient surface. Because `WGENTY.md` + `AGENTS.md` in real projects routinely exceed the 2000-token threshold, the warning fires on every startup, which (a) makes `committed_messages` non-empty and suppresses the TUI welcome banner, and (b) leaves a standalone `System` message in the chat that users mistake for a submitted-but-ignored "system prompt". The warning also has low terminal-user value: whether to trim `WGENTY.md` is a project decision, not something a recurring ⚠ usefully influences. The dev-facing `tracing::warn!` already serves operators. We therefore downgrade the spec's user-visible-warning requirement to dev-log-only and remove the chat-log injection.

## What Changes

- Remove the `token_budget_notice` construction and the `app.committed_messages.push(...)` injection in `App::new` (`src/tui/app/mod.rs`). The `tracing::warn!` dev log is retained unchanged.
- Harden the welcome-banner render condition in `src/tui/app/render.rs` so it no longer relies solely on `committed_messages.is_empty()`; the banner shows when no user/assistant turn has occurred, regardless of any startup-injected system messages.
- **SPEC CHANGE** — Modify `system-reminder-injection`: the "Token budget calculation and one-time warning" requirement no longer mandates a user-visible warning in the TUI status area. Token-budget estimation is still computed once per session and emitted via `tracing::warn!` (dev log only); no user-visible surface is required.
- Adjust tests: `tests/system_reminder.rs` assertions expecting the `System` message in `committed_messages` are updated to assert it is absent; `src/tui/app/mod.rs::token_budget_tests` over-threshold case is updated to verify dev-log emission only (or removed where it depended on the chat-log channel).

## Capabilities

### New Capabilities
<!-- None — this is a modification + bugfix. -->

### Modified Capabilities
- `system-reminder-injection`: The "Token budget calculation and one-time warning" requirement is downgraded — the per-session warning is dev-log-only (`tracing::warn!`), no longer emitted to the TUI status area or any user-visible chat surface. Estimation still runs once per session; the threshold (2000) and the four-source calculation are unchanged.

## Impact

- **Code**: `src/tui/app/mod.rs` (`App::new` — remove notice construction + push, keep `tracing::warn!`), `src/tui/app/render.rs` (welcome-banner condition hardening).
- **Specs**: `openspec/specs/system-reminder-injection/spec.md` — downgrade the warning requirement (delta in this change).
- **Tests**: `tests/system_reminder.rs`, `src/tui/app/mod.rs::token_budget_tests` — assert absence of the injected `System` message; dev-log-only behavior.
- **User-visible behavior**: TUI welcome banner renders again on startup; no ⚠ message appears in the chat log; "submitted system prompt with no response" misimpression is eliminated. Operators still see the warning in dev logs.
- **Non-goals**: token threshold (2000) unchanged; reminder estimation logic unchanged; `<system-reminder>` injection main flow unchanged; welcome-banner component itself not rewritten; no new user-visible notification mechanism introduced.
