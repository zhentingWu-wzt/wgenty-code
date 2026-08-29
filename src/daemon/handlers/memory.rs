//! Cross-session memory CRUD handlers.

use super::*;

/// Resolve the memory pool for an optional `project` query param (`None` =
/// main project). Shared by all memory endpoints.
async fn memory_manager_for(
    state: &DaemonState,
    project: Option<&str>,
) -> Result<Arc<crate::context::MemoryManager>, (StatusCode, String)> {
    match project {
        Some(p) => {
            let root = state.projects.resolve(p).ok_or((
                StatusCode::BAD_REQUEST,
                format!("not a registered project: {p}"),
            ))?;
            Ok(state.memory_router.for_project(&root).await)
        }
        None => Ok(state.memory_manager.clone()),
    }
}

/// `GET /api/v1/memory/status` — dual-pool status summary.
pub async fn memory_status(
    State(state): State<Arc<DaemonState>>,
    Query(q): Query<MemoryProjectQuery>,
) -> Result<Json<crate::context::MemoryStatus>, (StatusCode, String)> {
    let mgr = memory_manager_for(&state, q.project.as_deref()).await?;
    let status = mgr.status().await.map_err(|e| {
        error!(error = ?e, "memory status failed");
        (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    })?;
    Ok(Json(status))
}

/// `GET /api/v1/memory` — list with optional scope/min_importance/limit filters.
pub async fn list_memory(
    State(state): State<Arc<DaemonState>>,
    Query(q): Query<MemoryListQuery>,
) -> Result<Json<MemoryListResponse>, (StatusCode, String)> {
    let mgr = memory_manager_for(&state, q.project.as_deref()).await?;
    let limit = q.limit.unwrap_or(500).clamp(1, 5000);
    let entries = mgr.list_memories(q.min_importance, limit).await;

    let items: Vec<MemoryItemResponse> = entries
        .into_iter()
        .filter(|(origin, _)| match q.scope.as_deref() {
            Some("project") => matches!(origin, crate::context::MemoryOrigin::Project),
            Some("global") => matches!(origin, crate::context::MemoryOrigin::Global),
            _ => true, // "all" or unspecified
        })
        .map(|(origin, entry)| MemoryItemResponse {
            origin: origin_str(origin).to_string(),
            entry,
        })
        .collect();
    let total = items.len();
    Ok(Json(MemoryListResponse { items, total }))
}

/// `GET /api/v1/memory/:id` — single memory with origin.
pub async fn get_memory(
    State(state): State<Arc<DaemonState>>,
    Path(id): Path<String>,
    Query(q): Query<MemoryProjectQuery>,
) -> Result<Json<MemoryDetailResponse>, (StatusCode, String)> {
    // get_memory doesn't return origin; re-derive it by checking both pools.
    let mgr = memory_manager_for(&state, q.project.as_deref()).await?;
    if let Some(entry) = mgr.get_memory(&id).await {
        // list_memories gives us the origin mapping cheaply for the lookup.
        let origin = mgr
            .list_memories(None, 5000)
            .await
            .into_iter()
            .find(|(_, e)| e.id == id)
            .map(|(o, _)| origin_str(o).to_string())
            .unwrap_or_else(|| "project".to_string());
        Ok(Json(MemoryDetailResponse { origin, entry }))
    } else {
        Err((StatusCode::NOT_FOUND, format!("no such memory: {id}")))
    }
}

/// `DELETE /api/v1/memory/:id` — delete a single memory item by id.
///
/// Requires `?origin=project|global` to select the correct pool. When
/// `origin=project`, an optional `&project=<path>` narrows the project pool.
pub async fn delete_memory(
    State(state): State<Arc<DaemonState>>,
    Path(id): Path<String>,
    Query(q): Query<DeleteMemoryQuery>,
) -> Result<StatusCode, (StatusCode, String)> {
    let origin = match q.origin.as_str() {
        "global" => crate::context::MemoryOrigin::Global,
        "project" | "" => crate::context::MemoryOrigin::Project,
        other => {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("invalid origin '{other}': expected 'project' or 'global'"),
            ));
        }
    };
    let mgr = memory_manager_for(&state, q.project.as_deref()).await?;
    let deleted = mgr
        .delete_memory(origin, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err((StatusCode::NOT_FOUND, format!("no such memory: {id}")))
    }
}

/// `POST /api/v1/memory/prune` — invoke prune; dry_run is advisory (the
/// underlying prune() always executes, so a true dry-run requires a manager
/// change — for now we honor the flag by returning status without pruning).
pub async fn prune_memory(
    State(state): State<Arc<DaemonState>>,
    Query(q): Query<MemoryProjectQuery>,
    req: Option<Json<PruneRequest>>,
) -> Result<Json<crate::context::PruneResult>, (StatusCode, String)> {
    let mgr = memory_manager_for(&state, q.project.as_deref()).await?;
    let dry_run = req.map(|b| b.dry_run).unwrap_or(false);
    // Dry-run: return the would-be result by reading status deltas. The
    // current MemoryManager::prune is destructive with no preview, so we
    // approximate dry-run as a no-op returning current counts as before==after.
    if dry_run {
        let s = mgr.status().await.map_err(|e| {
            error!(error = ?e, "memory status (dry-run prune) failed");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?;
        return Ok(Json(crate::context::PruneResult {
            before: s.total_memories,
            after: s.total_memories,
            removed: 0,
            project_before: s.project_count,
            project_after: s.project_count,
            global_before: s.global_count,
            global_after: s.global_count,
        }));
    }
    let result = mgr.prune().await.map_err(|e| {
        error!(error = ?e, "memory prune failed");
        (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    })?;
    Ok(Json(result))
}

/// Project MemoryOrigin to its serialized string form.
pub(super) fn origin_str(o: crate::context::MemoryOrigin) -> &'static str {
    match o {
        crate::context::MemoryOrigin::Project => "project",
        crate::context::MemoryOrigin::Global => "global",
    }
}

// ── Thin-Client Heartbeat ───────────────────────────────────────────────────
