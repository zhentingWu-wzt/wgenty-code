//! Playground registry (web "playground plaza").
//!
//! Playgrounds are scratch workspaces decoupled from every project: each one
//! is an auto-created directory under the OS temp dir (`wgenty-playground-*`)
//! whose sessions, memory, checkpoints, and tasks live entirely inside it
//! (`<playground>/.wgenty-code/…`) — deleting the directory discards the
//! whole playground. Like the project registry, entries persist (to
//! `~/.wgenty-code/playgrounds.json`) and vanish-on-load entries are pruned;
//! unlike projects, `DELETE` also removes the on-disk directory when (and
//! only when) it is a daemon-owned auto-created temp dir.

use axum::{
    extract::{Query, State},
    http::StatusCode,
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Directory-name prefix shared by every daemon-created playground.
pub const PLAYGROUND_PREFIX: &str = "wgenty-playground-";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlaygroundInfo {
    /// Canonicalized absolute playground root.
    pub path: PathBuf,
    /// Display name (directory basename).
    pub name: String,
    pub created_at: DateTime<Utc>,
}

/// Response DTO for `GET /api/v1/playgrounds`.
#[derive(Debug, Clone, Serialize)]
pub struct PlaygroundResponse {
    pub path: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug)]
struct Inner {
    playgrounds: Vec<PlaygroundInfo>,
    persist_path: PathBuf,
}

/// Cheaply cloneable registry handle; clones share the same state.
#[derive(Debug, Clone)]
pub struct PlaygroundRegistry {
    inner: Arc<std::sync::RwLock<Inner>>,
}

impl PlaygroundRegistry {
    /// Registry persisted at the default location
    /// (`<config_dir>/playgrounds.json`).
    pub fn load_default() -> Self {
        Self::load(crate::utils::config_dir().join("playgrounds.json"))
    }

    /// Load (or initialize) a registry persisted at `persist_path`, pruning
    /// entries whose directory vanished (the OS may clean temp dirs at any
    /// time — same tolerance as the project registry).
    pub fn load(persist_path: PathBuf) -> Self {
        let playgrounds = match std::fs::read_to_string(&persist_path) {
            Ok(content) => match serde_json::from_str::<Vec<PlaygroundInfo>>(&content) {
                Ok(mut list) => {
                    let before = list.len();
                    list.retain(|p| p.path.is_dir());
                    if list.len() != before {
                        tracing::info!(
                            dropped = before - list.len(),
                            "pruned stale playground registry entries"
                        );
                    }
                    list
                }
                Err(e) => {
                    tracing::warn!(error = %e, path = %persist_path.display(), "corrupt playgrounds.json; starting empty");
                    Vec::new()
                }
            },
            Err(_) => Vec::new(), // first run — no file yet
        };
        Self {
            inner: Arc::new(std::sync::RwLock::new(Inner {
                playgrounds,
                persist_path,
            })),
        }
    }

    /// Registered playgrounds in insertion order.
    pub fn list(&self) -> Vec<PlaygroundInfo> {
        self.inner
            .read()
            .expect("playground registry lock poisoned")
            .playgrounds
            .clone()
    }

    /// Create a fresh auto-named playground directory under `base` (the OS
    /// temp dir when `None`) and register it. Errors are client-facing
    /// strings.
    pub fn create(&self, base: Option<&Path>) -> Result<PlaygroundInfo, String> {
        let base = base.map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        // mkdir the base first: a missing parent (e.g. TMPDIR pointing at a
        // not-yet-created dir) must create the playground, not fail.
        std::fs::create_dir_all(&base).map_err(|e| format!("create temp dir: {e}"))?;
        let dir = base.join(format!(
            "{PLAYGROUND_PREFIX}{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).map_err(|e| format!("create playground: {e}"))?;
        let canon = dir
            .canonicalize()
            .map_err(|e| format!("canonicalize playground: {e}"))?;

        let info = PlaygroundInfo {
            name: dir_name(&canon),
            path: canon,
            created_at: Utc::now(),
        };
        let mut inner = self
            .inner
            .write()
            .expect("playground registry lock poisoned");
        // UUID collision with a registered entry — practically impossible,
        // but keep the registry consistent rather than assume.
        inner.playgrounds.retain(|p| p.path != info.path);
        inner.playgrounds.push(info.clone());
        persist_locked(&inner)?;
        Ok(info)
    }

    /// Unregister a playground. Returns the canonicalized directory when it
    /// is a daemon-owned auto-created temp dir the caller should delete;
    /// `Ok(None)` when it was never registered or should be left on disk.
    /// Persist failures surface as `Err`.
    pub fn remove(&self, path: &str) -> Result<Option<PathBuf>, String> {
        let canon = canonicalize_dir_lossy(path);
        let mut inner = self
            .inner
            .write()
            .expect("playground registry lock poisoned");
        let before = inner.playgrounds.len();
        inner.playgrounds.retain(|p| p.path != canon);
        if inner.playgrounds.len() == before {
            return Ok(None); // not registered
        }
        persist_locked(&inner)?;
        Ok(owned_temp_dir(&canon))
    }

    /// Whitelist check: canonicalize `path` and return it when it is a
    /// registered playground. `None` = unknown (reject).
    pub fn resolve(&self, path: &str) -> Option<PathBuf> {
        let canon = canonicalize_dir_lossy(path);
        let inner = self
            .inner
            .read()
            .expect("playground registry lock poisoned");
        if inner.playgrounds.iter().any(|p| p.path == canon) {
            Some(canon)
        } else {
            None
        }
    }

    /// Registered playground roots — used to fan out per-root session /
    /// workspace lookups.
    pub fn roots(&self) -> Vec<PathBuf> {
        self.inner
            .read()
            .expect("playground registry lock poisoned")
            .playgrounds
            .iter()
            .map(|p| p.path.clone())
            .collect()
    }
}

fn dir_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string())
}

/// Like the project registry's lossy canonicalize: tolerant of vanished
/// directories so `remove`/`resolve` still match deleted entries.
fn canonicalize_dir_lossy(path: &str) -> PathBuf {
    let p = PathBuf::from(path);
    p.canonicalize().unwrap_or(p)
}

/// A playground directory the daemon may safely delete: it must sit directly
/// under the canonicalized OS temp dir and carry the daemon-owned name
/// prefix. Anything else (moved, symlinked, hand-registered) is left alone —
/// the check runs on the canonicalized path, so symlinks resolve to their
/// target and a re-pointed name simply fails the parent check.
fn owned_temp_dir(canon: &Path) -> Option<PathBuf> {
    let temp = std::env::temp_dir().canonicalize().ok()?;
    if canon.parent() != Some(temp.as_path()) {
        return None;
    }
    let name = canon.file_name()?.to_string_lossy();
    name.starts_with(PLAYGROUND_PREFIX)
        .then(|| canon.to_path_buf())
}

/// Atomic persist: write tmp + rename so a crash mid-write can't corrupt the
/// registry. Called with the write lock held (mirrors the project registry).
fn persist_locked(inner: &Inner) -> Result<(), String> {
    let tmp = inner.persist_path.with_extension("json.tmp");
    let content = serde_json::to_string_pretty(&inner.playgrounds)
        .map_err(|e| format!("serialize playgrounds: {e}"))?;
    if let Some(parent) = inner.persist_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create config dir: {e}"))?;
    }
    std::fs::write(&tmp, content).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &inner.persist_path)
        .map_err(|e| format!("rename {}: {e}", inner.persist_path.display()))?;
    Ok(())
}

// ── HTTP endpoints ───────────────────────────────────────────────────────────

use crate::daemon::state::DaemonState;

fn to_response(info: &PlaygroundInfo) -> PlaygroundResponse {
    PlaygroundResponse {
        path: info.path.to_string_lossy().to_string(),
        name: info.name.clone(),
        created_at: info.created_at,
    }
}

/// GET /api/v1/playgrounds — registered playgrounds in insertion order.
pub async fn list_playgrounds(
    State(state): State<Arc<DaemonState>>,
) -> Json<Vec<PlaygroundResponse>> {
    Json(state.playgrounds.list().iter().map(to_response).collect())
}

/// POST /api/v1/playgrounds — create a fresh playground: a new
/// `wgenty-playground-<id>` directory under the OS temp dir. 201 with the
/// playground info; 400 when the directory cannot be created/persisted.
pub async fn create_playground(
    State(state): State<Arc<DaemonState>>,
) -> Result<(StatusCode, Json<PlaygroundResponse>), (StatusCode, String)> {
    let info = state
        .playgrounds
        .create(None)
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    Ok((StatusCode::CREATED, Json(to_response(&info))))
}

#[derive(Debug, Deserialize)]
pub struct RemovePlaygroundQuery {
    pub path: String,
}

/// DELETE /api/v1/playgrounds?path=… — unregister a playground. When (and
/// only when) the directory is a daemon-owned auto-created temp dir it is
/// deleted recursively together with everything inside it (sessions, memory,
/// checkpoints, tasks — the playground is fully self-contained). 204 on
/// success, 404 when the path was not registered.
pub async fn remove_playground(
    State(state): State<Arc<DaemonState>>,
    Query(q): Query<RemovePlaygroundQuery>,
) -> Result<StatusCode, (StatusCode, String)> {
    let owned = state
        .playgrounds
        .remove(q.path.trim())
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let Some(dir) = owned else {
        return Err((
            StatusCode::NOT_FOUND,
            "playground not registered".to_string(),
        ));
    };
    if let Err(e) = std::fs::remove_dir_all(&dir) {
        // Unregistered but undeletable (locked file, permissions): the state
        // is still consistent — the OS will clean its temp dir eventually.
        tracing::warn!(error = %e, dir = %dir.display(), "playground dir not deleted");
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, PlaygroundRegistry) {
        let store = tempfile::tempdir().unwrap();
        let base = tempfile::tempdir().unwrap();
        let reg = PlaygroundRegistry::load(store.path().join("playgrounds.json"));
        (base, reg)
    }

    #[test]
    fn create_registers_owned_dir() {
        let (_base, reg) = setup();
        // Create directly under the OS temp dir (production behavior) so the
        // ownership check applies. Compare against the canonicalized temp
        // root: macOS symlinks /var -> /private/var.
        let info = reg.create(None).unwrap();
        let temp = std::env::temp_dir().canonicalize().unwrap();
        assert!(info.path.starts_with(&temp));
        assert!(info.name.starts_with(PLAYGROUND_PREFIX));
        assert!(info.path.is_dir());
        assert_eq!(reg.list().len(), 1);
        // Owned: directly under the base with the prefix.
        assert_eq!(owned_temp_dir(&info.path), Some(info.path.clone()));
        std::fs::remove_dir_all(&info.path).ok();
    }

    #[test]
    fn persists_across_reload() {
        let (base, store_dir, reg) = {
            let store = tempfile::tempdir().unwrap();
            let base = tempfile::tempdir().unwrap();
            let reg = PlaygroundRegistry::load(store.path().join("playgrounds.json"));
            (base, store, reg)
        };
        let persist = store_dir.path().join("playgrounds.json");
        let info = reg.create(Some(base.path())).unwrap();
        drop(reg);

        let reg2 = PlaygroundRegistry::load(persist);
        assert_eq!(reg2.list().len(), 1);
        assert_eq!(reg2.list()[0].path, info.path);
        assert_eq!(reg2.resolve(info.path.to_str().unwrap()), Some(info.path));
    }

    #[test]
    fn stale_entries_pruned_on_load() {
        let store = tempfile::tempdir().unwrap();
        let base = tempfile::tempdir().unwrap();
        let reg = PlaygroundRegistry::load(store.path().join("playgrounds.json"));
        reg.create(Some(base.path())).unwrap();
        drop(reg);
        // Base (and thus every playground under it) vanishes before reload.
        drop(base);

        let reg2 = PlaygroundRegistry::load(store.path().join("playgrounds.json"));
        assert!(reg2.list().is_empty());
    }

    #[test]
    fn resolve_rejects_unknown_dirs() {
        let (base, _store, reg) = {
            let store = tempfile::tempdir().unwrap();
            let base = tempfile::tempdir().unwrap();
            let reg = PlaygroundRegistry::load(store.path().join("playgrounds.json"));
            (base, store, reg)
        };
        let info = reg.create(Some(base.path())).unwrap();
        let stranger = tempfile::tempdir().unwrap();
        assert_eq!(
            reg.resolve(stranger.path().to_str().unwrap()),
            None,
            "unregistered dir must not resolve"
        );
        assert_eq!(reg.resolve(info.path.to_str().unwrap()), Some(info.path));
    }

    #[test]
    fn remove_unregisters_and_reports_owned_dir() {
        let (_base, reg) = setup();
        // Under the OS temp dir (production behavior) the dir is owned and
        // reported for deletion.
        let info = reg.create(None).unwrap();
        let owned = reg.remove(info.path.to_str().unwrap()).unwrap();
        assert_eq!(owned, Some(info.path.clone()));
        assert!(reg.list().is_empty());
        // Second remove: not registered anymore.
        assert_eq!(reg.remove(info.path.to_str().unwrap()).unwrap(), None);
    }

    #[test]
    fn remove_of_unowned_dir_leaves_it_alone() {
        let (_base, reg) = setup();
        // Simulate a hand-registered dir outside any temp base (the registry
        // only ever creates owned dirs; craft one via direct state access).
        let stranger = tempfile::tempdir().unwrap();
        {
            let mut inner = reg.inner.write().unwrap();
            inner.playgrounds.push(PlaygroundInfo {
                path: stranger.path().canonicalize().unwrap(),
                name: "stranger".into(),
                created_at: Utc::now(),
            });
            persist_locked(&inner).unwrap();
        }
        let owned = reg.remove(stranger.path().to_str().unwrap()).unwrap();
        assert_eq!(owned, None, "unowned dir must not be reported for deletion");
        assert!(stranger.path().exists());
    }

    #[test]
    fn owned_temp_dir_requires_prefix() {
        // Directly under the OS temp dir (canonicalized) but wrong prefix.
        let wrong = std::env::temp_dir().join("not-a-wgenty-playground");
        std::fs::create_dir_all(&wrong).unwrap();
        let canon = wrong.canonicalize().unwrap();
        assert_eq!(owned_temp_dir(&canon), None);
        std::fs::remove_dir_all(&wrong).ok();
        // Correct prefix directly under temp.
        let right = std::env::temp_dir().join(format!("{PLAYGROUND_PREFIX}x"));
        std::fs::create_dir_all(&right).unwrap();
        let canon = right.canonicalize().unwrap();
        assert_eq!(owned_temp_dir(&canon), Some(canon));
        std::fs::remove_dir_all(&right).ok();
    }
}
