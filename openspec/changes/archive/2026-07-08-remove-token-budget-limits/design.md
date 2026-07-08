## Context

The token-budget feature has two enforcement points:

1. **Main agent** — `TokenCounter` is constructed in `App::new` with `agent.token_budget.main_k` (0 = unlimited). The agent loop (`src/tui/agent/core.rs`) calls `is_exhausted()` before and after each LLM round; when exhausted it returns `AgentError::TokenBudgetExhausted`, which `turn.rs` maps to `TurnAbortReason::MaxRoundsExceeded`.

2. **Subagent** — The `task` tool resolves a `token_budget_k` via a 4-level fallback (explicit input → `subagent.token_budget_k` → `subagent_default_k` → `main_k`; 0/None = unlimited). `run_subagent_loop` enforces it each round, returning `SubagentError { error_type: ErrorType::BudgetExceeded, … }` with a partial-result snapshot.

The defaults are already 0/None (unlimited), so enforcement only triggers when a user explicitly configures a non-zero budget. The user wants enforcement gone entirely.

## Goals / Non-Goals

**Goals:**
- Eliminate token-budget cut-offs for both the main agent and subagents.
- Remove the now-dead enforcement code paths so `cargo clippy -D warnings` stays clean.
- Preserve token-usage tracking (cumulative + per-turn) for status display and diagnostics.
- Keep existing settings.json files valid (no breaking config change).

**Non-Goals:**
- Remove the `TokenBudget` config struct or `subagent.token_budget_k` config field (kept as inert no-ops for backwards compat).
- Remove the `token_budget` input parameter from the `task` tool schema or the `token_budget_k` parameter from `run_subagent_loop` (kept for progress-display plumbing; just not enforced).
- Touch the transcript-store `token_budget_k` column or `SubagentProgress::token_budget_k` field (display/audit only).
- Change per-turn token tracking or the status-bar token display.

## Decisions

### D1: Remove enforcement only — keep tracking and config fields
**Choice:** Delete the enforcement checks/blocks and the dead `TokenBudgetExhausted` variant + `TokenCounter` budget methods. Leave config fields, `token_budget_k` parameters, and progress/transcript plumbing in place as inert no-ops.
**Rationale:** Removing the `token_budget_k` parameter from `run_subagent_loop` would cascade to 3 callers (`task.rs`, `run_script.rs`, `rlm/pipeline.rs`), the `task` tool schema, `SubagentProgress`, and the transcript-store schema — a medium refactor, not a tweak. Keeping the parameter (un-enforced) is zero-risk and preserves the display/audit trail. Config fields stay serde-accepted so existing settings.json files don't break.
**Alternatives considered:**
- *Remove the entire token-budget feature (config + params + schema + DB column):* cleaner conceptually but touches ~15 files and a SQLite column — too large for a tweak and risks config/DB migration issues.

### D2: Simplify `TokenCounter` — remove `budget` field
**Choice:** Remove the `budget` field, `is_exhausted()`, and `budget_tokens()`. Make `new()` parameterless. `add()` unconditionally accumulates and keeps its `bool` return (always `true`) to avoid touching call sites that ignore the return value.
**Rationale:** After removing enforcement, `is_exhausted()` and `budget_tokens()` are dead code (`-D warnings` would fail). The `budget` field only fed those methods. A parameterless `new()` is the clean API for a counter with no ceiling.
**Alternatives considered:**
- *`#[allow(dead_code)]` on the methods:* leaves dead code, lazy, rejected.

### D3: Remove `AgentError::TokenBudgetExhausted` variant
**Choice:** Delete the variant and update the exhaustive `match` in `turn.rs` (remove it from the `|` pattern with `MaxRoundsExceeded`).
**Rationale:** Only constructed in the two enforcement blocks being deleted. An unused variant on a `pub enum` in a binary crate risks a dead-code warning. The `turn.rs` match is the single exhaustive use site.

## Risks / Trade-offs

- [Agents can run unbounded on token usage] → **Accepted.** This is the explicit user intent. The `max_rounds` limit and per-request `max_tokens` still cap individual turns; only the cumulative budget gate is removed.
- [Config fields become no-ops, potentially confusing] → Mitigation: acceptable for a tweak; a future cleanup can deprecate/remove them with a proper config migration.
- [Test `test_add_output_does_not_cross_budget` in `token_counter.rs` tests budget behavior] → Mitigation: remove/replace the test (the budget concept no longer exists); covered in tasks.
