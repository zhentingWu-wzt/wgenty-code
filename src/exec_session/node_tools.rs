//! Agent-facing tools for the node-level state machine.
//!
//! These tools wrap [`NodeRuntime`] methods and implement the [`Tool`] trait
//! so the agent can drive node lifecycle via tool calls. Each tool holds an
//! `Arc<NodeRuntime>` (shared with the agent loop).

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::tools::{Tool, ToolError, ToolOutput};

use super::node_runtime::{NodeRollbackResult, NodeRuntime, NodeVerificationOutcome};

/// `begin_node` -- start a new verifiable work unit.
pub struct BeginNodeTool {
    runtime: Arc<NodeRuntime>,
}

impl BeginNodeTool {
    pub fn new(runtime: Arc<NodeRuntime>) -> Self {
        Self { runtime }
    }
}

#[async_trait]
impl Tool for BeginNodeTool {
    fn name(&self) -> &str {
        "begin_node"
    }

    fn description(&self) -> &str {
        "Start a new verifiable work unit (node) with a goal, verify commands, \
and expected changed files. The current node must be Verified or absent. \
The runtime records the current turn as the node's start point for later \
verify scope and rollback."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "goal": {
                    "type": "string",
                    "description": "Human-readable goal for this work unit."
                },
                "verify_commands": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Supplemental final verification commands run after runtime-owned verification anchors (e.g. cargo test --doc)."
                },
                "expected_files": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Files expected to change within this node. Empty = no boundary check.",
                    "default": []
                }
            },
            "required": ["goal", "verify_commands"]
        })
    }

    // is_read_only defaults to false.

    async fn execute(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let goal = input
            .get("goal")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError {
                message: "missing or invalid 'goal' field".into(),
                code: Some("invalid_input".into()),
            })?
            .to_string();
        let verify_commands = input
            .get("verify_commands")
            .and_then(|v| v.as_array())
            .ok_or_else(|| ToolError {
                message: "missing or invalid 'verify_commands' field".into(),
                code: Some("invalid_input".into()),
            })?
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect::<Vec<_>>();
        let expected_files = input
            .get("expected_files")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let node_id = self
            .runtime
            .begin_node_with_anchors(
                goal,
                Vec::new(),
                Vec::new(),
                verify_commands,
                expected_files,
            )
            .await
            .map_err(|e| ToolError {
                message: format!("{e:#}"),
                code: Some("begin_node_failed".into()),
            })?;

        Ok(ToolOutput {
            output_type: "text".into(),
            content: json!({
                "node_id": node_id,
                "status": "running"
            })
            .to_string(),
            metadata: std::collections::HashMap::new(),
        })
    }
}

/// `verify_node` -- verify the current node.
pub struct VerifyNodeTool {
    runtime: Arc<NodeRuntime>,
}

impl VerifyNodeTool {
    pub fn new(runtime: Arc<NodeRuntime>) -> Self {
        Self { runtime }
    }
}

#[async_trait]
impl Tool for VerifyNodeTool {
    fn name(&self) -> &str {
        "verify_node"
    }

    fn description(&self) -> &str {
        "Verify the current node by executing its verify commands and checking \
for out-of-bounds changes. On success the node transitions to Verified. \
On failure the node transitions to Failed and the failure reason is returned \
for self-correction. After exceeding the retry budget, the session is marked \
Failed."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn execute(&self, _input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let outcome = self
            .runtime
            .verify_current_node()
            .await
            .map_err(|e| ToolError {
                message: format!("{e:#}"),
                code: Some("verify_node_failed".into()),
            })?;

        if let NodeVerificationOutcome::WorkGraph(result) = outcome {
            let mut metadata = std::collections::HashMap::new();
            metadata.insert("next_step".into(), json!(result.next_step));
            return Ok(ToolOutput {
                output_type: "text".into(),
                content: json!({ "next_step": result.next_step }).to_string(),
                metadata,
            });
        }
        let NodeVerificationOutcome::Legacy(result) = outcome else {
            unreachable!("work graph returned above");
        };

        let mut metadata = std::collections::HashMap::new();
        metadata.insert("status".into(), json!(result.status));
        metadata.insert("retry_count".into(), json!(result.retry_count));

        Ok(ToolOutput {
            output_type: "text".into(),
            content: json!({
                "status": result.status,
                "retry_count": result.retry_count,
                "failure_reason": result.failure_reason
            })
            .to_string(),
            metadata,
        })
    }
}

/// `rollback_node` -- roll back to the most recent verified node.
pub struct RollbackNodeTool {
    runtime: Arc<NodeRuntime>,
}

impl RollbackNodeTool {
    pub fn new(runtime: Arc<NodeRuntime>) -> Self {
        Self { runtime }
    }
}

#[async_trait]
impl Tool for RollbackNodeTool {
    fn name(&self) -> &str {
        "rollback_node"
    }

    fn description(&self) -> &str {
        "Roll back to the most recent Verified node, removing all nodes after \
it and restoring the workspace to that node's state. Requires at least one \
Verified node."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn execute(&self, _input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let result: NodeRollbackResult =
            self.runtime.rollback_node().await.map_err(|e| ToolError {
                message: format!("{e:#}"),
                code: Some("rollback_node_failed".into()),
            })?;

        Ok(ToolOutput {
            output_type: "text".into(),
            content: json!({
                "rolled_back_to": result.rolled_back_to,
                "removed_nodes": result.removed_nodes
            })
            .to_string(),
            metadata: std::collections::HashMap::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::{Arc, Mutex, RwLock};

    use anyhow::Result;
    use async_trait::async_trait;
    use tempfile::TempDir;

    use super::*;
    use crate::exec_session::{
        CommandExecutor, CommandRun, NodeStatus, SessionCoordinator, SessionSource, VerifyGate,
    };
    use crate::tools::checkpoint_store::CheckpointStore;

    struct RecordingExecutor {
        calls: Arc<Mutex<Vec<String>>>,
    }

    struct FailingExecutor;

    #[async_trait]
    impl CommandExecutor for RecordingExecutor {
        async fn execute(&self, command: &str, _project_root: &Path) -> Result<CommandRun> {
            self.calls.lock().expect("calls lock").push(command.into());
            Ok(CommandRun {
                cmd: command.into(),
                exit_code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
            })
        }
    }

    #[async_trait]
    impl CommandExecutor for FailingExecutor {
        async fn execute(&self, command: &str, _project_root: &Path) -> Result<CommandRun> {
            Ok(CommandRun {
                cmd: command.into(),
                exit_code: Some(1),
                stdout: String::new(),
                stderr: "command failed".into(),
            })
        }
    }

    #[tokio::test]
    async fn node_tools_use_rust_profile_anchors_in_order() {
        let directory = TempDir::new().expect("temp directory");
        std::fs::write(
            directory.path().join("Cargo.toml"),
            "[package]\nname = \"p\"\n",
        )
        .expect("write manifest");
        let coordinator = Arc::new(RwLock::new(
            SessionCoordinator::new(
                "tool-e2e".into(),
                SessionSource::AgentSelf,
                directory.path(),
                Arc::new(CheckpointStore::new(directory.path())),
            )
            .expect("session coordinator"),
        ));
        coordinator
            .write()
            .expect("coordinator write lock")
            .begin_turn()
            .expect("begin turn");
        let calls = Arc::new(Mutex::new(Vec::new()));
        let gate = Arc::new(VerifyGate::new_with_default_hooks(
            Arc::clone(&coordinator),
            Arc::new(RecordingExecutor {
                calls: Arc::clone(&calls),
            }),
        ));
        let runtime = Arc::new(NodeRuntime::new_with_default_hooks(
            coordinator.clone(),
            gate,
            2,
        ));
        let begin = BeginNodeTool::new(Arc::clone(&runtime));
        let verify = VerifyNodeTool::new(runtime);

        let schema = begin.input_schema();
        assert!(schema["properties"].get("compile_commands").is_none());
        assert!(schema["properties"].get("test_commands").is_none());

        begin
            .execute(json!({
                "goal": "tool e2e",
                "verify_commands": ["cargo test --doc"],
                "expected_files": []
            }))
            .await
            .expect("begin node tool");
        let output = verify.execute(json!({})).await.expect("verify node tool");

        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&output.content).expect("structured output")
                ["next_step"],
            "Complete"
        );
        assert_eq!(
            calls.lock().expect("calls lock").as_slice(),
            [
                "cargo check",
                "cargo test --all",
                "cargo clippy --all-targets -- -D warnings",
                "cargo test --doc",
            ]
        );
        assert_eq!(
            coordinator
                .read()
                .expect("coordinator read lock")
                .current_node()
                .expect("current node")
                .status,
            NodeStatus::Verified
        );
    }

    #[tokio::test]
    async fn verify_node_tool_marks_node_failed_when_work_graph_escalates() {
        let directory = TempDir::new().expect("temp directory");
        std::fs::write(
            directory.path().join("Cargo.toml"),
            "[package]\nname = \"p\"\n",
        )
        .expect("write manifest");
        let coordinator = Arc::new(RwLock::new(
            SessionCoordinator::new(
                "tool-escalation".into(),
                SessionSource::AgentSelf,
                directory.path(),
                Arc::new(CheckpointStore::new(directory.path())),
            )
            .expect("session coordinator"),
        ));
        coordinator
            .write()
            .expect("coordinator write lock")
            .begin_turn()
            .expect("begin turn");
        let gate = Arc::new(VerifyGate::new_with_default_hooks(
            Arc::clone(&coordinator),
            Arc::new(FailingExecutor),
        ));
        let runtime = Arc::new(NodeRuntime::new_with_default_hooks(
            Arc::clone(&coordinator),
            gate,
            1,
        ));
        BeginNodeTool::new(Arc::clone(&runtime))
            .execute(json!({
                "goal": "tool escalation",
                "verify_commands": [],
                "expected_files": []
            }))
            .await
            .expect("begin node tool");

        let output = VerifyNodeTool::new(runtime)
            .execute(json!({}))
            .await
            .expect("verify node tool");

        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&output.content).expect("structured output")
                ["next_step"],
            "Escalate"
        );
        assert_eq!(
            coordinator
                .read()
                .expect("coordinator")
                .current_node()
                .expect("current node")
                .status,
            NodeStatus::Failed
        );
    }
}
