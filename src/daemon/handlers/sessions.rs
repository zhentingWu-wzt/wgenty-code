//! Session CRUD and search handlers.

use super::*;

/// Map a stored [`SessionInfo`] to its API response shape (shared by the list
/// and search handlers).
pub(super) fn session_info_response(
    s: crate::context::memory_session::SessionInfo,
) -> SessionInfoResponse {
    SessionInfoResponse {
        id: s.id,
        name: s.name,
        project_path: s.project_path.map(|p| p.to_string_lossy().to_string()),
        created_at: s.created_at.to_rfc3339(),
        updated_at: s.updated_at.to_rfc3339(),
        message_count: s.message_count,
        status: format!("{:?}", s.status),
        worktree: s.worktree.map(|w| WorktreeRef {
            path: w.path,
            branch: w.branch,
        }),
    }
}

/// Sessions stored under a non-main project's directory but lacking
/// `project_path` (e.g. moved there manually) still group under that project.
pub(super) fn fill_project(
    root: &std::path::Path,
    sessions: &mut [crate::context::memory_session::SessionInfo],
) {
    for s in sessions.iter_mut() {
        if s.project_path.is_none() {
            s.project_path = Some(root.to_path_buf());
        }
    }
}

pub async fn list_sessions(
    State(state): State<Arc<DaemonState>>,
) -> Result<Json<Vec<SessionInfoResponse>>, StatusCode> {
    // Aggregate the main project and every registered project.
    let mut sessions = state
        .session_manager
        .list()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    for root in state.projects.registered_roots() {
        let mgr = state.session_manager_for_project(&root).await;
        let mut project_sessions = mgr
            .list()
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        fill_project(&root, &mut project_sessions);
        sessions.extend(project_sessions);
    }

    Ok(Json(
        sessions.into_iter().map(session_info_response).collect(),
    ))
}

pub async fn create_session(
    State(state): State<Arc<DaemonState>>,
    Json(body): Json<CreateSessionRequest>,
) -> Result<Json<SessionResponse>, (StatusCode, String)> {
    // Route to the owning project's session store (default: main project).
    let root = match &body.project_path {
        Some(p) => state.projects.resolve(p).ok_or((
            StatusCode::BAD_REQUEST,
            format!("not a registered project: {p}"),
        ))?,
        None => state.projects.main_root(),
    };
    let mgr = state.session_manager_for_project(&root).await;
    let session =
        crate::context::memory_session::Session::new(body.name.as_deref()).with_project(root);
    mgr.save(&session)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(SessionResponse {
        worktree: crate::context::memory_session::worktree_of(&session).map(|w| WorktreeRef {
            path: w.path,
            branch: w.branch,
        }),
        id: session.id,
        name: session.name,
        created_at: session.created_at.to_rfc3339(),
        updated_at: session.updated_at.to_rfc3339(),
        version: session.version,
        messages: session.messages,
        ui_messages: session.ui_messages,
    }))
}

pub async fn get_session(
    State(state): State<Arc<DaemonState>>,
    Path(id): Path<String>,
) -> Result<Json<SessionResponse>, StatusCode> {
    let (_mgr, session) = state
        .resolve_session(&id)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(SessionResponse {
        worktree: crate::context::memory_session::worktree_of(&session).map(|w| WorktreeRef {
            path: w.path,
            branch: w.branch,
        }),
        id: session.id,
        name: session.name,
        created_at: session.created_at.to_rfc3339(),
        updated_at: session.updated_at.to_rfc3339(),
        version: session.version,
        messages: session.messages,
        ui_messages: session.ui_messages,
    }))
}

pub async fn update_session(
    State(state): State<Arc<DaemonState>>,
    Path(id): Path<String>,
    Json(body): Json<UpdateSessionRequest>,
) -> Result<Json<SessionResponse>, (StatusCode, Json<serde_json::Value>)> {
    // Serialize the load → expected_version check → save sequence: without
    // this lock two concurrent PUTs with the same expected_version can both
    // read the pre-write version, both pass the check, and both "succeed" at
    // the same new version (observed in the T17 acceptance test). Held across
    // the disk I/O below; saves are small and infrequent, so a single global
    // lock costs nothing measurable on a loopback daemon.
    let _update_guard = state.session_update_lock.lock().await;

    // Run lock: mutating a session mid-run would race the run's final save.
    if state.session_runs.is_active(&id) {
        return Err((
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error": "run active"})),
        ));
    }

    // Route to the owning project's store; an unknown id upserts into the
    // main project (legacy TUI behavior).
    let (mgr, mut session) = match state.resolve_session(&id).await {
        Some((mgr, session)) => (mgr, session),
        // Upsert must preserve the path id. Session::new() mints a fresh UUID and
        // previously caused every SaveSession to write a new file (duplicate names
        // in the session panel) while the TUI continued using the original id.
        None => (
            state.session_manager.clone(),
            crate::context::memory_session::Session::with_id(id.clone(), None),
        ),
    };

    // Defense in depth: even if a future constructor changes, never let the
    // on-disk / response id diverge from the request path.
    session.id = id;

    // Optimistic concurrency guard: `Some(v)` must match the stored version
    // (read via `load`, i.e. the real on-disk value, not the lazy index);
    // `None` keeps legacy last-write-wins behavior.
    if let Some(expected) = body.expected_version {
        if expected != session.version {
            return Err((
                StatusCode::CONFLICT,
                Json(serde_json::json!({
                    "error": "version conflict",
                    "current_version": session.version,
                })),
            ));
        }
    }

    if let Some(name) = &body.name {
        session.name = name.clone();
    }
    if let Some(messages) = body.messages {
        session.messages = messages;
    }
    if let Some(ui_messages) = body.ui_messages {
        session.ui_messages = ui_messages;
    }
    session.updated_at = chrono::Utc::now();
    // Every persisted write advances the optimistic-concurrency version.
    session.version += 1;
    // Fully materialised write — clear any lazy index marker.
    session.lazy_message_count = None;

    mgr.save(&session).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
    })?;

    Ok(Json(SessionResponse {
        worktree: crate::context::memory_session::worktree_of(&session).map(|w| WorktreeRef {
            path: w.path,
            branch: w.branch,
        }),
        id: session.id,
        name: session.name,
        created_at: session.created_at.to_rfc3339(),
        updated_at: session.updated_at.to_rfc3339(),
        version: session.version,
        messages: session.messages,
        ui_messages: session.ui_messages,
    }))
}

pub async fn delete_session(
    State(state): State<Arc<DaemonState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let mgr = match state.resolve_session(&id).await {
        Some((mgr, _)) => mgr,
        // Unknown id: keep the legacy error semantics of the main store.
        None => state.session_manager.clone(),
    };
    match mgr.delete(&id).await {
        Ok(()) => Ok(Json(serde_json::json!({"success": true}))),
        Err(e) => {
            if e.to_string().contains("Invalid session ID") {
                Err(StatusCode::BAD_REQUEST)
            } else {
                Err(StatusCode::INTERNAL_SERVER_ERROR)
            }
        }
    }
}

pub async fn search_sessions(
    State(state): State<Arc<DaemonState>>,
    Query(query): Query<SearchSessionsQuery>,
) -> Result<Json<Vec<SessionInfoResponse>>, StatusCode> {
    let mut sessions = state.session_manager.search(&query.q).await;
    for root in state.projects.registered_roots() {
        let mgr = state.session_manager_for_project(&root).await;
        let mut project_sessions = mgr.search(&query.q).await;
        fill_project(&root, &mut project_sessions);
        sessions.extend(project_sessions);
    }

    Ok(Json(
        sessions.into_iter().map(session_info_response).collect(),
    ))
}

// ── Undo ───────────────────────────────────────────────────────────────────
