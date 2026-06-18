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

impl Default for StuckDetector {
    fn default() -> Self {
        Self::new()
    }
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
                let sig = args_signature(&tc.function.arguments);
                (tc.function.name.clone(), sig)
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

/// Build a stable signature from tool arguments including both keys and values,
/// so that calls with different parameter values produce different signatures.
fn args_signature(args_json: &str) -> String {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(args_json) {
        if let Some(obj) = v.as_object() {
            let mut pairs: Vec<(&str, String)> = obj
                .iter()
                .map(|(k, val)| (k.as_str(), value_fragment(val)))
                .collect();
            pairs.sort_by_key(|(k, _)| *k);
            return pairs
                .iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect::<Vec<_>>()
                .join(",");
        }
    }
    String::new()
}

/// Extract a short string representation of a JSON value for signature use.
fn value_fragment(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => {
            // Truncate long strings to keep signatures compact but distinct
            if s.len() > 80 {
                format!("{:.80}…", s)
            } else {
                s.clone()
            }
        }
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Array(arr) => format!("[len={}]", arr.len()),
        serde_json::Value::Object(_) => "{…}".to_string(),
    }
}
