//! Errors shared by the unified agent runtime.

/// Structured error returned by runtime stream / turn helpers.
///
/// Frontends map this into their own error type (e.g. TUI `AgentError`) so
/// callers never depend on fragile substring checks except at the single
/// timeout-classification boundary inside the stream layer.
#[derive(Debug, Clone, thiserror::Error)]
pub enum RuntimeError {
    /// Stream timed out (connection timeout or idle stall).
    #[error("{0}")]
    StreamTimeout(String),
    /// Unrecoverable stream / API error.
    #[error("{0}")]
    Stream(String),
    /// Agent exceeded the configured max LLM rounds.
    #[error("Exceeded {max_rounds} LLM rounds")]
    MaxRoundsExceeded { max_rounds: usize },
    /// Dedicated planner model call failed.
    #[error("Planner model call failed: {0}")]
    Planner(String),
    /// API returned a completely empty response.
    #[error("Empty response from API")]
    EmptyResponse,
    /// Tool execution failed before a structured tool result was produced.
    #[error("Tool error: {0}")]
    Tool(String),
}

impl RuntimeError {
    /// Heuristic: classify a free-form stream failure string as timeout.
    ///
    /// Kept in one place so frontends don't re-implement substring matching.
    pub fn from_stream_failure(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        let is_timeout = msg.contains("timed out")
            || msg.contains("Stream stalled")
            || msg.contains("timeout");
        if is_timeout {
            Self::StreamTimeout(msg)
        } else {
            Self::Stream(msg)
        }
    }
}
