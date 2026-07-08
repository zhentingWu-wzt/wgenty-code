//! Token Counter — tracks cumulative API token usage for display and diagnostics.
//!
//! A shared counter incremented on every successful API call. The counter no
//! longer enforces a budget ceiling — agents run without token-budget limits.
//! Per-turn input/output counters (reset on each user input) are also tracked
//! for status-bar display.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Shared token usage tracker.
#[derive(Debug, Clone)]
pub struct TokenCounter {
    used: Arc<AtomicUsize>,

    // ── Per-turn counters (reset on each user input) ──
    turn_input: Arc<AtomicUsize>,
    turn_output: Arc<AtomicUsize>,

    /// Tokens used in the most recent prompt (set by caller before each send).
    last_prompt_tokens: Arc<AtomicUsize>,
}

impl TokenCounter {
    /// Create a new counter with no budget ceiling (unlimited).
    pub fn new() -> Self {
        Self {
            used: Arc::new(AtomicUsize::new(0)),
            turn_input: Arc::new(AtomicUsize::new(0)),
            turn_output: Arc::new(AtomicUsize::new(0)),
            last_prompt_tokens: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Total tokens consumed so far.
    pub fn used_tokens(&self) -> usize {
        self.used.load(Ordering::Relaxed)
    }

    /// Add `tokens` to the cumulative count. Always succeeds (no budget
    /// ceiling). The `bool` return is kept for API compatibility with callers
    /// that ignore it.
    pub fn add(&self, tokens: usize) -> bool {
        self.used.fetch_add(tokens, Ordering::Relaxed);
        true
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

    /// Set the number of tokens used in the most recent prompt.
    pub fn set_prompt_tokens(&self, tokens: usize) {
        self.last_prompt_tokens.store(tokens, Ordering::Relaxed);
    }

    /// Number of tokens used in the most recent prompt.
    pub fn last_prompt_tokens(&self) -> usize {
        self.last_prompt_tokens.load(Ordering::Relaxed)
    }
}

impl Default for TokenCounter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_turn_counters_start_at_zero() {
        let counter = TokenCounter::new();
        assert_eq!(counter.turn_input_tokens(), 0);
        assert_eq!(counter.turn_output_tokens(), 0);
    }

    #[test]
    fn test_add_input_increments_turn_input() {
        let counter = TokenCounter::new();
        counter.add_input(100);
        assert_eq!(counter.turn_input_tokens(), 100);
        counter.add_input(50);
        assert_eq!(counter.turn_input_tokens(), 150);
    }

    #[test]
    fn test_add_output_increments_turn_output() {
        let counter = TokenCounter::new();
        counter.add_output(200);
        assert_eq!(counter.turn_output_tokens(), 200);
    }

    #[test]
    fn test_reset_turn_clears_both_counters() {
        let counter = TokenCounter::new();
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
        let counter = TokenCounter::new();
        counter.add_input(50);
        counter.add_output(50);
        assert_eq!(counter.used_tokens(), 0);
    }

    #[test]
    fn test_add_accumulates_used_tokens() {
        let counter = TokenCounter::new();
        assert!(counter.add(500));
        assert_eq!(counter.used_tokens(), 500);
        assert!(counter.add(300));
        assert_eq!(counter.used_tokens(), 800);
    }

    // ── last_prompt_tokens tests ─────────────────────────────────────────

    #[test]
    fn test_last_prompt_tokens_starts_at_zero() {
        let counter = TokenCounter::new();
        assert_eq!(counter.last_prompt_tokens(), 0);
    }

    #[test]
    fn test_set_prompt_tokens_stores_value() {
        let counter = TokenCounter::new();
        counter.set_prompt_tokens(500);
        assert_eq!(counter.last_prompt_tokens(), 500);
    }

    #[test]
    fn test_set_prompt_tokens_overwrites() {
        let counter = TokenCounter::new();
        counter.set_prompt_tokens(100);
        counter.set_prompt_tokens(200);
        assert_eq!(counter.last_prompt_tokens(), 200);
    }
}
