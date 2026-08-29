//! Tool listing, execution and approval handlers.

use super::*;

pub async fn list_tools(State(state): State<Arc<DaemonState>>) -> Json<ListToolsResponse> {
    let tools: Vec<ToolInfo> = state
        .tool_registry
        .list()
        .into_iter()
        .map(|t| ToolInfo {
            name: t.name().to_string(),
            description: t.description().to_string(),
            input_schema: t.input_schema(),
            is_read_only: t.is_read_only(),
        })
        .collect();

    Json(ListToolsResponse { tools })
}

pub async fn execute_tool(
    State(state): State<Arc<DaemonState>>,
    Json(body): Json<ExecuteToolRequest>,
) -> Result<Json<ExecuteToolResponse>, StatusCode> {
    let tool_name = &body.tool_name;
    let args = &body.arguments;
    // deprecated(compat): legacy chat_stream client path; server-side callers
    // must pass session_id. Removal is a separate change.
    let session_id = body.session_id.as_deref().unwrap_or("default");

    // Multi-project: validate against the session's own effective root (bound
    // worktree > project > main working_dir) so the policy boundary always
    // matches the execution workdir, and snapshot into that project's
    // checkpoint store.
    let session_root = state.effective_session_root(session_id).await;
    let (cp_manager, cp_store) = state.checkpoints_for_project(&session_root);

    // Per-project permission mode for this session's working directory.
    let mode_entry = state.permission_modes.get(&session_root);

    // Validate against policy
    let decision = {
        let policy = ToolPermissionPolicy::new(session_root.clone());
        let rules_handle = state.tool_executor.session_rules_handle();
        let rules = rules_handle.read().await;
        crate::tools::executor::validate_tool_call_shared(
            &state.tool_registry,
            &policy,
            &rules,
            tool_name,
            args,
        )
    };
    tracing::info!("🔐 Daemon: policy for '{}' = {:?}", tool_name, decision);
    match decision {
        Ok(PolicyDecision::Allow) => {
            // Build the trusted root execution context for this session. Uses
            // the coordinator's ensure_root so the root scope is registered and
            // the task tool can reserve children under it.
            let root_context = state
                .root_context(session_id)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            let effective_mode = mode_entry.effective_mode;
            // Ensure the turn snapshot exists when the client supplies a turn id
            // (TUI/REPL generate one per user message). Plan mode skips capture
            // inside maybe_capture_pre_edit via EffectiveMode::Plan.
            if let Some(turn_id) = body.turn_id.as_deref() {
                if let Err(e) = cp_manager.begin_turn(turn_id) {
                    tracing::warn!(error = %e, turn = %turn_id, "checkpoint begin_turn failed");
                }
            }
            let tool_context = crate::agent::ToolContext {
                agent: &root_context,
                invocation_id: crate::agent::ToolInvocationId::new(
                    uuid::Uuid::new_v4().to_string(),
                ),
                origin_turn_id: body.turn_id.as_deref(),
                workdir: Some(session_root.as_path()),
                effective_mode,
                checkpoint: Some(cp_store.as_ref()),
            };
            // Execute directly with hooks
            let msg = state
                .tool_executor
                .execute_with_hooks(&tool_context, "api", tool_name, args.clone())
                .await;
            let content = msg.content.unwrap_or_default();
            let parsed: serde_json::Value = serde_json::from_str(&content).unwrap_or_default();

            Ok(Json(ExecuteToolResponse {
                success: parsed["success"].as_bool().unwrap_or(false),
                output_type: parsed["output_type"].as_str().map(|s| s.to_string()),
                content: parsed["content"].as_str().map(|s| s.to_string()),
                error: parsed["error"]
                    .get("message")
                    .and_then(|m| m.as_str())
                    .map(|s| s.to_string()),
                metadata: parsed.get("metadata").cloned(),
                permission_required: None,
            }))
        }
        Ok(PolicyDecision::Ask(req)) => {
            // Check if rule was already approved for this session, OR root mode
            // auto-approves this tool (AcceptEdits / Yolo). Without the mode
            // bypass, AcceptEdits still bounced every write through the TUI.
            let mode_auto = mode_entry.root_mode.auto_approves(tool_name);
            let already = state.is_rule_approved(session_id, &req.session_rule).await;
            if already || mode_auto {
                if mode_auto && !already {
                    tracing::info!(
                        "🔐 Daemon: root_mode auto-approved '{}' (rule: {})",
                        tool_name,
                        req.session_rule
                    );
                }
                // Per-tool git-stash checkpoints removed: pre-edit capture happens
                // inside ToolRegistry::execute_with_context via CheckpointStore.
                if let Some(turn_id) = body.turn_id.as_deref() {
                    if let Err(e) = cp_manager.begin_turn(turn_id) {
                        tracing::warn!(error = %e, turn = %turn_id, "checkpoint begin_turn failed");
                    }
                }
                let root_context = state
                    .root_context(session_id)
                    .await
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
                let effective_mode = mode_entry.effective_mode;
                let tool_context = crate::agent::ToolContext {
                    agent: &root_context,
                    invocation_id: crate::agent::ToolInvocationId::new(
                        uuid::Uuid::new_v4().to_string(),
                    ),
                    origin_turn_id: body.turn_id.as_deref(),
                    workdir: Some(session_root.as_path()),
                    effective_mode,
                    checkpoint: Some(cp_store.as_ref()),
                };
                let msg = state
                    .tool_executor
                    .execute_with_hooks(&tool_context, "api", tool_name, args.clone())
                    .await;
                let content = msg.content.unwrap_or_default();
                let parsed: serde_json::Value = serde_json::from_str(&content).unwrap_or_default();

                return Ok(Json(ExecuteToolResponse {
                    success: parsed["success"].as_bool().unwrap_or(false),
                    output_type: parsed["output_type"].as_str().map(|s| s.to_string()),
                    content: parsed["content"].as_str().map(|s| s.to_string()),
                    error: parsed["error"]
                        .get("message")
                        .and_then(|m| m.as_str())
                        .map(|s| s.to_string()),
                    metadata: parsed.get("metadata").cloned(),
                    permission_required: None,
                }));
            }

            // Need permission from user
            tracing::info!(
                "🔐 Daemon: permission required for '{}': {} (rule: {})",
                tool_name,
                req.reason,
                req.session_rule
            );
            Ok(Json(ExecuteToolResponse {
                success: false,
                output_type: None,
                content: None,
                error: None,
                metadata: None,
                permission_required: Some(PermissionRequiredInfo {
                    tool_name: tool_name.clone(),
                    reason: req.reason,
                    session_rule: req.session_rule,
                }),
            }))
        }
        Err(e) => Ok(Json(ExecuteToolResponse {
            success: false,
            output_type: None,
            content: None,
            error: Some(e.message),
            metadata: None,
            permission_required: None,
        })),
    }
}

pub async fn approve_tool(
    State(state): State<Arc<DaemonState>>,
    Json(body): Json<ApproveToolRequest>,
) -> Json<serde_json::Value> {
    state
        .tool_executor
        .approve_rule(body.session_rule.clone())
        .await;
    // deprecated(compat): legacy chat_stream client path; server-side callers
    // must pass session_id. Removal is a separate change.
    state.approve_rule("default", body.session_rule).await;

    Json(serde_json::json!({"success": true}))
}

pub async fn unapprove_tool(
    State(state): State<Arc<DaemonState>>,
    Json(body): Json<ApproveToolRequest>,
) -> Json<serde_json::Value> {
    state.tool_executor.unapprove_rule(&body.session_rule).await;
    // deprecated(compat): legacy chat_stream client path; server-side callers
    // must pass session_id. Removal is a separate change.
    state.unapprove_rule("default", &body.session_rule).await;

    Json(serde_json::json!({"success": true}))
}
