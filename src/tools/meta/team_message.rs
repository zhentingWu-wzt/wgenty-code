//! `team_message` tool - send/broadcast messages to teammate mailboxes (s09).
//!
//! Writes directly to `.team/inbox/{recipient}.jsonl` so peers running
//! `MailboxInbox` drain them next round. No `.team/config.json` required -
//! any agent id is a valid recipient (best-effort: missing dir is created).

use crate::teams::mailbox::{Mailbox, TeamMessage};
use crate::tools::{Tool, ToolError, ToolOutput};
use async_trait::async_trait;

pub struct TeamMessageTool;

impl Default for TeamMessageTool {
    fn default() -> Self {
        Self
    }
}

impl TeamMessageTool {
    pub fn new() -> Self {
        Self
    }

    fn inbox_path(recipient: &str) -> Option<std::path::PathBuf> {
        let cwd = std::env::current_dir().ok()?;
        let safe = sanitize(recipient);
        Some(
            cwd.join(".team")
                .join("inbox")
                .join(format!("{safe}.jsonl")),
        )
    }

    fn now_rfc3339() -> String {
        chrono::Utc::now().to_rfc3339()
    }
}

#[async_trait]
impl Tool for TeamMessageTool {
    fn name(&self) -> &str {
        "team_message"
    }

    fn description(&self) -> &str {
        "Send a message to a teammate's mailbox (operation: send | broadcast) or \
         post a shutdown request (operation: shutdown_request). Peers drain their \
         inbox at the start of each round. Messages persist as JSONL under \
         .team/inbox/{recipient}.jsonl."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["send", "broadcast", "shutdown_request"],
                    "description": "send: direct message to one recipient; broadcast: all known peers; shutdown_request: ask a peer to shut down"
                },
                "from": {
                    "type": "string",
                    "description": "Sender agent id (your own id)"
                },
                "to": {
                    "type": "string",
                    "description": "Recipient agent id (send / shutdown_request)"
                },
                "content": {
                    "type": "string",
                    "description": "Message body (send / broadcast)"
                },
                "request_id": {
                    "type": "string",
                    "description": "Correlation id for shutdown_request"
                },
                "recipients": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "For broadcast: explicit recipient ids to fan out to (optional; if omitted, no-op)"
                }
            },
            "required": ["operation", "from"]
        })
    }

    fn is_read_only(&self) -> bool {
        false
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let operation = input["operation"].as_str().ok_or_else(|| ToolError {
            message: "operation is required".into(),
            code: Some("missing_operation".into()),
        })?;
        let from = input["from"]
            .as_str()
            .ok_or_else(|| ToolError {
                message: "from is required".into(),
                code: Some("missing_from".into()),
            })?
            .to_string();

        match operation {
            "send" => {
                let to = input["to"]
                    .as_str()
                    .ok_or_else(|| ToolError {
                        message: "to is required for send".into(),
                        code: Some("missing_to".into()),
                    })?
                    .to_string();
                let content = input["content"]
                    .as_str()
                    .ok_or_else(|| ToolError {
                        message: "content is required for send".into(),
                        code: Some("missing_content".into()),
                    })?
                    .to_string();
                let msg = TeamMessage::Message {
                    from: from.clone(),
                    to: to.clone(),
                    content,
                    timestamp: Self::now_rfc3339(),
                };
                deliver(&to, &msg).await?;
                Ok(ok(&format!("sent message to {to}")))
            }
            "broadcast" => {
                let content = input["content"]
                    .as_str()
                    .ok_or_else(|| ToolError {
                        message: "content is required for broadcast".into(),
                        code: Some("missing_content".into()),
                    })?
                    .to_string();
                let recipients: Vec<String> = input["recipients"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                let mut delivered = 0usize;
                for r in &recipients {
                    if r == &from {
                        continue;
                    }
                    let msg = TeamMessage::Broadcast {
                        from: from.clone(),
                        content: content.clone(),
                        timestamp: Self::now_rfc3339(),
                    };
                    if deliver(r, &msg).await.is_ok() {
                        delivered += 1;
                    }
                }
                Ok(ok(&format!("broadcast to {delivered} recipient(s)")))
            }
            "shutdown_request" => {
                let to = input["to"]
                    .as_str()
                    .ok_or_else(|| ToolError {
                        message: "to is required for shutdown_request".into(),
                        code: Some("missing_to".into()),
                    })?
                    .to_string();
                let request_id = input["request_id"]
                    .as_str()
                    .ok_or_else(|| ToolError {
                        message: "request_id is required for shutdown_request".into(),
                        code: Some("missing_request_id".into()),
                    })?
                    .to_string();
                let msg = TeamMessage::ShutdownRequest {
                    from: from.clone(),
                    request_id: request_id.clone(),
                };
                deliver(&to, &msg).await?;
                Ok(ok(&format!(
                    "sent shutdown request to {to} (id={request_id})"
                )))
            }
            other => Err(ToolError {
                message: format!("unknown operation: {other}"),
                code: Some("invalid_operation".into()),
            }),
        }
    }
}

async fn deliver(recipient: &str, msg: &TeamMessage) -> Result<(), ToolError> {
    let path = TeamMessageTool::inbox_path(recipient).ok_or_else(|| ToolError {
        message: "cannot resolve cwd for mailbox".into(),
        code: Some("io_error".into()),
    })?;
    let mailbox = Mailbox::new(path);
    mailbox.send(msg).await.map_err(|e| ToolError {
        message: format!("failed to write mailbox: {e}"),
        code: Some("io_error".into()),
    })
}

fn ok(message: &str) -> ToolOutput {
    ToolOutput {
        output_type: "json".to_string(),
        content: serde_json::json!({ "success": true, "message": message }).to_string(),
        metadata: std::collections::HashMap::new(),
    }
}

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_unsafe() {
        assert_eq!(sanitize("agent-1_2"), "agent-1_2");
        assert_eq!(sanitize("../etc"), "___etc");
        assert_eq!(sanitize("a b"), "a_b");
    }

    #[tokio::test]
    async fn send_and_drain_roundtrip() {
        let tmp = tempfile::TempDir::new().unwrap();
        let prev = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        // Simulate recipient inbox path directly.
        let path = TeamMessageTool::inbox_path("recipient-1").unwrap();
        let mailbox = Mailbox::new(path);
        mailbox
            .send(&TeamMessage::Message {
                from: "sender".into(),
                to: "recipient-1".into(),
                content: "hello team".into(),
                timestamp: "t".into(),
            })
            .await
            .unwrap();

        let drained = mailbox.receive_all().await.unwrap();
        assert_eq!(drained.len(), 1);

        std::env::set_current_dir(prev).unwrap();
    }
}
