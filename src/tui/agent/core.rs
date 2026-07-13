//! TUI agent loop — thin facade over `agent::runtime::run_agent_loop`.

use super::adapters::{
    ApiPlannerPort, DaemonLlmPort, DaemonToolPort, TuiCompactor, TuiEventSink, TuiInteractionPort,
};
use super::{AgentError, AgentLoop};
use crate::agent::runtime::{
    run_agent_loop, LoopHooks, LoopTurnState, MutexHistoryStore, RunLoopArgs, RuntimeConfig,
    StreamStyle,
};
use std::sync::Arc;

impl AgentLoop {
    /// Core LLM loop: stream, handle tools, compaction, etc.
    ///
    /// Control flow lives in `agent::runtime::run_agent_loop`. This method only
    /// builds TUI ports (daemon LLM/tools, permission UI, auto-compactor) and
    /// maps `RuntimeError` → `AgentError`.
    pub(super) async fn run_agent_loop(&mut self) -> Result<(), AgentError> {
        let llm = DaemonLlmPort::new(self.client.clone());
        let events = TuiEventSink::new(self.event_tx.clone());
        let tools = DaemonToolPort::new(
            self.client.clone(),
            self.event_tx.clone(),
            self.hook_manager.clone(),
            self.session_id.clone(),
            self.subagent_timeout_secs,
            self.agent_generation,
        );
        let history = MutexHistoryStore::new(self.conversation_history.clone());
        let interaction = TuiInteractionPort::new(self.event_tx.clone());

        let compacted_summary = Arc::new(tokio::sync::Mutex::new(self.compacted_summary.clone()));
        let compactor = TuiCompactor::new(
            self.client.clone(),
            self.event_tx.clone(),
            self.assembled_system_messages.clone(),
            self.memory_manager.clone(),
            compacted_summary.clone(),
        );

        let planner = self
            .planner_client
            .clone()
            .map(ApiPlannerPort::new);

        let config = RuntimeConfig {
            max_rounds: self.max_rounds,
            plan_mode: self.plan_mode,
            subagent_timeout_secs: self.subagent_timeout_secs,
            context_window: self.context_window,
            max_tokens: self.max_tokens,
            session_id: self.session_id.clone(),
            turn_id: self.turn_id.clone(),
            agent_generation: self.agent_generation,
            stream_max_retries: super::MAX_RETRIES,
        };

        let mut state = LoopTurnState {
            compact_requested: self.compact_requested,
            compaction_failed: self.compaction_failed,
            preparing_tools_fired: self.preparing_tools_fired,
            rounds_since_plan: self.rounds_since_plan,
            compacted_summary: self.compacted_summary.clone(),
        };

        let planner_ref = planner.as_ref().map(|p| p as &dyn crate::agent::runtime::PlannerPort);

        let result = run_agent_loop(RunLoopArgs {
            llm: &llm,
            tools: &tools,
            events: &events,
            history: &history,
            config: &config,
            state: &mut state,
            stream_style: StreamStyle::tui_daemon(),
            hooks: LoopHooks {
                compactor: Some(&compactor),
                interaction: Some(&interaction),
                planner: planner_ref,
                stuck_detector: Some(&mut self.stuck_detector),
                token_counter: Some(&self.token_counter),
                synthesis: None,
                observer: None,
            },
        })
        .await;

        // Sync mutable flags back onto AgentLoop for the next turn / compact_only.
        self.compact_requested = state.compact_requested;
        self.compaction_failed = state.compaction_failed;
        self.preparing_tools_fired = state.preparing_tools_fired;
        self.rounds_since_plan = state.rounds_since_plan;
        self.compacted_summary = compacted_summary.lock().await.clone();
        if self.compacted_summary.is_empty() {
            self.compacted_summary = state.compacted_summary;
        }

        result.map(|_| ()).map_err(AgentError::from)
    }
}
