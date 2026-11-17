//! Permission resolution and mode handlers.

use super::*;

/// GET /api/v1/tools/pending-permissions — subagent policy Ask waiters.
pub async fn list_pending_permissions(
    State(state): State<Arc<DaemonState>>,
) -> Json<crate::daemon::models::ListPendingPermissionsResponse> {
    let pending = state
        .permission_bridge
        .pending()
        .await
        .into_iter()
        .map(|a| crate::daemon::models::PendingSubagentPermission {
            request_id: a.request_id,
            from: a.from,
            kind: a.kind,
            tool: a.tool,
            policy_reason: a.policy_reason,
            session_rule: a.session_rule,
            human_summary: a.human_summary,
        })
        .collect();
    Json(crate::daemon::models::ListPendingPermissionsResponse { pending })
}

/// POST /api/v1/tools/resolve-permission — unblock a subagent Ask waiter.
///
/// Duplicate answers get 409 with the standing (first) decision instead of an
/// indistinguishable `{success:false}`; unknown ids get 404.
pub async fn resolve_subagent_permission(
    State(state): State<Arc<DaemonState>>,
    Json(body): Json<crate::daemon::models::ResolveSubagentPermissionRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    use crate::teams::PermissionResolveOutcome as Outcome;
    match state
        .permission_bridge
        .resolve(&body.request_id, body.approved)
        .await
    {
        Outcome::Resolved => {
            // Approve the standing rule only on the FIRST resolution — a
            // duplicate answer must produce no second effect (spec §审批).
            if body.approved && body.always {
                if let Some(rule) = body.session_rule.clone() {
                    state.tool_executor.approve_rule(rule.clone()).await;
                    // deprecated(compat): legacy global rule scope, see Task 12.
                    state.approve_rule("default", rule).await;
                }
            }
            Ok(Json(
                serde_json::json!({ "success": true, "resolved": true }),
            ))
        }
        Outcome::AlreadyResolved(approved) => Err((
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "success": false, "resolved": true, "approved": approved,
            })),
        )),
        Outcome::Unknown => Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "success": false, "resolved": false })),
        )),
    }
}

/// `POST /api/v1/interactions/:id/resolve` — answer a pending ask_user_question
/// prompt from the server-side loop. The answer string unblocks the waiting
/// InteractionBridge waiter. Duplicate answers get 409 with the standing
/// (first) resolution instead of an indistinguishable 404.
pub async fn resolve_interaction(
    State(state): State<Arc<DaemonState>>,
    Path(request_id): Path<String>,
    Json(body): Json<crate::daemon::models::ResolveInteractionRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    match state
        .interaction_bridge
        .resolve(&request_id, body.answer)
        .await
    {
        crate::daemon::interaction_bridge::ResolveOutcome::Resolved => {
            Ok(Json(serde_json::json!({ "resolved": true })))
        }
        crate::daemon::interaction_bridge::ResolveOutcome::AlreadyResolved(answer) => {
            // Duplicate answer: conflict, and hand back the standing resolution.
            Err((
                StatusCode::CONFLICT,
                Json(serde_json::json!({ "resolved": false, "answer": answer })),
            ))
        }
        // Never existed (or waiter gone). 404 so the client stops retrying.
        crate::daemon::interaction_bridge::ResolveOutcome::Unknown => Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "unknown interaction" })),
        )),
    }
}

/// POST /api/v1/permission-mode - update the session's permission mode
/// (Yolo/AcceptEdits/Normal) and optional sandbox effective mode (Plan).
/// Plan is normalized before the override is persisted.
pub async fn set_permission_mode(
    State(state): State<Arc<DaemonState>>,
    Json(body): Json<crate::daemon::models::SetPermissionModeRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    // Server-side path: approvals/rules must belong to a real session
    // (design §4) — missing session_id is a client bug, not a default.
    let Some(session_id) = body.session_id.as_deref() else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "session_id required"})),
        ));
    };
    // Reject ids that could escape the sessions directory (path traversal /
    // absolute paths) before any storage access: a hostile id is a client
    // bug worth a 400, never a 500 from a failed write.
    if crate::context::memory_session::validate_id(session_id).is_err() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "invalid session_id"})),
        ));
    }

    if body.reset && (body.mode.is_some() || body.effective_mode.is_some()) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "reset cannot be combined with mode or effective_mode"
            })),
        ));
    }

    if body.reset {
        let _update_guard = state.session_update_lock.lock().await;
        if let Some((sessions, _)) = state.resolve_session(session_id).await {
            sessions
                .update_session_record(session_id, |session| {
                    session.permission_override = None;
                })
                .await
                .map_err(|error| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({
                            "error": format!("clear session permission: {error}")
                        })),
                    )
                })?;
        }
        let entry = state.effective_permission(session_id).await;
        state.broadcast_global(
            crate::daemon::global_events::GlobalEventKind::ModeChanged,
            serde_json::json!({
                "session_id": session_id,
                "mode": entry.root_mode,
                "effective_mode": entry.effective_mode,
                "reset": true,
                "is_override": false,
            }),
        );
        return Ok(Json(serde_json::json!({
            "success": true,
            "session_id": session_id,
            "mode": entry.root_mode,
            "effective_mode": entry.effective_mode,
            "reset": true,
            "is_override": false,
        })));
    }

    let Some(mode) = body.mode else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "mode required"})),
        ));
    };
    let effective = body
        .effective_mode
        .unwrap_or_else(|| crate::sandbox::EffectiveMode::from_root_permission_mode(mode));
    // Serialize with PUT overwrites (which take the same lock around their
    // load → version check → save sequence) so the two full-record writers
    // cannot interleave.
    let _update_guard = state.session_update_lock.lock().await;
    match state.resolve_session(session_id).await {
        Some((sessions, _)) => {
            // Targeted in-lock update: a concurrent run save (mid-turn mode
            // switches are allowed) can neither be clobbered by this write
            // nor erase the override afterwards; version bumps per contract.
            sessions
                .update_session_record(session_id, |session| {
                    session.permission_override = Some(crate::permissions::PermissionModeEntry {
                        root_mode: mode,
                        effective_mode: effective,
                    });
                })
                .await
                .map_err(|error| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({
                            "error": format!("save session permission: {error}")
                        })),
                    )
                })?;
        }
        None => {
            // Session not known to any project store yet: create it in the
            // main project's manager with the override applied (same shape
            // the previous full-record write produced). The creating write
            // advances the version like a PUT upsert does.
            let mut session = crate::context::memory_session::Session::with_id(session_id, None)
                .with_permission_override(crate::permissions::PermissionModeEntry {
                    root_mode: mode,
                    effective_mode: effective,
                });
            session.version += 1;
            state
                .session_manager
                .save(&session)
                .await
                .map_err(|error| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({
                            "error": format!("save session permission: {error}")
                        })),
                    )
                })?;
        }
    }
    tracing::info!(
        mode = ?mode,
        effective_mode = ?effective,
        "root permission / effective mode updated"
    );
    state.broadcast_global(
        crate::daemon::global_events::GlobalEventKind::ModeChanged,
        serde_json::json!({
            "session_id": session_id,
            "mode": mode,
            "effective_mode": effective,
            "is_override": true,
        }),
    );
    Ok(Json(serde_json::json!({
        "success": true,
        "mode": mode,
        "effective_mode": effective,
        "is_override": true,
    })))
}

/// GET /api/v1/permission-mode - get the session's effective permission mode.
pub async fn get_permission_mode(
    State(state): State<Arc<DaemonState>>,
    Query(params): Query<crate::daemon::models::PermissionModeQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    // Server-side path: approvals/rules must belong to a real session
    // (design §4) — missing session_id is a client bug, not a default.
    let Some(session_id) = params.session_id.as_deref() else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "session_id required"})),
        ));
    };
    let session = state
        .resolve_session(session_id)
        .await
        .map(|(_, session)| session);
    let (entry, is_override) = permission_mode_snapshot(&state, session_id, session);
    Ok(Json(serde_json::json!({
        "mode": entry.root_mode,
        "effective_mode": entry.effective_mode,
        "is_override": is_override,
    })))
}

fn permission_mode_snapshot(
    state: &DaemonState,
    session_id: &str,
    session: Option<crate::context::memory_session::Session>,
) -> (crate::permissions::PermissionModeEntry, bool) {
    let root = match session {
        Some(session) => {
            if let Some(permission) = session.permission_override {
                return (permission, true);
            }
            state
                .session_workdir(session_id)
                .unwrap_or_else(|| state.session_project_root(&session))
        }
        None => state
            .session_workdir(session_id)
            .unwrap_or_else(|| state.projects.main_root()),
    };
    (state.permission_modes.get(&root), false)
}

// ── Tasks ────────────────────────────────────────────────────────────────────
