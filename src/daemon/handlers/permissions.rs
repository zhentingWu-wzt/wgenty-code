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

/// POST /api/v1/permission-mode - update the root agent's runtime permission
/// mode (Yolo/AcceptEdits/Normal) and optional sandbox effective mode (Plan).
/// Subagents snapshot values at spawn time.
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
    let session_root = state.effective_session_root(session_id).await;
    let effective = body
        .effective_mode
        .unwrap_or_else(|| crate::sandbox::EffectiveMode::from_root_permission_mode(body.mode));
    state
        .permission_modes
        .set(session_root, body.mode, effective);
    tracing::info!(
        mode = ?body.mode,
        effective_mode = ?effective,
        "root permission / effective mode updated"
    );
    state.broadcast_global(
        crate::daemon::global_events::GlobalEventKind::ModeChanged,
        serde_json::json!({
            "session_id": session_id,
            "mode": body.mode,
            "effective_mode": effective,
        }),
    );
    Ok(Json(serde_json::json!({
        "success": true,
        "mode": body.mode,
        "effective_mode": effective,
    })))
}

/// GET /api/v1/permission-mode - get the current root agent permission mode.
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
    let session_root = state.effective_session_root(session_id).await;
    let entry = state.permission_modes.get(&session_root);
    Ok(Json(serde_json::json!({
        "mode": entry.root_mode,
        "effective_mode": entry.effective_mode,
    })))
}

// ── Tasks ────────────────────────────────────────────────────────────────────
