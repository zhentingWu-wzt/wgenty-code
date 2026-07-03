## Context

Commit `006945f` (verify-phase) added a startup token-budget warning in `App::new` (`src/tui/app/mod.rs`). It estimated the `<system-reminder>` block size and, when > 2000 tokens, pushed a `System` `UIMessage` into `committed_messages`. Two regressions followed:

1. `render.rs:68` shows the welcome banner only when `committed_messages.is_empty() && !streaming_active`. The injected message makes that false on every startup for non-trivial projects → banner never shows.
2. The injected `System` message sits alone in the chat log, doesn't trigger an agent turn, and reads to users as "I submitted a system prompt and nothing happened."

The `system-reminder-injection` spec previously mandated a user-visible warning to the TUI status area. This change downgrades that requirement to dev-log-only (`tracing::warn!`), which the implementation already emits alongside the (now-removed) chat-log injection.

## Goals / Non-Goals

**Goals:**
- Eliminate the `committed_messages` injection so the welcome banner renders on startup.
- Stop the standalone `System` message that causes the "submitted prompt, no response" misimpression.
- Keep the operator-facing `tracing::warn!` dev log.
- Harden the welcome-banner condition so future startup-injected system messages cannot silently suppress it.
- Align the spec with the dev-log-only behavior.

**Non-Goals:**
- Change the 2000-token threshold or the estimation algorithm.
- Touch the `<system-reminder>` injection main flow (`build_user_turn_reminder`).
- Rewrite the welcome-banner component.
- Introduce a new user-visible notification mechanism (no status-area warning, no toast).
- Add per-file size breakdown to any warning (no user-visible warning exists).

## Decisions

### D1: Remove the user-visible warning entirely (dev-log only), not route it to the status area
**Choice:** Delete the `token_budget_notice` construction and the `committed_messages.push`. Keep `tracing::warn!`.
**Rationale:** A recurring ⚠ offers low value to terminal users — trimming `WGENTY.md` is a project decision, not a runtime nudge. The dev log already serves operators. Routing to the status area (the spec's literal wording) would preserve a surface that fires every startup for real projects and still needs lifecycle/dismissal logic. Removing it is simpler and eliminates both regressions at the root.
**Alternatives considered:**
- *Route to `render_status` with first-input dismissal:* matches the old spec letter, but adds transient-state plumbing for a low-value notice and keeps a per-startup interruption.
- *Raise the threshold:* paper-over; the warning still misfires for large but legitimate projects.

### D2: Harden the welcome-banner condition by filtering on user/assistant turns, not message-list emptiness
**Choice:** In `render.rs`, replace `self.committed_messages.is_empty()` with a check that no user or assistant turn has been committed (e.g. `committed_messages.iter().all(|m| matches!(m.role, MessageRole::System))`), keeping the `!streaming_active` guard.
**Rationale:** The root fragility is that any startup `System` message suppresses the banner. Filtering on user/assistant presence makes the banner robust to future system-message injections without a new state flag.
**Alternatives considered:**
- *Explicit `show_welcome: bool` flag:* more invasive (new field + lifecycle setters) for the same effect; rejected as over-engineering for a hotfix.

### D3: Spec downgrade via MODIFIED delta
**Choice:** Modify the "Token budget calculation and one-time warning" requirement in `system-reminder-injection` to dev-log-only, with a new scenario asserting no `committed_messages` injection and banner preservation.
**Rationale:** The implementation contract changes (no user-visible surface), so the spec must follow to stay truthful. MODIFIED (full block) preserves the under-threshold / subsequent-turn / cross-source scenarios while swapping the channel.

## Risks / Trade-offs

- [Operators lose a user-visible budget signal] → Mitigation: `tracing::warn!` retained; operators who relied on the ⚠ must now check dev logs. Acceptable given low terminal-user value.
- [Existing test `tests/system_reminder.rs` asserts the `System` message presence] → Mitigation: update assertions to assert absence; covered in tasks.
- [Welcome-banner condition change could mask a legitimately empty chat that should show chat surface] → Mitigation: only `System` messages are treated as banner-compatible; user/assistant turns flip to chat view as before.
