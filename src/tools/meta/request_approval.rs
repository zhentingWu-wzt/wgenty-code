//! `request_approval` tool - ask a parent/peer to approve an action (s10).
//!
//! Sends an `ApprovalRequest` to the recipient's mailbox and blocks (with a
//! timeout) for an `ApprovalResponse`. The response is delivered by
//! `MailboxInbox::drain` resolving the oneshot registered here.

use crate::agent::ToolContext;
use crate::teams::approval_registry;
use crate::teams::mailbox::{Mailbox, TeamMessage};
use crate::tools::{Tool, ToolError, ToolOutput};
use async_trait::async_trait;
use std::collections::HashMap;
use std::time::Duration;

pub struct RequestApprovalTool;

impl Default for RequestApprovalTool {
    fn default() -> Self {
        Self
    }
}

impl RequestApprovalTool {
    pub fn new() -> Self {
        Self
    }

    fn inbox_path(recipient: &str) -> Option<std::path::PathBuf> {
        let cwd = std::env::current_dir().ok()?;
        let safe: String = recipient
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        Some(cwd.join(".team").join("inbox").join(format!("{safe}.jsonl")))
    }
}

#[async_trait]
impl Tool for RequestApprovalTool {
    fn name(&self) -> &str {
        "request_approval"
    }

    fn description(&self) -> &str {
        "Ask a parent or peer agent to approve an action before proceeding. \
         Sends an ApprovalRequest to their mailbox and blocks for a response \
         (default timeout 60s). Use the request_id to correlate."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "to": {"type": "string", "description": "Recipient agent id (defaults to your parent)"},
                "kind": {"type": "string", "description": "Approval kind, e.g. 'destructive_edit', 'deploy'"},
                "payload": {"type": "string", "description": "What you want approved"},
                "request_id": {"type": "string", "description": "Correlation id (generate a unique one)"},
                "timeout_secs": {"type": "integer", "description": "Max seconds to wait (default 60)"}
            },
            "required": ["kind", "payload", "request_id"]
        })
    }

    fn is_read_only(&self) -> bool {
        false
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        // Without a trusted context we can't know the requester's agent_id, so
        // fall back to an "unknown" sender and require `to` explicitly.
        let from = input["from"].as_str().unwrap_or("unknown").to_string();
        let to = input["to"]
            .as_str()
            .ok_or_else(|| ToolError { message: "to is required".into(), code: Some("missing_to".into()) })?
            .to_string();
        let kind = input["kind"]
            .as_str()
            .ok_or_else(|| ToolError { message: "kind is required".into(), code: Some("missing_kind".into()) })?
            .to_string();
        let payload = input["payload"]
            .as_str()
            .ok_or_else(|| ToolError { message: "payload is required".into(), code: Some("missing_payload".into()) })?
            .to_string();
        let request_id = input["request_id"]
            .as_str()
            .ok_or_else(|| ToolError { message: "request_id is required".into(), code: Some("missing_request_id".into()) })?
            .to_string();
        let timeout_secs = input["timeout_secs"].as_u64().unwrap_or(60);

        self.do_request(&from, &to, &kind, &payload, &request_id, timeout_secs)
            .await
    }

    async fn execute_with_context(
        &self,
        context: &ToolContext<'_>,
        args: serde_json::Value,
    ) -> Result<ToolOutput, ToolError> {
        // Prefer parent as recipient when `to` omitted.
        let from = context.agent.agent_id.as_str().to_string();
        let default_to = context
            .agent
            .parent_id
            .as_ref()
            .map(|p| p.as_str().to_string())
            .unwrap_or_else(|| from.clone());
        let to = args["to"].as_str().unwrap_or(&default_to).to_string();
        let kind = args["kind"].as_str().unwrap_or("generic").to_string();
        let payload = args["payload"].as_str().unwrap_or("").to_string();
        let request_id = args["request_id"]
            .as_str()
            .map(String::from)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let timeout_secs = args["timeout_secs"].as_u64().unwrap_or(60);
        self.do_request(&from, &to, &kind, &payload, &request_id, timeout_secs)
            .await
    }
}

impl RequestApprovalTool {
    async fn do_request(
        &self,
        from: &str,
        to: &str,
        kind: &str,
        payload: &str,
        request_id: &str,
        timeout_secs: u64,
    ) -> Result<ToolOutput, ToolError> {
        // Register a waiter on THIS agent's pending-approvals map.
        let pending = approval_registry::register_agent(from);
        let (tx, rx) = tokio::sync::oneshot::channel::<bool>();
        {
            let mut map = pending.lock().unwrap();
            map.insert(request_id.to_string(), tx);
        }

        // Send the request to the recipient's mailbox.
        let path = Self::inbox_path(to)
            .ok_or_else(|| ToolError { message: "cannot resolve cwd for mailbox".into(), code: Some("io_error".into()) })?;
        let mailbox = Mailbox::new(path);
        let msg = TeamMessage::ApprovalRequest {
            from: from.to_string(),
            request_id: request_id.to_string(),
            kind: kind.to_string(),
            payload: payload.to_string(),
        };
        mailbox.send(&msg).await.map_err(|e| ToolError {
            message: format!("failed to write mailbox: {e}"),
            code: Some("io_error".into()),
        })?;

        // Wait for the response (driven by MailboxInbox::drain resolving the oneshot).
        let outcome = tokio::time::timeout(Duration::from_secs(timeout_secs), rx).await;
        // Clean up the waiter if it's still there (e.g. timeout).
        let mut map = pending.lock().unwrap();
        map.remove(request_id);

        match outcome {
            Ok(Ok(approve)) => Ok(ToolOutput {
                output_type: "json".to_string(),
                content: serde_json::json!({
                    "success": true,
                    "request_id": request_id,
                    "approved": approve,
                    "timed_out": false
                })
                .to_string(),
                metadata: HashMap::new(),
            }),
            Ok(Err(_)) => Ok(ToolOutput {
                output_type: "json".to_string(),
                content: serde_json::json!({
                    "success": false,
                    "request_id": request_id,
                    "error": "approval channel closed without response",
                    "timed_out": false
                })
                .to_string(),
                metadata: HashMap::new(),
            }),
            Err(_) => Ok(ToolOutput {
                output_type: "json".to_string(),
                content: serde_json::json!({
                    "success": false,
                    "request_id": request_id,
                    "error": format!("timed out after {timeout_secs}s"),
                    "timed_out": true
                })
                .to_string(),
                metadata: HashMap::new(),
            }),
        }
    }
}

