## 1. Remove main agent token budget enforcement

- [x] 1.1 In `src/tui/agent/core.rs`, delete the pre-LLM-call enforcement block (`if self.token_counter.is_exhausted() { … return Err(AgentError::TokenBudgetExhausted { … }); }`).
- [x] 1.2 In `src/tui/agent/core.rs`, delete the post-accounting enforcement block (second `if self.token_counter.is_exhausted() { … }`). Keep the `token_counter.add()` / `add_output()` calls.
- [x] 1.3 In `src/tui/agent/mod.rs`, remove the `TokenBudgetExhausted { budget_k: usize }` variant from `AgentError`.
- [x] 1.4 In `src/tui/app/turn.rs`, remove `AgentError::TokenBudgetExhausted { .. }` from the `|` match arm (leave `MaxRoundsExceeded` mapping to `TurnAbortReason::MaxRoundsExceeded`).

## 2. Remove subagent token budget enforcement

- [x] 2.1 In `src/teams/subagent_loop.rs`, delete the `if let Some(budget_k) = token_budget_k { … return Err(SubagentError { error_type: ErrorType::BudgetExceeded, … }); }` enforcement block. Keep the `cumulative_tokens` accumulation above it and the `token_budget_k` parameter (still used in `SubagentProgress`).

## 3. Simplify TokenCounter

- [x] 3.1 In `src/api/token_counter.rs`, remove the `budget` field from the struct. Change `pub fn new(budget_k: usize)` → `pub fn new()`. Simplify `add()` to unconditionally accumulate (remove the budget == 0 fast-path and the CAS loop; always `fetch_add` + return `true`). Remove `is_exhausted()` and `budget_tokens()`. Added `impl Default` (clippy `new_without_default`).
- [x] 3.2 Update `token_counter.rs` tests: change `TokenCounter::new(10)` / `TokenCounter::new(1)` → `TokenCounter::new()`. Replaced `test_add_output_does_not_cross_budget` with `test_add_accumulates_used_tokens`. All turn-counter tests pass.
- [x] 3.3 In `src/tui/app/mod.rs`, update `TokenCounter::new(s.agent.token_budget.main_k)` → `TokenCounter::new()`.

## 4. Verify

- [x] 4.1 `cargo fmt` — no diff.
- [x] 4.2 `cargo clippy --all-targets -- -D warnings` — zero warnings.
- [x] 4.3 `cargo test --all` — all green.
