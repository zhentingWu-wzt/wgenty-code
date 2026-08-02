//! Session worktree binding + archive endpoints (project v1: single repo).
//!
//! Binding contract (docs/superpowers/specs/2026-08-02-session-worktree-binding-design.md):
//! N:1 — multiple sessions may bind the same worktree; unbound sessions run in
//! the main working_dir. Bindings persist in `Session.metadata["worktree"]`.
//! Archive uses the existing `SessionStatus::Archived` (list visibility flag).

use crate::context::memory_session::SessionWorktree;
use crate::daemon::models::WorktreeRef;
use crate::daemon::state::DaemonState;
use axum::{
    extract::{Path as AxumPath, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub struct BindWorktreeRequest {
    pub path: String,
    pub branch: String,
}

#[derive(Debug, Serialize)]
pub struct BindWorktreeResponse {
    pub session_id: String,
    pub worktree: WorktreeRef,
}

#[derive(Debug, Deserialize)]
pub struct SetArchivedRequest {
    pub archived: bool,
}

#[derive(Debug, Serialize)]
pub struct SetArchivedResponse {
    pub session_id: String,
    pub archived: bool,
}

/// Canonicalize `input` (joined to `root` when relative) and require it to
/// exist and stay inside `root`. Err carries a client-facing message.
pub(crate) fn resolve_worktree_path(root: &Path, input: &str) -> Result<PathBuf, String> {
    let root = root
        .canonicalize()
        .map_err(|e| format!("bad working_dir: {e}"))?;
    let joined = if Path::new(input).is_absolute() {
        PathBuf::from(input)
    } else {
        root.join(input)
    };
    let canon = joined
        .canonicalize()
        .map_err(|_| format!("worktree does not exist: {input}"))?;
    if !canon.starts_with(&root) {
        return Err(format!("worktree path escapes the project: {input}"));
    }
    Ok(canon)
}

fn bad_request(msg: String) -> (StatusCode, String) {
    (StatusCode::BAD_REQUEST, msg)
}

/// PUT /api/v1/sessions/:id/worktree — bind a session to a worktree (N:1).
pub async fn bind_worktree(
    State(state): State<Arc<DaemonState>>,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<BindWorktreeRequest>,
) -> Result<Json<BindWorktreeResponse>, (StatusCode, String)> {
    let canon = resolve_worktree_path(&state.app_state.settings.storage.working_dir, &body.path)
        .map_err(bad_request)?;

    // The daemon session must exist (single identity: this id is also the
    // /tools/execute session_id).
    let exists = state.session_manager.get(&id).await.is_some();
    if !exists {
        return Err((StatusCode::NOT_FOUND, format!("no such session: {id}")));
    }

    let wt = SessionWorktree {
        path: canon.to_string_lossy().to_string(),
        branch: body.branch,
    };
    state.bind_session_worktree(&id, canon);
    state
        .session_manager
        .set_metadata(
            &id,
            "worktree",
            // A two-field plain struct — serialization cannot fail.
            Some(serde_json::to_value(&wt).expect("SessionWorktree serialization is infallible")),
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(BindWorktreeResponse {
        session_id: id,
        worktree: WorktreeRef {
            path: wt.path,
            branch: wt.branch,
        },
    }))
}

/// DELETE /api/v1/sessions/:id/worktree — unbind (session returns to the main
/// checkout; the on-disk worktree is untouched). Idempotent 204.
pub async fn unbind_worktree(
    State(state): State<Arc<DaemonState>>,
    AxumPath(id): AxumPath<String>,
) -> StatusCode {
    state.unbind_session_worktree(&id);
    let _ = state
        .session_manager
        .set_metadata(&id, "worktree", None)
        .await;
    StatusCode::NO_CONTENT
}

/// PUT /api/v1/sessions/:id/archive — set/clear the archived flag. Archived
/// sessions are hidden from default list views (client-side filtering).
pub async fn set_archived(
    State(state): State<Arc<DaemonState>>,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<SetArchivedRequest>,
) -> Result<Json<SetArchivedResponse>, (StatusCode, String)> {
    let exists = state.session_manager.get(&id).await.is_some();
    if !exists {
        return Err((StatusCode::NOT_FOUND, format!("no such session: {id}")));
    }
    if body.archived {
        state
            .session_manager
            .archive(&id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    } else {
        state
            .session_manager
            .unarchive(&id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }
    Ok(Json(SetArchivedResponse {
        session_id: id,
        archived: body.archived,
    }))
}

/// Rebuild the in-memory binding map from persisted session metadata. Called
/// once at daemon startup after `load_index` so bindings survive restarts.
pub async fn reconcile_worktree_bindings(state: &DaemonState) {
    match state.session_manager.list().await {
        Ok(sessions) => {
            for info in sessions {
                if let Some(wt) = info.worktree {
                    state.bind_session_worktree(&info.id, PathBuf::from(wt.path));
                }
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "reconcile_worktree_bindings failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::memory_session::worktree_of;

    #[test]
    fn resolve_accepts_existing_path_inside_root() {
        let dir = tempfile::tempdir().unwrap();
        let wt = dir.path().join(".worktrees").join("a");
        std::fs::create_dir_all(&wt).unwrap();
        let resolved = resolve_worktree_path(dir.path(), ".worktrees/a").unwrap();
        assert_eq!(resolved, wt.canonicalize().unwrap());
    }

    #[test]
    fn resolve_rejects_escape_and_outside() {
        let dir = tempfile::tempdir().unwrap();
        assert!(resolve_worktree_path(dir.path(), "../outside").is_err());
        assert!(resolve_worktree_path(dir.path(), "/etc").is_err());
    }

    #[test]
    fn resolve_rejects_nonexistent() {
        let dir = tempfile::tempdir().unwrap();
        assert!(resolve_worktree_path(dir.path(), ".worktrees/nope").is_err());
    }

    #[test]
    fn worktree_metadata_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = crate::context::memory_session::SessionManager::with_project_root(
            dir.path().to_path_buf(),
        );
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let s = mgr.create(Some("t")).await.unwrap();
            let wt = SessionWorktree {
                path: "/repo/.worktrees/a".to_string(),
                branch: "a".to_string(),
            };
            assert!(mgr
                .set_metadata(&s.id, "worktree", Some(serde_json::to_value(&wt).unwrap()))
                .await
                .unwrap());
            let loaded = mgr.get(&s.id).await.unwrap();
            assert_eq!(worktree_of(&loaded), Some(wt));

            // Clearing removes the key.
            assert!(mgr.set_metadata(&s.id, "worktree", None).await.unwrap());
            let loaded = mgr.get(&s.id).await.unwrap();
            assert_eq!(worktree_of(&loaded), None);

            // Unknown session → false.
            assert!(!mgr.set_metadata("nope", "k", None).await.unwrap());
        });
    }
}
