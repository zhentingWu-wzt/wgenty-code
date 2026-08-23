//! Runtime configuration shared by every agent frontend.

use crate::config::{
    DEFAULT_MAX_ROUNDS, DEFAULT_STREAM_MAX_RETRIES, DEFAULT_SUBAGENT_TIMEOUT_SECS,
};

/// Fallback context window when no model lookup or settings entry provides one.
const DEFAULT_CONTEXT_WINDOW: usize = 200_000;
/// Fallback max output tokens when settings provide none.
const DEFAULT_MAX_TOKENS: usize = 4096;

/// Static knobs for one agent session / turn runner.
///
/// Frontends build this from `Settings` once and pass it into runtime helpers.
/// Mutable per-turn flags (`compact_requested`, stuck detector, …) stay on the
/// frontend or on a future `TurnState` until the full loop is migrated.
#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub max_rounds: usize,
    pub plan_mode: bool,
    pub subagent_timeout_secs: u64,
    pub context_window: usize,
    pub max_tokens: usize,
    pub session_id: String,
    pub turn_id: Option<String>,
    pub agent_generation: u64,
    /// Mid-stream retry budget (not connection-level; those live in ApiClient).
    pub stream_max_retries: u32,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            max_rounds: DEFAULT_MAX_ROUNDS,
            plan_mode: false,
            subagent_timeout_secs: DEFAULT_SUBAGENT_TIMEOUT_SECS,
            context_window: DEFAULT_CONTEXT_WINDOW,
            max_tokens: DEFAULT_MAX_TOKENS,
            session_id: String::new(),
            turn_id: None,
            agent_generation: 0,
            stream_max_retries: DEFAULT_STREAM_MAX_RETRIES,
        }
    }
}
