//! RLM Delegate Tool — Recursive Language Model delegation.
//!
//! The `delegate` tool implements a Planner → Executor → Aggregator pipeline
//! for complex tasks. The parent agent delegates a complex task, the RLM
//! system automatically:
//!   1. Decomposes the task into independent sub-tasks (Planner)
//!   2. Executes sub-tasks in parallel via subagent loops (Executor)
//!   3. Merges results into a coherent response (Aggregator)
//!
//! This coexists with the existing `task` tool: simple tasks use `task` directly,
//! complex tasks use `delegate` to trigger the full RLM pipeline.

pub mod budget;
pub mod formats;
mod pipeline;

pub use pipeline::{extract_json, run_rlm_pipeline, RlmResult};

use crate::agent::progress::{SubagentProgress, SubagentStatus};
use crate::agent::{AgentCoordinator, ToolContext};
use crate::config::Settings;
use crate::tools::{Tool, ToolError, ToolOutput, ToolRegistry};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct RlmDelegateTool {
    settings: Settings,
    tool_registry: std::sync::Weak<ToolRegistry>,
    coordinator: Arc<AgentCoordinator>,
    progress_store: Arc<RwLock<HashMap<String, HashMap<String, SubagentProgress>>>>,
}

impl RlmDelegateTool {
    pub fn new(
        settings: Settings,
        tool_registry: std::sync::Weak<ToolRegistry>,
        coordinator: Arc<AgentCoordinator>,
        progress_store: Arc<RwLock<HashMap<String, HashMap<String, SubagentProgress>>>>,
    ) -> Self {
        Self {
            settings,
            tool_registry,
            coordinator,
            progress_store,
        }
    }
}

#[async_trait]
impl Tool for RlmDelegateTool {
    fn name(&self) -> &str {
        "delegate"
    }

    fn is_read_only(&self) -> bool {
        false
    }

    fn description(&self) -> &str {
        "Delegate a complex task to the RLM system for automatic decomposition, parallel execution, and result aggregation. Use for complex, multi-step tasks that benefit from structured planning and parallel sub-agent execution. Simple tasks should use the `task` tool instead."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "task": {
                    "type": "string",
                    "description": "The complex task to delegate — describe what needs to be done, including context and constraints"
                },
                "context": {
                    "type": "string",
                    "description": "Optional context or reference information to help the planner decompose the task effectively"
                }
            },
            "required": ["task"]
        })
    }

    async fn execute(&self, _input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        Err(ToolError {
            message: "delegate requires trusted agent context".to_string(),
            code: Some("missing_agent_context".to_string()),
        })
    }

    async fn execute_with_context(
        &self,
        context: &ToolContext<'_>,
        input: serde_json::Value,
    ) -> Result<ToolOutput, ToolError> {
        let task = input["task"].as_str().unwrap_or("");
        let context_str = input["context"].as_str().unwrap_or("");
        // Trusted session identity from the execution context; `_session_id`
        // in input is ignored.
        let session_id = context.agent.session_id.as_str().to_string();

        let tool_registry = self.tool_registry.upgrade().ok_or_else(|| ToolError {
            message: "Tool registry is no longer available".to_string(),
            code: Some("registry_dropped".to_string()),
        })?;

        // Register root node in progress store.
        let root_node_id = uuid::Uuid::new_v4().to_string();
        {
            let mut store = self.progress_store.write().await;
            store.entry(session_id.clone()).or_default().insert(
                root_node_id.clone(),
                SubagentProgress {
                    node_id: root_node_id.clone(),
                    parent_id: None,
                    label: format!("delegate: {}", task),
                    status: SubagentStatus::Running,
                    round: None,
                    max_rounds: None,
                    current_tool: None,
                    current_params: None,
                    action_log: Vec::new(),
                    text_snapshot: None,
                    started_at: chrono::Utc::now().timestamp_millis(),
                    elapsed_ms: 0,
                    metadata: None,
                    progress_delta: None,
                    token_budget_k: None,
                    cumulative_tokens: 0,
                    error_details: None,
                    events: Vec::new(),
                    messages: Vec::new(),
                },
            );
        }
        let result = run_rlm_pipeline(
            &self.settings,
            tool_registry,
            self.coordinator.clone(),
            context.agent,
            task,
            context_str,
            Some((self.progress_store.clone(), session_id)),
            Some(root_node_id),
            None,
            None,
        )
        .await
        .map_err(|e| ToolError {
            message: e.clone(),
            code: Some("rlm_pipeline_error".to_string()),
        })?;

        Ok(ToolOutput {
            output_type: "text".to_string(),
            content: result.aggregated,
            metadata: {
                let mut m = HashMap::new();
                m.insert(
                    "sub_task_count".to_string(),
                    serde_json::json!(result.sub_task_count),
                );
                m.insert("completed".to_string(), serde_json::json!(result.completed));
                m.insert("failed".to_string(), serde_json::json!(result.failed));
                m
            },
        })
    }
}
