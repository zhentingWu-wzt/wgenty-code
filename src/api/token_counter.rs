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
}

impl TokenCounter {
    /// Create a new counter with the given budget in thousands of tokens.
    /// `0` means unlimited.
    pub fn new(budget_k: usize) -> Self {
        Self {
            used: Arc::new(AtomicUsize::new(0)),
            budget: budget_k * 1000,
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
    /// within budget, `false` if budget was just exceeded.
    pub fn add(&self, tokens: usize) -> bool {
        let new_total = self.used.fetch_add(tokens, Ordering::Relaxed) + tokens;
        self.budget == 0 || new_total <= self.budget
    }

    /// Check whether the budget is already exhausted.
    pub fn is_exhausted(&self) -> bool {
        self.budget > 0 && self.used.load(Ordering::Relaxed) >= self.budget
    }
}
