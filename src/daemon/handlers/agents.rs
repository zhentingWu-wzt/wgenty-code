//! Agent tree views, child lifecycle and work claiming.

use super::*;

/// Extracts and resolves the viewer token from headers. Returns None on any
/// failure; callers respond with a stable 404.
async fn resolve_viewer_from_headers(
    state: &DaemonState,
    headers: &HeaderMap,
) -> Option<crate::agent::capability::ViewerId> {
    let token = headers.get(VIEWER_TOKEN_HEADER)?.to_str().ok()?;
    state.resolve_viewer(token).await
}

pub(super) fn map_scoped_coordinator_error(error: crate::agent::CoordinatorError) -> StatusCode {
    match error {
        crate::agent::CoordinatorError::NotVisible => StatusCode::NOT_FOUND,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// `POST /api/v1/ui/viewers` -- create a trusted UI viewer. Generates a
/// 256-bit bearer token, stores only its HMAC digest, returns the token once.
pub async fn create_viewer(
    State(state): State<Arc<DaemonState>>,
) -> Result<Json<CreateViewerResponse>, StatusCode> {
    let token = state.create_viewer().await;
    Ok(Json(CreateViewerResponse {
        viewer_token: token,
    }))
}

/// Builds a `LocalAgentViewResponse` for `caller`, issuing fresh navigate
/// capabilities for each direct child bound to `viewer`.
async fn build_local_view(
    state: &DaemonState,
    caller: &crate::agent::AgentExecutionContext,
    viewer: &crate::agent::capability::ViewerId,
) -> Result<LocalAgentViewResponse, StatusCode> {
    // Cross-populate from the legacy progress store so the TUI focus view has
    // conversation data for self and each direct child. Once the
    // coordinator owns the canonical progress store this lookup becomes a
    // coordinator projection; for now it bridges the migration.
    let session_progress = {
        // Clone only this session's progress so the read guard is released
        // before issuing child capabilities across await points below.
        let progress_store = state.subagent_progress.read().await;
        progress_store
            .get(caller.session_id.as_str())
            .cloned()
            .unwrap_or_default()
    };

    assemble_local_view(
        &state.coordinator,
        &state.capability_service,
        &session_progress,
        caller,
        viewer,
    )
    .await
}

/// Computes a live `elapsed_ms` for a subagent progress entry.
///
/// The subagent loop only emits progress at round/tool boundaries, so a
/// long-running tool (e.g. `sleep`) freezes the stored `elapsed_ms` until the
/// next emit. For still-running agents recompute from `started_at` so the TUI
/// timer keeps ticking; terminal agents keep their stored final value.
pub(super) fn live_elapsed_ms(progress: Option<&crate::progress::SubagentProgress>) -> u64 {
    let Some(p) = progress else {
        return 0;
    };
    let stored = p.elapsed_ms;
    if p.status.is_terminal() || p.started_at <= 0 {
        return stored;
    }
    let now_ms = chrono::Utc::now().timestamp_millis();
    ((now_ms - p.started_at).max(0) as u64).max(stored)
}

/// Assembles a trusted UI local response and issues generation-bound
/// capabilities for its canonical direct children.
pub(super) async fn assemble_local_view(
    coordinator: &crate::agent::AgentCoordinator,
    capability_service: &crate::agent::capability::CapabilityService,
    session_progress: &HashMap<String, crate::progress::SubagentProgress>,
    caller: &crate::agent::AgentExecutionContext,
    viewer: &crate::agent::capability::ViewerId,
) -> Result<LocalAgentViewResponse, StatusCode> {
    let (self_record, child_records) = coordinator
        .trusted_ui_local_records(&caller.session_id, &caller.agent_id)
        .await
        .map_err(map_scoped_coordinator_error)?;
    let self_node = session_progress.get(self_record.agent_id.as_str());
    let mut children = Vec::with_capacity(child_records.len());
    for child in child_records {
        let grant = crate::agent::capability::CapabilityGrant::navigate(
            viewer.as_str(),
            caller.session_id.as_str(),
            child.agent_id.as_str(),
            child.generation,
        );
        let cap = capability_service.issue(&grant).await;
        // Cross-fill snapshot data from the legacy progress store.
        let node = session_progress.get(child.agent_id.as_str());
        let text_snapshot = node.and_then(|p| p.text_snapshot.clone());
        let cumulative_tokens = node.map(|p| p.cumulative_tokens).unwrap_or(0);
        let started_at = node.map(|p| p.started_at).unwrap_or(0);
        let elapsed_ms = live_elapsed_ms(node);
        let round = node.and_then(|p| p.round);
        let max_rounds = node.and_then(|p| p.max_rounds);
        let messages = node.map(|p| p.messages.clone()).unwrap_or_default();
        children.push(DirectChildResponse {
            agent_id: child.agent_id.as_str().to_string(),
            status: child.status,
            label: child.label.clone(),
            summary: child.summary.as_ref().map(|summary| summary.text.clone()),
            navigation_capability: cap,
            text_snapshot,
            cumulative_tokens,
            started_at,
            elapsed_ms,
            round,
            max_rounds,
            messages,
        });
    }
    Ok(LocalAgentViewResponse {
        self_view: SelfAgentResponse {
            agent_id: self_record.agent_id.as_str().to_string(),
            status: self_record.status,
            label: self_record.label,
            text_snapshot: self_node.and_then(|progress| progress.text_snapshot.clone()),
            cumulative_tokens: self_node
                .map(|progress| progress.cumulative_tokens)
                .unwrap_or(0),
            started_at: self_node.map(|progress| progress.started_at).unwrap_or(0),
            elapsed_ms: live_elapsed_ms(self_node),
            round: self_node.and_then(|progress| progress.round),
            max_rounds: self_node.and_then(|progress| progress.max_rounds),
            messages: self_node
                .map(|progress| progress.messages.clone())
                .unwrap_or_default(),
        },
        children,
    })
}

/// Resolves an opaque navigation capability and reconstructs a read-only UI
/// context from the canonical hierarchy record.
pub(super) async fn resolve_navigation_context(
    coordinator: &crate::agent::AgentCoordinator,
    capability_service: &crate::agent::capability::CapabilityService,
    capability: &str,
    viewer: &crate::agent::capability::ViewerId,
    session_id: &str,
) -> Result<crate::agent::AgentExecutionContext, StatusCode> {
    let resolved = capability_service
        .resolve_navigation(capability, viewer.as_str(), session_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let session = crate::agent::SessionId::new(session_id);
    let record = coordinator
        .trusted_ui_record(&session, &resolved.target)
        .await
        .map_err(map_scoped_coordinator_error)?;
    if record.generation != resolved.generation {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(crate::agent::AgentExecutionContext {
        session_id: record.session_id,
        agent_id: record.agent_id,
        parent_id: record.parent_id,
        depth: record.depth,
        // This context is used only for trusted, read-only UI projection. It
        // must not borrow cancellation authority from an unrelated ancestor.
        cancellation: tokio_util::sync::CancellationToken::new(),
    })
}

/// `GET /api/v1/agents/self?session_id=<id>` -- root local view (self + direct
/// children).
pub async fn get_agent_self(
    State(state): State<Arc<DaemonState>>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Result<Json<LocalAgentViewResponse>, StatusCode> {
    let viewer = resolve_viewer_from_headers(&state, &headers)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;
    // Server-side path: approvals/rules must belong to a real session
    // (design §4) — missing session_id is a client bug, not a default.
    let session_id = params
        .get("session_id")
        .map(|s| s.as_str())
        .ok_or(StatusCode::BAD_REQUEST)?;
    let root = state
        .root_context(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let view = build_local_view(&state, &root, &viewer).await?;
    Ok(Json(view))
}

/// `GET /api/v1/agents/children?session_id=<id>` -- alias for the root local
/// view, kept for the route shape in the plan.
pub async fn get_agent_children(
    State(state): State<Arc<DaemonState>>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Result<Json<LocalAgentViewResponse>, StatusCode> {
    get_agent_self(State(state), Query(params), headers).await
}

/// Converts a store directory entry into its DTO form, recursively
/// cross-filling progress metrics (tokens/timers/rounds) from the session
/// progress map. Entries without progress (typically the root) get defaults.
pub(super) fn to_directory_entry(
    entry: crate::agent::store::DirectoryEntry,
    progress: &HashMap<String, crate::agent::SubagentProgress>,
) -> AgentDirectoryEntry {
    let node = progress.get(entry.agent_id.as_str());
    let children = entry
        .children
        .into_iter()
        .map(|child| to_directory_entry(child, progress))
        .collect();
    AgentDirectoryEntry {
        agent_id: entry.agent_id.as_str().to_string(),
        status: entry.status,
        label: entry.label,
        summary: entry.summary,
        cumulative_tokens: node.map(|p| p.cumulative_tokens).unwrap_or(0),
        started_at: node.map(|p| p.started_at).unwrap_or(0),
        elapsed_ms: live_elapsed_ms(node),
        round: node.and_then(|p| p.round),
        max_rounds: node.and_then(|p| p.max_rounds),
        depth: entry.depth,
        children,
    }
}

/// `GET /api/v1/agents/directory?session_id=<id>` -- recursive read-only
/// whole-tree projection for panel polling. Same trust gate as
/// `get_agent_self` (viewer token, then session); progress cross-fill happens
/// here rather than in the store.
pub async fn get_agent_directory(
    State(state): State<Arc<DaemonState>>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Result<Json<AgentDirectoryResponse>, StatusCode> {
    resolve_viewer_from_headers(&state, &headers)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;
    // Server-side path: the directory must belong to a real session —
    // missing session_id is a client bug, not a default.
    let session_id = params
        .get("session_id")
        .map(|s| s.as_str())
        .ok_or(StatusCode::BAD_REQUEST)?;
    let root = state
        .root_context(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    // Clone only this session's progress so the read guard is released
    // before the recursive store walk below (pattern: `build_local_view`).
    let session_progress = {
        let progress_store = state.subagent_progress.read().await;
        progress_store
            .get(root.session_id.as_str())
            .cloned()
            .unwrap_or_default()
    };
    let directory = state
        .coordinator
        .trusted_ui_directory(&root.session_id, &root.agent_id)
        .await
        .map_err(map_scoped_coordinator_error)?;
    Ok(Json(AgentDirectoryResponse {
        session_id: root.session_id.as_str().to_string(),
        root: to_directory_entry(directory, &session_progress),
    }))
}

/// `GET /api/v1/agents/children/:capability?session_id=<id>` -- navigate into
/// the direct child bound by `capability`. Returns that child's local view
/// (self + its direct children), with fresh navigate capabilities.
pub async fn navigate_agent_view(
    State(state): State<Arc<DaemonState>>,
    Path(capability): Path<String>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Result<Json<LocalAgentViewResponse>, StatusCode> {
    let viewer = resolve_viewer_from_headers(&state, &headers)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;
    // Server-side path: approvals/rules must belong to a real session
    // (design §4) — missing session_id is a client bug, not a default.
    let session_id = params
        .get("session_id")
        .map(|s| s.as_str())
        .ok_or(StatusCode::BAD_REQUEST)?;
    let target_context = resolve_navigation_context(
        &state.coordinator,
        &state.capability_service,
        &capability,
        &viewer,
        session_id,
    )
    .await?;
    let target_view = build_local_view(&state, &target_context, &viewer).await?;
    Ok(Json(target_view))
}

/// `GET /api/v1/agents/children/:capability/transcript?session_id=<id>` --
/// read the transcript of the direct child bound by `capability`.
pub async fn get_child_transcript(
    State(state): State<Arc<DaemonState>>,
    Path(capability): Path<String>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let viewer = resolve_viewer_from_headers(&state, &headers)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;
    // Server-side path: approvals/rules must belong to a real session
    // (design §4) — missing session_id is a client bug, not a default.
    let session_id = params
        .get("session_id")
        .map(|s| s.as_str())
        .ok_or(StatusCode::BAD_REQUEST)?;
    let root = state
        .root_context(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let view = state
        .coordinator
        .list_local(&root)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    for child in &view.children {
        let req = crate::agent::capability::CapabilityRequest::transcript(
            viewer.as_str(),
            session_id,
            child.agent_id.as_str(),
            0,
        );
        if state
            .capability_service
            .verify(&capability, &req)
            .await
            .is_ok()
        {
            let transcript = state
                .coordinator
                .read_transcript(&root, child.agent_id.clone())
                .await
                .map_err(|_| StatusCode::NOT_FOUND)?;
            return Ok(Json(serde_json::json!({ "transcript": transcript })));
        }
    }
    // Indistinguishable denial.
    Err(StatusCode::NOT_FOUND)
}

/// `POST /api/v1/agents/children/:capability/cancel?session_id=<id>` --
/// cancel the direct child bound by `capability`.
pub async fn cancel_child(
    State(state): State<Arc<DaemonState>>,
    Path(capability): Path<String>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Result<StatusCode, StatusCode> {
    let viewer = resolve_viewer_from_headers(&state, &headers)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;
    // Server-side path: approvals/rules must belong to a real session
    // (design §4) — missing session_id is a client bug, not a default.
    let session_id = params
        .get("session_id")
        .map(|s| s.as_str())
        .ok_or(StatusCode::BAD_REQUEST)?;
    let root = state
        .root_context(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let view = state
        .coordinator
        .list_local(&root)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    for child in &view.children {
        let req = crate::agent::capability::CapabilityRequest::cancel(
            viewer.as_str(),
            session_id,
            child.agent_id.as_str(),
            0,
        );
        if state
            .capability_service
            .verify(&capability, &req)
            .await
            .is_ok()
        {
            let result = state
                .coordinator
                .cancel_subtree(&root, child.agent_id.clone())
                .await;
            return match result {
                Ok(()) => Ok(StatusCode::NO_CONTENT),
                Err(_) => Err(StatusCode::NOT_FOUND),
            };
        }
    }
    Err(StatusCode::NOT_FOUND)
}

/// `POST /api/v1/agents/task-groups/claim` -- atomically claim one ready
/// root-direct task group for the persistent main agent. Returns `200` with a
/// delivery when a ready group exists, or `204 No Content` when nothing is
/// ready. Atomicity is coordinator-owned: concurrent claims deliver a group at
/// most once.
pub async fn claim_task_group(
    State(state): State<Arc<DaemonState>>,
    Json(body): Json<ClaimTaskGroupRequest>,
) -> Result<Json<TaskGroupDeliveryResponse>, StatusCode> {
    let root = state
        .root_context(&body.session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let delivery = state
        .coordinator
        .claim_ready_root_group(&root, body.generation)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    match delivery {
        Some(d) => {
            // Broadcast the claimed group's result summary on the global bus;
            // `project` lets multi-project clients filter (design §10).
            let project = state.effective_session_root(&body.session_id).await;
            state.broadcast_global(
                crate::daemon::global_events::GlobalEventKind::TaskGroupResult,
                serde_json::json!({
                    "task_group_id": d.group_id.as_str(),
                    "session_id": body.session_id,
                    "generation": d.generation,
                    "project": project,
                    "result_count": d.results.len(),
                    "statuses": d.results.iter().map(|r| r.status).collect::<Vec<_>>(),
                }),
            );
            Ok(Json(TaskGroupDeliveryResponse {
                group_id: d.group_id.as_str().to_string(),
                generation: d.generation,
                results: d.results,
            }))
        }
        None => Err(StatusCode::NO_CONTENT),
    }
}

/// `POST /api/v1/agents/generation/reset` -- advance the session generation
/// and cancel obsolete root-direct subtrees. The old generation's ready groups
/// are no longer deliverable; in-flight root children are cancelled
/// bottom-up. Returns the new generation.
pub async fn reset_agent_generation(
    State(state): State<Arc<DaemonState>>,
    Json(body): Json<ResetAgentGenerationRequest>,
) -> Result<Json<ResetAgentGenerationResponse>, StatusCode> {
    let root = state
        .root_context(&body.session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    // Cancel obsolete root-direct children before advancing so their permits
    // are released and their terminals persisted as Cancelled.
    let _ = state.coordinator.cancel_root_children(&root).await;
    let old_generation = state.coordinator.current_generation(&root.session_id).await;
    let _ = state
        .coordinator
        .cancel_generation(&root.session_id, old_generation)
        .await;
    let new_generation = state.coordinator.advance_generation(&root.session_id).await;
    Ok(Json(ResetAgentGenerationResponse {
        generation: new_generation,
    }))
}

/// `POST /api/v1/agents/session/cancel` -- cancel the entire agent session:
/// resolve the trusted root, cancel its live subtrees bottom-up, await handles
/// with the shutdown timeout, persist `Cancelled` descendants, and release
/// every permit. Used on application shutdown so no subagent outlives the
/// session.
pub async fn cancel_agent_session(
    State(state): State<Arc<DaemonState>>,
    Json(body): Json<ResetAgentGenerationRequest>,
) -> Result<StatusCode, StatusCode> {
    let root = state
        .root_context(&body.session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let _ = state.coordinator.cancel_root_children(&root).await;
    Ok(StatusCode::NO_CONTENT)
}

// ── Memory ops API (Tier 2 web-ops-console) ──────────────────────────────────
// Thin wrappers over the shared MemoryManager. All read-only except prune.
