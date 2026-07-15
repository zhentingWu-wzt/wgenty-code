//! Frontend-agnostic events emitted by the agent runtime.

/// Events produced by the shared stream / loop engines.
///
/// Frontends map these into their own UI event type (TUI `AppEvent`, CLI
/// stdout, headless logs). Runtime code must never import TUI types.
#[derive(Debug, Clone)]
pub enum RuntimeEvent {
    /// About to open (or retry) an LLM stream.
    Connecting { attempt: usize, max_retries: usize },
    /// Model text delta.
    ContentDelta(String),
    /// Model reasoning / thinking delta.
    ReasoningDelta(String),
    /// First tool-call fragment observed; UI may show "preparing tools…".
    PreparingTools,
    /// Stream finished with a finish_reason (may be empty on incomplete streams).
    StreamDone { finish_reason: String },
    /// Recoverable or terminal stream error message for display.
    StreamError(String),
    /// Auto / manual compaction started.
    CompactionStarted,
    /// Compaction finished; `summary_chars` is the summary size for the status bar.
    ContextCompacted { summary_chars: usize },
    /// A tool invocation is about to run (or was scheduled).
    ToolStart {
        name: String,
        args: serde_json::Value,
    },
    /// A tool invocation finished (success or structured failure payload).
    ToolResult {
        name: String,
        args: serde_json::Value,
        content: String,
    },
    /// Background command task completed (non-subagent).
    BackgroundTaskResult(String),
    /// Plan panel update payload (`update_plan` tool).
    PlanUpdate(serde_json::Value),
    /// Persist the session (history checkpoint after a tool round / turn end).
    SaveSession,
}
