//! AgentLoop — the core agent loop: SSE streaming + tool execution + context compaction.
//! Port of TypeScript agent-loop.ts to Rust.
//!
//! Each Turn (one user input → final response) creates its own AgentLoop instance
//! backed by a *shared* conversation_history (Arc<Mutex<Vec<ChatMessage>>>).
//! This allows multiple user inputs to be queued while one is processing:
//! each pending input becomes a new AgentLoop that inherits the accumulated history.

mod compaction;
mod core;
mod stream;
mod tool_dispatch;

use crate::api::ChatMessage;
use crate::runtime::hooks::HookManager;
use crate::tui::app::AppEvent;
use crate::tui::client::DaemonClient;
use crate::utils::stuck_detector::StuckDetector;
use std::sync::Arc;
use tokio::sync::mpsc;

pub(super) const MAX_RETRIES: u32 = 2;
// MAX_LLM_ROUNDS (100 default, configurable via settings.json) defined inside run_agent_loop as safety valve.

/// Structured error returned by the agent loop.
///
/// Replaces bare `String` errors so callers match on variants instead of
/// fragile substring checks (e.g. `e.contains("timed out")`).
#[derive(Debug, Clone, thiserror::Error)]
pub enum AgentError {
    /// Agent exceeded the configured max LLM rounds.
    #[error("Exceeded {max_rounds} LLM rounds")]
    MaxRoundsExceeded { max_rounds: usize },
    /// Stream timed out (connection timeout or idle stall).
    #[error("{0}")]
    StreamTimeout(String),
    /// Unrecoverable stream / API error.
    #[error("{0}")]
    StreamError(String),
    /// Dedicated planner model call failed.
    #[error("Planner model call failed: {0}")]
    PlannerError(String),
    /// API returned a completely empty response.
    #[error("Empty response from API")]
    EmptyResponse,
}

pub struct AgentLoop {
    pub(super) client: DaemonClient,
    pub(super) event_tx: mpsc::UnboundedSender<AppEvent>,
    /// Shared conversation history across all Turns in this session.
    /// Each AgentLoop instance reads/writes through this Arc, so the
    /// accumulated context is inherited by the next Turn in the queue.
    pub(super) conversation_history: Arc<tokio::sync::Mutex<Vec<ChatMessage>>>,
    /// Pre-assembled system messages (layered instructions from PromptAssembler).
    /// Used when initializing or resetting the conversation history.
    pub(super) assembled_system_messages: Vec<ChatMessage>,
    pub(super) rounds_since_plan: usize,
    pub(super) compacted_summary: String,
    /// Set by the `compact` tool to request compaction at the next loop-top
    /// check. Compaction runs at the loop top (not mid-tool-batch) so it never
    /// wipes the in-flight assistant tool_calls message — which would orphan
    /// the subsequent tool results and make Ark reject the next request with
    /// `InvalidParameter`.
    pub(super) compact_requested: bool,
    /// Set when a compaction attempt failed during this turn. Prevents
    /// unbounded retries: `do_auto_compact` returns false on failure, and the
    /// loop falls through to a normal request instead of spinning. Reset at
    /// the start of each turn.
    pub(super) compaction_failed: bool,
    pub(super) preparing_tools_fired: bool,
    pub(super) max_rounds: usize,
    pub(super) stuck_detector: StuckDetector,
    pub(super) token_counter: crate::api::token_counter::TokenCounter,
    pub(super) plan_mode: bool,
    /// Optional ApiClient for a dedicated planner model. Used for plan
    /// generation when PlanMode is active and planner_model is configured.
    pub(super) planner_client: Option<crate::api::ApiClient>,
    pub(super) session_id: String,
    /// Hook manager shared with the App for lifecycle event hooks.
    pub(super) hook_manager: std::sync::Arc<HookManager>,
    /// Prompt context for building per-turn `<system-reminder>` blocks.
    pub(super) prompt_context: std::sync::Arc<crate::prompts::PromptContext>,
    /// Subagent timeout from settings.agent.subagent.timeout_secs (default 1800).
    pub(super) subagent_timeout_secs: u64,
    /// Model context window from settings.models.context_window (default 200_000).
    pub(super) context_window: usize,
    /// Max output tokens from settings.models.transport.max_tokens. Compaction
    /// reserves this much of the context window for the model's output before
    /// applying the 80 % threshold, so input + max_tokens never overflows the
    /// window (which would trigger InvalidParameter on the input side).
    pub(super) max_tokens: usize,
    /// Memory manager for cross-session memory extraction and recall.
    pub(super) memory_manager: Arc<crate::context::MemoryManager>,
}

impl AgentLoop {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        client: DaemonClient,
        event_tx: mpsc::UnboundedSender<AppEvent>,
        session_id: String,
        conversation_history: Arc<tokio::sync::Mutex<Vec<ChatMessage>>>,
        system_messages: Vec<ChatMessage>,
        plan_mode: bool,
        planner_client: Option<crate::api::ApiClient>,
        max_rounds: usize,
        token_counter: crate::api::token_counter::TokenCounter,
        hook_manager: std::sync::Arc<HookManager>,
        prompt_context: std::sync::Arc<crate::prompts::PromptContext>,
        subagent_timeout_secs: u64,
        context_window: usize,
        max_tokens: usize,
        memory_manager: Arc<crate::context::MemoryManager>,
    ) -> Self {
        Self {
            client,
            event_tx,
            conversation_history,
            assembled_system_messages: system_messages,
            rounds_since_plan: 0,
            compacted_summary: String::new(),
            compact_requested: false,
            compaction_failed: false,
            preparing_tools_fired: false,
            max_rounds,
            stuck_detector: StuckDetector::new(),
            token_counter,
            session_id,
            plan_mode,
            planner_client,
            hook_manager,
            prompt_context,
            subagent_timeout_secs,
            context_window,
            max_tokens,
            memory_manager,
        }
    }

    /// Process a single user input. Handles the full agent loop (SSE + tools).
    /// Returns Ok(()) on normal completion, Err if cancelled or on error.
    pub async fn process_input(&mut self, input: String) -> Result<(), AgentError> {
        self.token_counter.reset_turn();
        // Each turn gets a fresh compaction attempt — a prior failure may have
        // been transient (network blip, reasoning model quirks).
        self.compaction_failed = false;
        self.process_input_inner(input).await
    }

    /// Run a single compaction pass without generating an LLM response.
    /// Used by the `/compact` slash command for user-initiated compaction:
    /// archives the transcript and replaces history with a summary. The
    /// `CompactionStarted` / `ContextCompacted` events fired inside
    /// `do_auto_compact` drive the UI status bar.
    pub async fn compact_only(&mut self) -> Result<(), AgentError> {
        self.token_counter.reset_turn();
        self.compaction_failed = false;
        // Guard: nothing meaningful to summarize (only system messages).
        let has_content = {
            let history = self.conversation_history.lock().await;
            history.iter().any(|m| m.role != "system")
        };
        if !has_content {
            let _ = self.event_tx.send(AppEvent::StreamError(
                "Nothing to compact — the conversation is empty.".to_string(),
            ));
            return Ok(());
        }
        let _ = self.do_auto_compact().await;
        Ok(())
    }

    /// Generate a plan using a dedicated planner model (non-streaming).
    /// Returns the plan text or an error message.
    pub(super) async fn plan_with_model(
        &self,
        planner: &crate::api::ApiClient,
        messages: &[ChatMessage],
    ) -> Result<String, String> {
        let response = planner
            .chat(messages.to_vec(), None)
            .await
            .map_err(|e| format!("Planner model call failed: {}", e))?;
        Ok(response
            .choices
            .first()
            .map(|c| c.message.content.clone().unwrap_or_default())
            .unwrap_or_default())
    }

    /// Inner implementation of the agent loop.
    async fn process_input_inner(&mut self, input: String) -> Result<(), AgentError> {
        self.inject_background_results().await;

        // 1a. Fire UserPromptSubmit hook synchronously with a 10s timeout.
        //     On timeout the turn continues with empty outcomes (graceful degradation).
        let outcomes = {
            let hook_ctx = crate::runtime::hooks::HookContext {
                event: "UserPromptSubmit".to_string(),
                tool_name: None,
                tool_input: Some(serde_json::Value::String(input.clone())),
                tool_result: None,
                session_id: Some(self.session_id.clone()),
                working_directory: std::env::current_dir()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default(),
                timestamp: chrono::Utc::now().to_rfc3339(),
                comet_phase: None,
                workflow_state: None,
                variables: Default::default(),
            };
            match tokio::time::timeout(
                std::time::Duration::from_secs(10),
                self.hook_manager.fire(
                    &crate::runtime::hooks::HookEvent::UserPromptSubmit,
                    &hook_ctx,
                    None,
                    None,
                ),
            )
            .await
            {
                Ok(v) => v,
                Err(_) => {
                    tracing::warn!(
                        "UserPromptSubmit hook timed out after 10s; proceeding with empty outcomes"
                    );
                    Vec::new()
                }
            }
        };

        // 1b. Collect injected fragments from hook outcomes.
        let injections = crate::runtime::hooks::collect_injections(&outcomes);

        // 2. Build per-turn `<system-reminder>` block from file sources + hook injections.
        let reminder =
            crate::prompts::build_user_turn_reminder(self.prompt_context.as_ref(), &injections);

        // 3. Assemble user message content: reminder (if any) prepended to user input.
        let user_content = match &reminder {
            Some(r) => format!("{}\n\n{}", r.to_model, input),
            None => input.clone(),
        };

        // 4. Push to history with token estimate.
        {
            let mut history = self.conversation_history.lock().await;
            let input_tokens = user_content.len() / 4;
            self.token_counter.add_input(input_tokens);
            history.push(ChatMessage::user(&user_content));
        }

        // TODO(§4+): deliver reminder.to_transcript via a new AppEvent::SystemNotice
        // channel so the TUI shows the user-visible portion. The model already sees
        // the full reminder via to_model; visibility filtering already works for the
        // model-facing path.

        self.run_agent_loop().await
    }

    // ── Session state ────────────────────────────────────────────────────

    pub async fn load_history(&self, messages: Vec<ChatMessage>) {
        let mut history = self.conversation_history.lock().await;
        *history = messages;
    }

    pub async fn get_history(&self) -> Vec<ChatMessage> {
        self.conversation_history.lock().await.clone()
    }

    pub async fn reset(&self) {
        let mut history = self.conversation_history.lock().await;
        *history = self.assembled_system_messages.clone();
    }
}

// System prompt is now assembled via crate::prompts::assemble_instructions()

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::MemoryManager;
    use std::sync::Arc;

    /// Verify that AgentLoop can be constructed with a memory_manager field.
    /// This is a compile-time and structural test: if the field or constructor
    /// parameter is missing, the call to AgentLoop::new below will fail to
    /// compile, and the field access at the end will also fail.
    #[test]
    fn agent_loop_accepts_memory_manager() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let client = DaemonClient::new("http://localhost:8080".to_string());
        let mm = Arc::new(MemoryManager::new());

        // Construct AgentLoop with memory_manager — must compile.
        let loop_instance = AgentLoop::new(
            client,
            tx,
            "test-session".into(),
            Arc::new(tokio::sync::Mutex::new(vec![])),
            vec![],
            false,
            None,
            100,
            crate::api::token_counter::TokenCounter::new(),
            std::sync::Arc::new(HookManager::default()),
            std::sync::Arc::new(crate::prompts::PromptContext::new()),
            1800,
            200_000,
            65536,
            mm.clone(),
        );

        // Verify the field is accessible and is of the correct type.
        let _field: &Arc<MemoryManager> = &loop_instance.memory_manager;
        assert!(Arc::ptr_eq(&mm, &loop_instance.memory_manager));
    }
}
