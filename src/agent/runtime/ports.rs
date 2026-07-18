//! Port traits that isolate the agent loop from frontends and I/O backends.

use super::error::RuntimeError;
use super::events::RuntimeEvent;
use crate::api::{ChatMessage, ToolDefinition, Usage};
use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::BoxStream;

/// Full non-streaming completion (subagent path uses this).
#[derive(Debug, Clone)]
pub struct ChatCompletion {
    pub message: ChatMessage,
    pub finish_reason: String,
    pub usage: Option<Usage>,
}

/// Opens LLM chat streams (and optional non-stream chat) for the agent loop.
#[async_trait]
pub trait LlmPort: Send + Sync {
    /// Open a streaming chat completion. Each item is a raw body chunk.
    ///
    /// `tools` is the model-facing tool list. Daemon-backed ports may ignore
    /// this when the daemon injects tools server-side; in-process ports must
    /// forward it to the provider.
    async fn open_chat_stream(
        &self,
        messages: Vec<ChatMessage>,
        tools: Option<Vec<ToolDefinition>>,
        max_tokens: Option<usize>,
        plan_mode: Option<bool>,
    ) -> Result<BoxStream<'static, Result<Bytes, RuntimeError>>, RuntimeError>;

    /// Non-streaming completion (subagent / planner). Default: unsupported.
    async fn chat_completion(
        &self,
        _messages: Vec<ChatMessage>,
        _tools: Option<Vec<ToolDefinition>>,
    ) -> Result<ChatCompletion, RuntimeError> {
        Err(RuntimeError::Stream(
            "non-streaming chat is not supported by this LlmPort".into(),
        ))
    }

    /// Convenience: non-streaming message only (planner).
    async fn chat(
        &self,
        messages: Vec<ChatMessage>,
        tools: Option<Vec<ToolDefinition>>,
    ) -> Result<ChatMessage, RuntimeError> {
        Ok(self.chat_completion(messages, tools).await?.message)
    }
}

/// Receives runtime events for UI / logging.
pub trait EventSink: Send + Sync {
    fn emit(&self, event: RuntimeEvent);
}

/// Conversation history storage used by compaction and the loop.
#[async_trait]
pub trait HistoryStore: Send + Sync {
    async fn get(&self) -> Vec<ChatMessage>;
    async fn replace(&self, messages: Vec<ChatMessage>);
    async fn push(&self, message: ChatMessage);
}

/// Tool execution backend (daemon HTTP or in-process registry).
#[async_trait]
pub trait ToolPort: Send + Sync {
    /// Execute a tool and return a model-facing result string (usually JSON).
    async fn execute(&self, req: ToolRequest) -> ToolResponse;

    /// Tool definitions advertised to the model (may be empty when a daemon
    /// injects tools server-side).
    fn definitions(&self) -> Vec<ToolDefinition>;
}

/// Optional auto-compaction (transcript archive + LLM summary).
///
/// Micro-compaction is pure and always applied by the loop; this port covers
/// the expensive summarization path.
///
/// The compact implementation must **not** mutate `history`. It archives the
/// transcript, asks the LLM for a summary, and returns `Some(summary)` on
/// success. The caller assembles the API-facing context view from the summary
/// plus a boundary into the *full, preserved* history - so the on-disk session
/// always keeps the original messages verbatim.
#[async_trait]
pub trait Compactor: Send + Sync {
    /// Run a compaction pass. Returns `Some(summary)` on success, `None` on
    /// failure. Never rewrites the stored history.
    async fn compact(&self, history: &dyn HistoryStore) -> Option<String>;
}

/// Interactive prompts (`ask_user_question`) for frontends that can show UI.
#[async_trait]
pub trait InteractionPort: Send + Sync {
    async fn ask_user_question(&self, args: &serde_json::Value) -> String;
}

/// Dedicated planner model used when plan mode is active.
#[async_trait]
pub trait PlannerPort: Send + Sync {
    async fn plan(&self, messages: &[ChatMessage]) -> Result<String, String>;
}

/// Non-root subagent synthesis barrier (collect child results before finalize).
#[async_trait]
pub trait SynthesisPort: Send + Sync {
    /// Called when the model returns a final candidate (no tool calls).
    ///
    /// - `Ok(None)` — accept the candidate and finish the loop.
    /// - `Ok(Some(msg))` — inject the message as a `user` turn and continue another round.
    /// - `Err(_)` — fail the loop.
    async fn on_candidate_final(&self, candidate: &str) -> Result<Option<String>, RuntimeError>;
}

/// Per-round progress observer (subagent status bar / action log).
pub trait RoundObserver: Send + Sync {
    fn on_round_start(&self, round: usize, messages: &[ChatMessage]);
    fn on_tool_start(&self, round: usize, tool_name: &str, messages: &[ChatMessage]);
    fn on_completed(&self, round: usize, messages: &[ChatMessage]);
    fn on_failed(&self, round: usize, error: &str, messages: &[ChatMessage]);
    /// Optional: API-reported token usage for this round.
    fn on_usage(&self, _total_tokens: usize) {}
}

/// Optional task-board progress for the agent loop to surface reminders.
///
/// Implementations (daemon/CLI holding a `TaskManagementTool`) expose how many
/// tasks are blocked vs ready so the loop can inject a gentle nudge when the
/// model goes several rounds without acting on a ready task. Async because the
/// TUI path fetches counts over HTTP from the daemon.
#[async_trait]
pub trait TaskProgressPort: Send + Sync {
    /// `(blocked_count, ready_count)` excluding completed/deleted tasks.
    async fn blocked_and_ready(&self) -> (usize, usize);
}

/// Optional async inbox drain run at the top of each loop round (s09 mailbox).
///
/// Returns a system message to inject into history before the LLM call, or
/// `None` when the inbox is empty. Async so implementors can do file/HTTP I/O
/// (JSONL mailbox, daemon endpoint) without blocking the runtime.
#[async_trait]
pub trait InboxPort: Send + Sync {
    /// Drain pending inbound messages; return a system message to inject, if any.
    async fn drain(&self) -> Option<String>;
}

/// Input to [`ToolPort::execute`].
#[derive(Debug, Clone)]
pub struct ToolRequest {
    pub name: String,
    pub arguments: serde_json::Value,
    pub session_id: String,
    pub turn_id: Option<String>,
    /// Model-issued tool_call id (used as trusted invocation id when present).
    pub invocation_id: Option<String>,
    /// When true, the port must not block on interactive permission prompts.
    pub parallel: bool,
}

/// Structured tool outcome before it is stringified into a `role=tool` message.
#[derive(Debug, Clone)]
pub struct ToolResponse {
    pub content: String,
    pub success: bool,
}
