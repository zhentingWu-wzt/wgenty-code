//! Token Counter — tracks cumulative API token usage and enforces budget limits.
//!
//! A shared counter incremented on every successful API call. When the
//! configured `token_budget_k` (in thousands) is exceeded, further calls
//! are rejected with a budget-exhausted error.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Shared token usage tracker.
#[derive(Debug, Clone)]
pub struct TokenCounter {
    used: Arc<AtomicUsize>,
    budget: usize, // in thousands of tokens (0 = unlimited)

    // ── Per-turn counters (reset on each user input) ──
    turn_input: Arc<AtomicUsize>,
    turn_output: Arc<AtomicUsize>,
}

impl TokenCounter {
    /// Create a new counter with the given budget in thousands of tokens.
    /// `0` means unlimited.
    pub fn new(budget_k: usize) -> Self {
        Self {
            used: Arc::new(AtomicUsize::new(0)),
            budget: budget_k * 1000,
            turn_input: Arc::new(AtomicUsize::new(0)),
            turn_output: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Total tokens consumed so far.
    pub fn used_tokens(&self) -> usize {
        self.used.load(Ordering::Relaxed)
    }

    /// Budget limit in raw tokens (0 = unlimited).
    pub fn budget_tokens(&self) -> usize {
        self.budget
    }

    /// Add `tokens` to the cumulative count. Returns `true` if still
    /// within budget, `false` if budget would be exceeded.
    ///
    /// Uses compare-exchange loop to ensure the budget check and increment
    /// are atomic — two concurrent calls cannot both pass the check when
    /// only one should.
    pub fn add(&self, tokens: usize) -> bool {
        if self.budget == 0 {
            // Unlimited — just accumulate
            self.used.fetch_add(tokens, Ordering::Relaxed);
            return true;
        }

        loop {
            let current = self.used.load(Ordering::Acquire);
            let new_total = current + tokens;
            if new_total > self.budget {
                return false;
            }
            match self.used.compare_exchange(
                current,
                new_total,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return true,
                Err(_) => continue, // another thread updated; retry
            }
        }
    }

    /// Check whether the budget is already exhausted.
    pub fn is_exhausted(&self) -> bool {
        self.budget > 0 && self.used.load(Ordering::Acquire) >= self.budget
    }

    // ── Per-turn token tracking ──────────────────────────────────────────

    /// Add `tokens` to the per-turn input counter.
    pub fn add_input(&self, tokens: usize) {
        self.turn_input.fetch_add(tokens, Ordering::Relaxed);
    }

    /// Add `tokens` to the per-turn output counter.
    pub fn add_output(&self, tokens: usize) {
        self.turn_output.fetch_add(tokens, Ordering::Relaxed);
    }

    /// Reset per-turn counters to zero (called at start of each turn).
    pub fn reset_turn(&self) {
        self.turn_input.store(0, Ordering::Relaxed);
        self.turn_output.store(0, Ordering::Relaxed);
    }

    /// Current turn's input tokens.
    pub fn turn_input_tokens(&self) -> usize {
        self.turn_input.load(Ordering::Relaxed)
    }

    /// Current turn's output tokens.
    pub fn turn_output_tokens(&self) -> usize {
        self.turn_output.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_turn_counters_start_at_zero() {
        let counter = TokenCounter::new(10);
        assert_eq!(counter.turn_input_tokens(), 0);
        assert_eq!(counter.turn_output_tokens(), 0);
    }

    #[test]
    fn test_add_input_increments_turn_input() {
        let counter = TokenCounter::new(10);
        counter.add_input(100);
        assert_eq!(counter.turn_input_tokens(), 100);
        counter.add_input(50);
        assert_eq!(counter.turn_input_tokens(), 150);
    }

    #[test]
    fn test_add_output_increments_turn_output() {
        let counter = TokenCounter::new(10);
        counter.add_output(200);
        assert_eq!(counter.turn_output_tokens(), 200);
    }

    #[test]
    fn test_reset_turn_clears_both_counters() {
        let counter = TokenCounter::new(10);
        counter.add_input(100);
        counter.add_output(200);
        assert_eq!(counter.turn_input_tokens(), 100);
        assert_eq!(counter.turn_output_tokens(), 200);

        counter.reset_turn();
        assert_eq!(counter.turn_input_tokens(), 0);
        assert_eq!(counter.turn_output_tokens(), 0);
    }

    #[test]
    fn test_turn_counters_do_not_affect_used() {
        let counter = TokenCounter::new(10);
        counter.add_input(50);
        counter.add_output(50);
        assert_eq!(counter.used_tokens(), 0);
    }

    #[test]
    fn test_add_output_does_not_cross_budget() {
        let counter = TokenCounter::new(1); // 1000 token budget
        counter.add_output(999);
        assert_eq!(counter.turn_output_tokens(), 999);
        counter.add_output(2);
        assert_eq!(counter.turn_output_tokens(), 1001);
    }
}
