//! Shared agent runtime primitives.
//!
//! Phase A: pure helpers (compaction, tool timeouts).
//! Phase B+: ports, stream engine, multi-round [`run_agent_loop`] used by
//! both TUI and headless CLI.

pub mod adapters;
pub mod compaction;
pub mod compactor;
pub mod config;
pub mod error;
pub mod events;
pub mod history;
pub mod loop_;
pub mod ports;
pub mod stream;
pub mod timeout;

#[cfg(test)]
mod loop_tests;

pub use adapters::ApiLlmPort;
pub use compaction::{
    assemble_post_compaction_history, micro_compact_messages, needs_compaction, request_size_chars,
    split_for_compaction,
};
pub use compactor::{
    archive_transcript, build_transcript_text, fallback_micro_compact, is_payload_too_large_error,
    parse_compaction_response, prepare_compaction_transcript, truncate_transcript_text,
    ApiCompactor, COMPACTION_SYSTEM_PROMPT, COMPACTION_TRANSCRIPT_CHAR_CAP,
};
pub use config::RuntimeConfig;
pub use error::RuntimeError;
pub use events::RuntimeEvent;
pub use history::MutexHistoryStore;
pub use loop_::{run_agent_loop, LoopHooks, LoopTurnState, RunLoopArgs, StreamStyle};
pub use ports::{
    ChatCompletion, Compactor, EventSink, HistoryStore, InboxPort, InteractionPort, LlmPort,
    PlannerPort, RoundObserver, SynthesisPort, TaskProgressPort, ToolPort, ToolRequest,
    ToolResponse,
};
pub use stream::{stream_response, stream_with_retry, StreamRetryOpts, STREAM_IDLE_TIMEOUT};
pub use timeout::resolve_tool_timeout;
