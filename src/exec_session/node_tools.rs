//! Agent-facing tools for the node-level state machine.
//!
//! These tools wrap [`NodeRuntime`] methods and implement the [`Tool`] trait
//! so the agent can drive node lifecycle via tool calls. Each tool holds an
//! `Arc<NodeRuntime>` (shared with the agent loop).

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::tools::{Tool, ToolError, ToolOutput};

use super::node_runtime::{NodeRollbackResult, NodeRuntime, NodeVerifyResult};

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
                    "description": "Commands the runtime will execute to verify this node (e.g. cargo test)."
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
            .begin_node(goal, verify_commands, expected_files)
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
        let result: NodeVerifyResult = self.runtime.verify_node().await.map_err(|e| ToolError {
            message: format!("{e:#}"),
            code: Some("verify_node_failed".into()),
        })?;

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
