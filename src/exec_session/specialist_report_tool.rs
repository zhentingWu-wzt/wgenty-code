//! Trusted tool boundary for specialist sub-agent handoffs.

use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

use crate::agent::{AgentCoordinator, ToolContext};
use crate::org_graph::{SpecialistEvidence, SpecialistReport, SpecialistReportKind};
use crate::tools::{Tool, ToolError, ToolOutput};

use super::ExecutionSessionRuntimeStore;

/// Model-supplied report contents. The producer is deliberately absent: it is
/// derived from the trusted child execution context before persistence.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SpecialistReportInput {
    kind: SpecialistReportKind,
    summary: String,
    evidence: Vec<SpecialistEvidence>,
    suspected_files: Vec<String>,
    recommended_actions: Vec<String>,
}

/// `submit_specialist_report` -- persist a typed handoff from a live
/// specialised sub-agent.
pub struct SubmitSpecialistReportTool {
    runtime_store: Arc<ExecutionSessionRuntimeStore>,
    coordinator: Arc<AgentCoordinator>,
}

impl SubmitSpecialistReportTool {
    /// Creates a context-scoped report tool backed by shared runtime state.
    pub fn new(
        runtime_store: Arc<ExecutionSessionRuntimeStore>,
        coordinator: Arc<AgentCoordinator>,
    ) -> Self {
        Self {
            runtime_store,
            coordinator,
        }
    }
}

#[async_trait]
impl Tool for SubmitSpecialistReportTool {
    fn name(&self) -> &str {
        "submit_specialist_report"
    }

    fn description(&self) -> &str {
        "Persist one structured, evidence-backed handoff report for the current \
specialist sub-agent. The runtime derives the report producer from the trusted \
agent context and rejects root, completed, or unauthorized callers."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "kind": {
                    "type": "string",
                    "enum": ["exploration", "root_cause", "implementation_plan"]
                },
                "summary": { "type": "string" },
                "evidence": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "path": { "type": "string" },
                            "detail": { "type": "string" }
                        },
                        "required": ["path", "detail"]
                    }
                },
                "suspected_files": {
                    "type": "array",
                    "items": { "type": "string" }
                },
                "recommended_actions": {
                    "type": "array",
                    "items": { "type": "string" }
                }
            },
            "required": [
                "kind",
                "summary",
                "evidence",
                "suspected_files",
                "recommended_actions"
            ],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        Err(ToolError {
            message: "specialist report submission requires a trusted tool context".into(),
            code: Some("missing_tool_context".into()),
        })
    }

    async fn execute_with_context(
        &self,
        context: &ToolContext<'_>,
        input: serde_json::Value,
    ) -> Result<ToolOutput, ToolError> {
        let input: SpecialistReportInput =
            serde_json::from_value(input).map_err(|error| ToolError {
                message: format!("invalid specialist report: {error}"),
                code: Some("invalid_input".into()),
            })?;
        let runtime_store = Arc::clone(&self.runtime_store);
        let session_id = context.agent.session_id.clone();
        self.coordinator
            .with_active_child_node_type(context.agent, move |producer| {
                let report = SpecialistReport {
                    producer: producer.clone(),
                    kind: input.kind,
                    summary: input.summary,
                    evidence: input.evidence,
                    suspected_files: input.suspected_files,
                    recommended_actions: input.recommended_actions,
                };
                runtime_store
                    .record_specialist_report(&session_id, producer, report)
                    .map_err(|error| ToolError {
                        message: format!("{error:#}"),
                        code: Some("specialist_report_rejected".into()),
                    })
            })
            .await
            .map_err(|error| ToolError {
                message: error.to_string(),
                code: Some("specialist_identity_rejected".into()),
            })??;

        Ok(ToolOutput {
            output_type: "text".into(),
            content: json!({ "status": "recorded" }).to_string(),
            metadata: std::collections::HashMap::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tempfile::TempDir;

    use super::SubmitSpecialistReportTool;
    use crate::agent::{
        AgentCoordinator, AgentExecutionContext, SessionId, SpawnChildRequest, ToolContext,
        ToolInvocationId,
    };
    use crate::exec_session::ExecutionSessionRuntimeStore;
    use crate::org_graph::{NodeType, SpecialistReportKind};
    use crate::tools::checkpoint_store::CheckpointStore;
    use crate::tools::Tool;

    fn test_store(directory: &TempDir) -> Arc<ExecutionSessionRuntimeStore> {
        Arc::new(ExecutionSessionRuntimeStore::new(
            directory.path().to_path_buf(),
            Arc::new(CheckpointStore::new(directory.path())),
            2,
        ))
    }

    fn context(agent: &AgentExecutionContext) -> ToolContext<'_> {
        ToolContext {
            agent,
            invocation_id: ToolInvocationId::new("specialist-report"),
            origin_turn_id: None,
            workdir: None,
            effective_mode: crate::sandbox::EffectiveMode::Normal,
            checkpoint: None,
        }
    }

    fn root_cause_input() -> serde_json::Value {
        serde_json::json!({
            "kind": "root_cause",
            "summary": "A null branch skips the guard.",
            "evidence": [{ "path": "src/guard.rs", "detail": "The branch returns before validation." }],
            "suspected_files": ["src/guard.rs"],
            "recommended_actions": ["Validate before branching."]
        })
    }

    #[tokio::test]
    async fn live_specialist_report_is_checkpointed_with_trusted_producer() {
        let directory = TempDir::new().expect("create temp directory");
        let store = test_store(&directory);
        let coordinator = Arc::new(AgentCoordinator::new(4, 2));
        let root = coordinator
            .ensure_root(SessionId::new("specialist-session"))
            .await
            .expect("create root");
        let child = coordinator
            .reserve_child(
                &root,
                SpawnChildRequest::new("diagnose").with_node_type(NodeType::RootCause),
            )
            .await
            .expect("reserve specialist");
        store
            .ensure_turn(&child.context.session_id)
            .expect("start graph turn");
        store
            .runtime_for(&child.context.session_id)
            .expect("resolve graph runtime")
            .begin_node("diagnose defect".into(), Vec::new(), Vec::new())
            .await
            .expect("start graph node");
        let tool = SubmitSpecialistReportTool::new(Arc::clone(&store), Arc::clone(&coordinator));

        let unknown_field_error = tool
            .execute_with_context(
                &context(&child.context),
                serde_json::json!({
                    "kind": "root_cause",
                    "summary": "A null branch skips the guard.",
                    "evidence": [{ "path": "src/guard.rs", "detail": "The branch returns before validation." }],
                    "suspected_files": ["src/guard.rs"],
                    "recommended_actions": ["Validate before branching."],
                    "producer": "general_purpose"
                }),
            )
            .await
            .expect_err("model-supplied identity field must be rejected");
        assert_eq!(unknown_field_error.code.as_deref(), Some("invalid_input"));

        let output = tool
            .execute_with_context(&context(&child.context), root_cause_input())
            .await
            .expect("submit trusted specialist report");

        assert!(output.content.contains("recorded"));
        let work_state = store.work_state_for_test(&child.context.session_id);
        let reports = work_state
            .specialist_reports(NodeType::GeneralPurpose)
            .expect("coordinator reads reports");
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].producer, NodeType::RootCause);
        assert_eq!(reports[0].kind, SpecialistReportKind::RootCause);

        let checkpointed_state = store.checkpointed_work_state_for_test(&child.context.session_id);
        assert_eq!(
            checkpointed_state
                .specialist_reports(NodeType::GeneralPurpose)
                .expect("coordinator reads checkpointed reports"),
            reports,
            "the report must survive checkpoint restore"
        );
    }

    #[tokio::test]
    async fn root_and_unauthorized_specialists_are_rejected() {
        let directory = TempDir::new().expect("create temp directory");
        let store = test_store(&directory);
        let coordinator = Arc::new(AgentCoordinator::new(4, 2));
        let root = coordinator
            .ensure_root(SessionId::new("specialist-session"))
            .await
            .expect("create root");
        store
            .ensure_turn(&root.session_id)
            .expect("start graph turn");
        store
            .runtime_for(&root.session_id)
            .expect("resolve graph runtime")
            .begin_node("diagnose defect".into(), Vec::new(), Vec::new())
            .await
            .expect("start graph node");
        let tool = SubmitSpecialistReportTool::new(Arc::clone(&store), Arc::clone(&coordinator));

        let root_error = tool
            .execute_with_context(&context(&root), root_cause_input())
            .await
            .expect_err("root must not impersonate a specialist");
        assert_eq!(
            root_error.code.as_deref(),
            Some("specialist_identity_rejected")
        );

        let verification = coordinator
            .reserve_child(
                &root,
                SpawnChildRequest::new("verify").with_node_type(NodeType::Verification),
            )
            .await
            .expect("reserve verification child");
        let verification_error = tool
            .execute_with_context(&context(&verification.context), root_cause_input())
            .await
            .expect_err("verification node lacks report write permission");
        assert_eq!(
            verification_error.code.as_deref(),
            Some("specialist_report_rejected")
        );
    }

    #[tokio::test]
    async fn finished_specialist_context_is_rejected() {
        let directory = TempDir::new().expect("create temp directory");
        let store = test_store(&directory);
        let coordinator = Arc::new(AgentCoordinator::new(4, 2));
        let root = coordinator
            .ensure_root(SessionId::new("finished-specialist-session"))
            .await
            .expect("create root");
        let child = coordinator
            .reserve_child(
                &root,
                SpawnChildRequest::new("diagnose").with_node_type(NodeType::RootCause),
            )
            .await
            .expect("reserve specialist");
        store
            .ensure_turn(&root.session_id)
            .expect("start graph turn");
        store
            .runtime_for(&root.session_id)
            .expect("resolve graph runtime")
            .begin_node("diagnose defect".into(), Vec::new(), Vec::new())
            .await
            .expect("start graph node");
        coordinator
            .finish_child(
                &child.context,
                crate::agent::ChildTerminal::completed("diagnosis complete"),
            )
            .await
            .expect("finish specialist");
        let tool = SubmitSpecialistReportTool::new(store, coordinator);

        let error = tool
            .execute_with_context(&context(&child.context), root_cause_input())
            .await
            .expect_err("finished specialist must not submit a report");
        assert_eq!(error.code.as_deref(), Some("specialist_identity_rejected"));
    }
}
