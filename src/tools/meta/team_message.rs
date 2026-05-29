//! Team Message Tool — send/receive messages via JSONL mailboxes.
//!
//! Provides a single `team_message` tool that supports four actions:
//!   - send:      Direct message a team member
//!   - inbox:     Check your own mailbox (drains it)
//!   - broadcast: Message all team members
//!   - members:   List the team roster

use crate::teams::mailbox::TeamManager;
use crate::tools::{Tool, ToolError, ToolOutput};
use async_trait::async_trait;
use std::sync::Arc;

pub struct TeamMessageTool {
    team_manager: Option<Arc<TeamManager>>,
}

impl TeamMessageTool {
    pub fn new(team_manager: Option<Arc<TeamManager>>) -> Self {
        Self { team_manager }
    }
}

#[async_trait]
impl Tool for TeamMessageTool {
    fn name(&self) -> &str {
        "team_message"
    }

    fn description(&self) -> &str {
        "Send a message to a team member or check your inbox. \
         Use 'send' to message a teammate, 'inbox' to check your messages, \
         'broadcast' to message everyone."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "Action: send (to a member), inbox (check own messages), broadcast (to all), members (list team)",
                    "enum": ["send", "inbox", "broadcast", "members"]
                },
                "to": {
                    "type": "string",
                    "description": "Team member name to send to (required for send)"
                },
                "content": {
                    "type": "string",
                    "description": "Message content (required for send and broadcast)"
                },
                "from": {
                    "type": "string",
                    "description": "Your team member name (required for inbox check)"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let tm = match &self.team_manager {
            Some(tm) => tm,
            None => {
                return Err(ToolError {
                    message:
                        "No team configured. Create .team/config.json to enable team features."
                            .to_string(),
                    code: Some("no_team".to_string()),
                })
            }
        };

        let action = input["action"].as_str().unwrap_or("inbox");

        match action {
            "send" => {
                let to = input["to"].as_str().ok_or_else(|| ToolError {
                    message: "to is required for send".to_string(),
                    code: Some("missing_to".to_string()),
                })?;
                let from = input["from"].as_str().unwrap_or("agent");
                let content = input["content"].as_str().unwrap_or("");

                tm.send_message(from, to, content)
                    .await
                    .map_err(|e| ToolError {
                        message: format!("Failed to send: {}", e),
                        code: Some("send_error".to_string()),
                    })?;

                Ok(ToolOutput {
                    output_type: "json".to_string(),
                    content: serde_json::json!({
                        "success": true,
                        "message": format!("Message sent to {}", to)
                    })
                    .to_string(),
                    metadata: std::collections::HashMap::new(),
                })
            }
            "inbox" => {
                let from = input["from"].as_str().unwrap_or("agent");
                let messages = tm.receive(from).await.map_err(|e| ToolError {
                    message: format!("Failed to read inbox: {}", e),
                    code: Some("inbox_error".to_string()),
                })?;

                Ok(ToolOutput {
                    output_type: "json".to_string(),
                    content: serde_json::json!({
                        "success": true,
                        "messages": messages,
                        "count": messages.len()
                    })
                    .to_string(),
                    metadata: std::collections::HashMap::new(),
                })
            }
            "broadcast" => {
                let from = input["from"].as_str().unwrap_or("agent");
                let content = input["content"].as_str().unwrap_or("");
                let results = tm.broadcast(from, content).await;
                let succeeded = results.iter().filter(|r| r.is_ok()).count();

                Ok(ToolOutput {
                    output_type: "json".to_string(),
                    content: serde_json::json!({
                        "success": true,
                        "message": format!("Broadcast sent to {} members", succeeded)
                    })
                    .to_string(),
                    metadata: std::collections::HashMap::new(),
                })
            }
            "members" => {
                let members: Vec<serde_json::Value> = tm
                    .members()
                    .iter()
                    .map(|m| {
                        serde_json::json!({
                            "name": m.name,
                            "role": m.role,
                            "status": m.status,
                        })
                    })
                    .collect();

                Ok(ToolOutput {
                    output_type: "json".to_string(),
                    content: serde_json::json!({
                        "success": true,
                        "team": tm.team_name(),
                        "members": members,
                    })
                    .to_string(),
                    metadata: std::collections::HashMap::new(),
                })
            }
            _ => Err(ToolError {
                message: format!("Unknown action: {}", action),
                code: Some("unknown_action".to_string()),
            }),
        }
    }
}
