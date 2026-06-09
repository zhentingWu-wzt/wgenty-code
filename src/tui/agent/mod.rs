//! AgentLoop — the core agent loop: SSE streaming + tool execution + context compaction.
//! Port of TypeScript agent-loop.ts to Rust.
//!
//! Each Turn (one user input → final response) creates its own AgentLoop instance
//! backed by a *shared* conversation_history (Arc<Mutex<Vec<ChatMessage>>>).
//! This allows multiple user inputs to be queued while one is processing:
//! each pending input becomes a new AgentLoop that inherits the accumulated history.

mod core;
mod stream;
mod tool_dispatch;
mod compaction;

use crate::api::ChatMessage;
use crate::tui::app::AppEvent;
use crate::tui::client::DaemonClient;
use crate::utils::stuck_detector::StuckDetector;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

pub(super) const MAX_RETRIES: u32 = 2;
pub(super) const MAX_ESTIMATED_TOKENS: usize = 50_000;
// MAX_LLM_ROUNDS (100 default, configurable via settings.json) defined inside run_agent_loop as safety valve.

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
    pub(super) rounds_since_todo: usize,
    pub(super) compacted_summary: String,
    pub(super) preparing_tools_fired: bool,
    pub(super) max_rounds: usize,
    pub(super) stuck_detector: StuckDetector,
    pub(super) token_counter: crate::api::token_counter::TokenCounter,
    pub(super) plan_mode: bool,
    /// Optional ApiClient for a dedicated planner model. Used for plan
    /// generation when PlanMode is active and planner_model is configured.
    pub(super) planner_client: Option<crate::api::ApiClient>,
    pub(super) session_id: String,
}

impl AgentLoop {
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
    ) -> Self {
        Self {
            client,
            event_tx,
            conversation_history,
            assembled_system_messages: system_messages,
            rounds_since_todo: 0,
            compacted_summary: String::new(),
            preparing_tools_fired: false,
            max_rounds,
            stuck_detector: StuckDetector::new(),
            token_counter,
            session_id,
            plan_mode,
            planner_client,
        }
    }

    /// Process a single user input. Handles the full agent loop (SSE + tools).
    /// Returns Ok(()) on normal completion, Err if cancelled or timed out.
    pub async fn process_input(&mut self, input: String) -> Result<(), String> {
        const AGENT_LOOP_TIMEOUT: Duration = Duration::from_secs(3600);

        match tokio::time::timeout(AGENT_LOOP_TIMEOUT, self.process_input_inner(input)).await {
            Ok(result) => result,
            Err(_elapsed) => {
                let _ = self.event_tx.send(AppEvent::StreamError(format!(
                    "Agent loop timed out after {} minutes",
                    AGENT_LOOP_TIMEOUT.as_secs() / 60
                )));
                Err("Agent loop timed out".to_string())
            }
        }
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
    async fn process_input_inner(&mut self, input: String) -> Result<(), String> {
        self.inject_background_results().await;

        {
            let mut history = self.conversation_history.lock().await;
            history.push(ChatMessage::user(&input));
        }

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
