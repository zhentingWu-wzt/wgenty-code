//! Runtime configuration shared by every agent frontend.

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
            max_rounds: 100,
            plan_mode: false,
            subagent_timeout_secs: 1800,
            context_window: 200_000,
            max_tokens: 4096,
            session_id: String::new(),
            turn_id: None,
            agent_generation: 0,
            stream_max_retries: 2,
        }
    }
}
