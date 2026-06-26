use super::AgentLoop;
use crate::runtime::guardian::classify_risk;
use crate::tui::app::AppEvent;
use crate::tui::client::DaemonClient;
use std::time::Duration;
use tokio::sync::mpsc;

impl AgentLoop {
    /// Static version of tool execution for use in parallel spawned tasks.
    /// Skips the interactive permission flow (tools requiring permission should
    /// not be executed in parallel batches).
    pub(super) async fn execute_tool_static(
        client: &DaemonClient,
        name: &str,
        args: serde_json::Value,
        session_id: &str,
        event_tx: Option<mpsc::UnboundedSender<AppEvent>>,
    ) -> String {
        // Guardian: inline safety check (no UI interaction in parallel path)
        if name == "execute_command" || name == "exec_command" {
            if let Some(cmd) = args.get("command").and_then(|v| v.as_str()) {
                let risk = classify_risk(cmd);
                if risk >= crate::runtime::guardian::RiskLevel::Critical {
                    return format!(
                        r#"{{"success":false,"error":"GUARDIAN BLOCK: critical-risk command rejected. {}"}}"#,
                        cmd
                    );
                }
            }
        }

        // Poll subagent progress while task/delegate tools are running
        let is_long_running = name == "task" || name == "delegate";

        let poll_handle = if let (true, Some(tx)) = (is_long_running, event_tx.as_ref()) {
            let tx = tx.clone();
            let client_clone = client.clone();
            let sid = session_id.to_string();
            Some(tokio::spawn(async move {
                let start = tokio::time::Instant::now();
                let max_poll_duration = Duration::from_secs(120);
                loop {
                    if start.elapsed() > max_poll_duration {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    match client_clone.poll_subagent_progress(&sid).await {
                        Ok(map) => {
                            if map.is_empty() {
                                continue;
                            }
                            for (_id, progress) in map {
                                let _ = tx.send(AppEvent::SubagentUpdate(Box::new(progress)));
                            }
                        }
                        Err(_) => break,
                    }
                }
            }))
        } else {
            None
        };

        let result = match client.execute_tool(name, args, session_id).await {
            Ok(resp) => {
                if let Some(perm) = resp.permission_required {
                    format!(
                        r#"{{"success":false,"error":"PERMISSION REQUIRED: {} (cannot prompt in parallel mode)"}}"#,
                        perm.reason
                    )
                } else {
                    format!(
                        r#"{{"success":{},"output_type":{},"content":{},"error":{},"metadata":{}}}"#,
                        resp.success,
                        serde_json::to_string(&resp.output_type).unwrap_or_default(),
                        serde_json::to_string(&resp.content).unwrap_or_default(),
                        serde_json::to_string(&resp.error).unwrap_or_default(),
                        serde_json::to_string(&resp.metadata).unwrap_or_default(),
                    )
                }
            }
            Err(e) => format!(r#"{{"success":false,"error":"{}"}}"#, e),
        };

        // Poller continues in background for async subagents (task tool with background=true).
        // It self-terminates after 120s max. Drop the handle — tokio::spawn keeps it alive.
        drop(poll_handle);

        result
    }

    pub(super) async fn execute_tool_with_permission(
        &mut self,
        name: &str,
        args: serde_json::Value,
    ) -> String {
        // Guardian: block critical-risk commands before they reach the daemon
        if name == "execute_command" || name == "exec_command" {
            if let Some(cmd) = args.get("command").and_then(|v| v.as_str()) {
                let risk = classify_risk(cmd);
                if risk >= crate::runtime::guardian::RiskLevel::Critical {
                    let msg = format!("GUARDIAN BLOCK: critical-risk command rejected. {}", cmd);
                    tracing::warn!("{}", msg);
                    return format!(r#"{{"success":false,"error":"{}"}}"#, msg);
                }
            }
        }

        let result = match self
            .client
            .execute_tool(name, args.clone(), &self.session_id)
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                tracing::warn!("Tool execution failed for '{}': {}", name, e);
                return format!(r#"{{"success":false,"error":"{}"}}"#, e);
            }
        };

        // If permission required, ask the user via inline panel
        if let Some(perm) = result.permission_required {
            tracing::info!(
                "🔐 Permission required for '{}': {} (rule: {})",
                name,
                perm.reason,
                perm.session_rule
            );

            // Fire PermissionRequest hook asynchronously before sending event
            {
                let hm = self.hook_manager.clone();
                let hook_name = name.to_string();
                let hook_args = args.clone();
                let hook_sid = self.session_id.clone();
                tokio::spawn(async move {
                    let cwd = std::env::current_dir().unwrap_or_default();
                    let ctx = crate::runtime::hooks::HookContext {
                        event: "PermissionRequest".to_string(),
                        tool_name: Some(hook_name),
                        tool_input: Some(hook_args),
                        tool_result: None,
                        session_id: Some(hook_sid),
                        working_directory: cwd.to_string_lossy().to_string(),
                        timestamp: chrono::Utc::now().to_rfc3339(),
                        comet_phase: None,
                        workflow_state: None,
                        variables: Default::default(),
                    };
                    hm.fire(&crate::runtime::hooks::HookEvent::PermissionRequest, &ctx, None, None)
                        .await;
                });
            }

            let (tx, rx) = tokio::sync::oneshot::channel();

            let _ = self.event_tx.send(AppEvent::PermissionRequired {
                reason: perm.reason.clone(),
                rule: perm.session_rule.clone(),
                responder: crate::tui::app::PermissionResponder(Some(tx)),
            });

            match rx.await {
                Ok(crate::tui::app::PermissionResponse::AllowOnce) => {
                    // Approve → execute → unapprove (one-shot)
                    if self.client.approve_tool(&perm.session_rule).await.is_err() {
                        return r#"{"success":false,"error":"Failed to approve permission"}"#
                            .to_string();
                    }

                    let result = self
                        .client
                        .execute_tool(name, args.clone(), &self.session_id)
                        .await;

                    // Remove the temporary approval
                    let _ = self.client.unapprove_tool(&perm.session_rule).await;

                    match result {
                        Ok(resp) => {
                            return format!(
                                r#"{{"success":{},"output_type":{},"content":{},"error":{},"metadata":{}}}"#,
                                resp.success,
                                serde_json::to_string(&resp.output_type).unwrap_or_default(),
                                serde_json::to_string(&resp.content).unwrap_or_default(),
                                serde_json::to_string(&resp.error).unwrap_or_default(),
                                serde_json::to_string(&resp.metadata).unwrap_or_default(),
                            );
                        }
                        Err(e) => {
                            return format!(r#"{{"success":false,"error":"{}"}}"#, e);
                        }
                    }
                }
                Ok(crate::tui::app::PermissionResponse::AlwaysAllow) => {
                    // Approve the rule, then re-execute the tool
                    if self.client.approve_tool(&perm.session_rule).await.is_err() {
                        return r#"{{"success":false,"error":"Failed to approve permission"}}"#
                            .to_string();
                    }

                    match self
                        .client
                        .execute_tool(name, args.clone(), &self.session_id)
                        .await
                    {
                        Ok(resp) => {
                            return format!(
                                r#"{{"success":{},"output_type":{},"content":{},"error":{},"metadata":{}}}"#,
                                resp.success,
                                serde_json::to_string(&resp.output_type).unwrap_or_default(),
                                serde_json::to_string(&resp.content).unwrap_or_default(),
                                serde_json::to_string(&resp.error).unwrap_or_default(),
                                serde_json::to_string(&resp.metadata).unwrap_or_default(),
                            );
                        }
                        Err(e) => {
                            return format!(r#"{{"success":false,"error":"{}"}}"#, e);
                        }
                    }
                }
                Ok(crate::tui::app::PermissionResponse::Deny) | Err(_) => {
                    return format!(
                        r#"{{"success":false,"error":"PERMISSION DENIED: {}"}}"#,
                        perm.reason
                    );
                }
            }
        }

        // No permission required — return result directly
        format!(
            r#"{{"success":{},"output_type":{},"content":{},"error":{},"metadata":{}}}"#,
            result.success,
            serde_json::to_string(&result.output_type).unwrap_or_default(),
            serde_json::to_string(&result.content).unwrap_or_default(),
            serde_json::to_string(&result.error).unwrap_or_default(),
            serde_json::to_string(&result.metadata).unwrap_or_default(),
        )
    }

    pub(super) async fn handle_ask_user_question(&self, args: &serde_json::Value) -> String {
        let (tx, rx) = tokio::sync::oneshot::channel();

        let question = args["question"]
            .as_str()
            .unwrap_or("Choose an option:")
            .to_string();
        let options: Vec<String> = args["options"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|o| o["label"].as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let multi_select = args["multi_select"].as_bool().unwrap_or(false);

        let _ = self.event_tx.send(AppEvent::QuestionAsked {
            question,
            options,
            multi_select,
            responder: crate::tui::app::QuestionResponder(Some(tx)),
        });

        match rx.await {
            Ok(answers) => {
                let answers_json: Vec<serde_json::Value> = answers
                    .iter()
                    .map(|a| serde_json::json!({"label": a, "value": a, "custom": false}))
                    .collect();
                serde_json::json!({
                    "success": true,
                    "answers": answers_json
                })
                .to_string()
            }
            Err(_) => {
                // Channel closed without response (user pressed Esc)
                serde_json::json!({
                    "success": false,
                    "error": "User cancelled the question"
                })
                .to_string()
            }
        }
    }
}
