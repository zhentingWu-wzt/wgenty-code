//! Checkpoint/undo handlers.

use super::*;

pub async fn undo_checkpoint(State(state): State<Arc<DaemonState>>) -> Result<String, StatusCode> {
    match state.checkpoint_manager.undo(None).await {
        Ok(output) => Ok(output),
        Err(e) => {
            tracing::warn!(error = %e, "undo_checkpoint failed");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// Query for `GET /api/v1/checkpoints/:id/diff`.
#[derive(serde::Deserialize)]
pub struct CheckpointDiffQuery {
    /// Project whose checkpoint store holds the turn (`None` = main project).
    #[serde(default)]
    pub project: Option<String>,
}

/// GET /api/v1/checkpoints/:id/diff - preview what undoing the turn would
/// change, WITHOUT touching the workspace. `saved` files carry a unified diff
/// of (blob → current on-disk content); `tombstone` files that currently
/// exist are flagged `will_delete` (rewind removes them); `skipped` files
/// (binary/oversize at capture time) are flagged `skipped`. Read-only.
pub async fn checkpoint_diff(
    State(state): State<Arc<DaemonState>>,
    axum::extract::Path(turn_id): axum::extract::Path<String>,
    Query(q): Query<CheckpointDiffQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let manager = match &q.project {
        Some(p) => {
            let root = state.projects.resolve(p).ok_or((
                StatusCode::BAD_REQUEST,
                format!("not a registered project: {p}"),
            ))?;
            state.checkpoints_for_project(&root).0
        }
        None => state.checkpoint_manager.clone(),
    };
    match manager.preview_diff(&turn_id).await {
        Ok(entries) => Ok(Json(serde_json::json!({
            "turn_id": turn_id,
            "files": entries,
        }))),
        Err(e) => Err((StatusCode::NOT_FOUND, e)),
    }
}

/// Request body for `POST /api/v1/tools/undo-turn`.
///
/// `turn_ids` is oldest-first; the store rewinds them newest-first so the
/// oldest turn's pre-edit content wins for files edited across multiple turns.
#[derive(serde::Deserialize)]
pub struct UndoTurnRangeRequest {
    pub turn_ids: Vec<String>,
    /// Project whose checkpoint store holds the turns (`None` = main project).
    #[serde(default)]
    pub project: Option<String>,
}

/// POST /api/v1/tools/undo-turn - rewind a range of turns, restoring files to
/// the state before the oldest turn in the range. Backs the TUI `/undo`
/// code-rollback flow (roll back all turns after the kept turn N).
pub async fn undo_turn_range(
    State(state): State<Arc<DaemonState>>,
    Json(body): Json<UndoTurnRangeRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let manager = match &body.project {
        Some(p) => {
            let root = state.projects.resolve(p).ok_or((
                StatusCode::BAD_REQUEST,
                format!("not a registered project: {p}"),
            ))?;
            state.checkpoints_for_project(&root).0
        }
        None => state.checkpoint_manager.clone(),
    };
    match manager.undo_range(body.turn_ids).await {
        Ok(report) => Ok(Json(serde_json::json!({
            "restored": report.restored,
            "skipped": report.skipped,
            "failed": report.failed,
            "rewound_turns": report.rewound_turns,
        }))),
        Err(e) => {
            tracing::warn!(error = %e, "undo_turn_range failed");
            Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
        }
    }
}

/// Query for `GET /api/v1/checkpoints`.
#[derive(serde::Deserialize)]
pub struct ListCheckpointsQuery {
    /// Project whose checkpoints to list (`None` = main project).
    #[serde(default)]
    pub project: Option<String>,
}

/// GET /api/v1/checkpoints - list per-turn checkpoint snapshots (newest-first).
/// Each entry carries the turn id, creation timestamp, file count, the user
/// input summary (when recorded) and the touched file list, so clients can
/// render "what this turn changed" instead of a bare UUID.
pub async fn list_checkpoints(
    State(state): State<Arc<DaemonState>>,
    Query(q): Query<ListCheckpointsQuery>,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, String)> {
    let manager = match &q.project {
        Some(p) => {
            let root = state.projects.resolve(p).ok_or((
                StatusCode::BAD_REQUEST,
                format!("not a registered project: {p}"),
            ))?;
            state.checkpoints_for_project(&root).0
        }
        None => state.checkpoint_manager.clone(),
    };
    match manager.store().list() {
        Ok(infos) => Ok(Json(
            infos
                .into_iter()
                .map(|i| {
                    serde_json::json!({
                        "turn_id": i.turn_id,
                        "created_at": i.created_at,
                        "file_count": i.file_count,
                        "user_summary": i.user_summary,
                        "files": i.files.iter().map(|f| serde_json::json!({
                            "path": f.path,
                            "state": f.state,
                        })).collect::<Vec<_>>(),
                    })
                })
                .collect(),
        )),
        Err(e) => {
            tracing::warn!(error = %e, "list_checkpoints failed");
            Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
        }
    }
}

// ── Scoped agent APIs (strict subagent isolation) ────────────────────────────
//
// These handlers replace the flat `/api/v1/subagent/progress` endpoint with
// capability-scoped local views. Every denial (missing/unknown viewer token,
// expired/wrong-viewer/wrong-session/wrong-target capability, hidden target)
// maps to one stable 404 with no target details, so denials are
// indistinguishable.
