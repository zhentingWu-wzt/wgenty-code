//! Event types emitted by StreamProcessor during SSE streaming.

use crate::api::{ToolCall, Usage};

/// Events produced by StreamProcessor as it parses SSE chunks.
/// The frontend handles each event according to its rendering model.
pub enum StreamEvent {
    /// A content delta from the model (text being generated).
    ContentDelta(String),
    /// A reasoning_content delta (thinking, not shown to user).
    ReasoningDelta(String),
    /// A tool call delta fragment — accumulated by index.
    ToolCallDelta {
        index: usize,
        id: Option<String>,
        name: Option<String>,
        arguments: Option<String>,
    },
    /// Streaming completed with the given finish_reason.
    StreamDone { finish_reason: String },
    /// An error event from the daemon (e.g., network failure, API error).
    /// The frontend should display this to the user.
    StreamError(String),
}

/// The final result after streaming completes.
pub struct StreamResult {
    pub content: String,
    pub reasoning_content: String,
    pub tool_calls: Vec<ToolCall>,
    pub has_tool_calls: bool,
    pub finish_reason: String,
    /// Token usage reported by the API, if available (streaming with
    /// `stream_options.include_usage` or Anthropic MessageDelta).
    pub usage: Option<Usage>,
}
