//! Shared stuck detector — reused by both the main agent loop and subagent loop.
//!
//! Tracks consecutive tool-call patterns to detect when an LLM is stuck
//! repeating identical operations without making progress.

/// Tracks consecutive tool-call patterns to detect when the LLM is stuck
/// repeating identical operations without making progress.
pub struct StuckDetector {
    prev_signatures: Vec<(String, String)>,
    stale_rounds: usize,
}

impl StuckDetector {
    pub fn new() -> Self {
        Self {
            prev_signatures: Vec::new(),
            stale_rounds: 0,
        }
    }

    /// Record a round of tool calls. Returns:
    /// - `StuckStatus::Ok` on first occurrence or first repeat (could be legitimate)
    /// - `StuckStatus::Warn(msg)` on second repeat (warning to inject into tool results)
    /// - `StuckStatus::Abort(msg)` on third+ repeat (agent should stop)
    pub fn record_round(&mut self, tool_calls: &[crate::api::ToolCall]) -> StuckStatus {
        let signatures: Vec<(String, String)> = tool_calls
            .iter()
            .map(|tc| {
                let keys = sorted_arg_keys(&tc.function.arguments);
                (tc.function.name.clone(), keys)
            })
            .collect();

        if signatures == self.prev_signatures && !signatures.is_empty() {
            self.stale_rounds += 1;
        } else {
            self.stale_rounds = 0;
        }
        self.prev_signatures = signatures;

        match self.stale_rounds {
            0 => StuckStatus::Ok,
            1 => StuckStatus::Ok, // first repeat could be legitimate
            2 => StuckStatus::Warn(
                "\n\n\u{26A0}\u{FE0F} You have repeated the same tool calls multiple times. \
                 Consider a different approach or ask the user for guidance."
                    .to_string(),
            ),
            _ => StuckStatus::Abort(
                "Stuck in loop: repeated identical tool calls 3+ times".to_string(),
            ),
        }
    }
}

pub enum StuckStatus {
    Ok,
    Warn(String),
    Abort(String),
}

/// Extract sorted JSON keys from tool arguments for signature comparison.
pub(crate) fn sorted_arg_keys(args_json: &str) -> String {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(args_json) {
        if let Some(obj) = v.as_object() {
            let mut keys: Vec<&str> = obj.keys().map(|s| s.as_str()).collect();
            keys.sort();
            return keys.join(",");
        }
    }
    String::new()
}
