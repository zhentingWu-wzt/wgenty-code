# Remove token budget limits for main agent and subagents

## Why

Token budget enforcement prematurely terminates the main agent (`AgentError::TokenBudgetExhausted`) and subagents (`SubagentError` with `ErrorType::BudgetExceeded`) when a configured cumulative-token limit is reached. The user wants both the main agent and subagents to run without token-budget constraints — they should never be cut off mid-task by a budget ceiling.

Today the enforcement lives in two places:

- **Main agent** — `src/tui/agent/core.rs` checks `TokenCounter::is_exhausted()` before and after every LLM call and returns `AgentError::TokenBudgetExhausted`.
- **Subagent** — `src/teams/subagent_loop.rs` checks `if used > budget_k * 1000` each round and returns a `BudgetExceeded` error.

The tracking infrastructure (cumulative + per-turn counters) is useful for display and diagnostics and should stay; only the **enforcement** (the hard cut-off) is removed.

## What Changes

1. **Main agent loop** (`src/tui/agent/core.rs`) — delete the two `is_exhausted()` enforcement blocks (pre-LLM-call and post-accounting). Keep `token_counter.add()` for usage tracking.
2. **`AgentError`** (`src/tui/agent/mod.rs`) — remove the now-unused `TokenBudgetExhausted { budget_k }` variant.
3. **Turn abort mapping** (`src/tui/app/turn.rs`) — remove `TokenBudgetExhausted` from the `match` arm.
4. **Subagent loop** (`src/teams/subagent_loop.rs`) — delete the `if let Some(budget_k) = token_budget_k { … }` enforcement block. Keep the `token_budget_k` parameter (still flows to `SubagentProgress` for display) and `cumulative_tokens` tracking.
5. **`TokenCounter`** (`src/api/token_counter.rs`) — remove the `budget` field, `is_exhausted()`, and `budget_tokens()` (now dead code). Simplify `new()` to parameterless; `add()` always accumulates and returns `true`. Update tests.
6. **TokenCounter caller** (`src/tui/app/mod.rs`) — update `TokenCounter::new(s.agent.token_budget.main_k)` → `TokenCounter::new()`.

## Impact

- **Behavior**: Main agent and subagents no longer terminate on token-budget exhaustion. Long-running tasks complete without artificial cut-offs.
- **Tracking preserved**: Per-turn and cumulative token counters still work (used for status-bar display and diagnostics).
- **Config compatibility**: `agent.token_budget.main_k`, `agent.token_budget.subagent_default_k`, and `agent.subagent.token_budget_k` remain valid settings (serde-accepted) but become inert no-ops — no settings.json migration needed.
- **Surface area**: 6 source files + 1 test file. No schema changes, no DB migration, no new dependencies.
