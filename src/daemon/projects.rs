//! Multi-project registry (web v1).
//!
//! The daemon's `working_dir` is always the implicit main project; additional
//! projects are arbitrary directories (a git repo is NOT required — non-git
//! projects simply have no worktree/task features) registered here and
//! persisted to `~/.wgenty-code/projects.json`. Every project-parameterized
//! endpoint validates its target through [`ProjectRegistry::resolve`] so a
//! client can never point the daemon at an arbitrary directory.

use axum::{
    extract::{Query, State},
    http::StatusCode,
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectInfo {
    /// Canonicalized absolute project root.
    pub path: PathBuf,
    /// Display name (directory basename).
    pub name: String,
    pub added_at: DateTime<Utc>,
}

/// Response DTO for `GET /api/v1/projects`.
#[derive(Debug, Clone, Serialize)]
pub struct ProjectResponse {
    pub path: String,
    pub name: String,
    pub is_main: bool,
    /// Whether the root looks like a git checkout (`.git` entry present).
    /// Non-git projects hide worktree/task features in the UI.
    pub is_git_repo: bool,
    pub added_at: DateTime<Utc>,
}

#[derive(Debug)]
struct Inner {
    /// Canonicalized daemon working_dir — the implicit main project.
    main_root: PathBuf,
    main_added_at: DateTime<Utc>,
    projects: Vec<ProjectInfo>,
    persist_path: PathBuf,
}

/// Cheaply cloneable registry handle; clones share the same state.
#[derive(Debug, Clone)]
pub struct ProjectRegistry {
    inner: Arc<std::sync::RwLock<Inner>>,
}

impl ProjectRegistry {
    /// Registry persisted at the default location (`<config_dir>/projects.json`).
    pub fn load_default(main_root: PathBuf) -> Self {
        Self::load(main_root, crate::utils::config_dir().join("projects.json"))
    }

    /// Load (or initialize) a registry persisted at `persist_path`.
    pub fn load(main_root: PathBuf, persist_path: PathBuf) -> Self {
        let main_root = main_root.canonicalize().unwrap_or(main_root);
        let projects = match std::fs::read_to_string(&persist_path) {
            Ok(content) => match serde_json::from_str::<Vec<ProjectInfo>>(&content) {
                Ok(mut list) => {
                    // Drop entries whose directory vanished or that duplicate
                    // the main root — they are unusable / meaningless.
                    let before = list.len();
                    list.retain(|p| p.path != main_root && p.path.is_dir());
                    if list.len() != before {
                        tracing::info!(
                            dropped = before - list.len(),
                            "pruned stale project registry entries"
                        );
                    }
                    list
                }
                Err(e) => {
                    tracing::warn!(error = %e, path = %persist_path.display(), "corrupt projects.json; starting empty");
                    Vec::new()
                }
            },
            Err(_) => Vec::new(), // first run — no file yet
        };
        Self {
            inner: Arc::new(std::sync::RwLock::new(Inner {
                main_added_at: Utc::now(),
                main_root,
                projects,
                persist_path,
            })),
        }
    }

    /// The main project root (daemon working_dir, canonicalized).
    pub fn main_root(&self) -> PathBuf {
        self.inner
            .read()
            .expect("project registry lock poisoned")
            .main_root
            .clone()
    }

    /// Main project first, then registered projects in insertion order.
    pub fn list(&self) -> Vec<ProjectInfo> {
        let inner = self.inner.read().expect("project registry lock poisoned");
        let mut out = Vec::with_capacity(inner.projects.len() + 1);
        out.push(ProjectInfo {
            name: dir_name(&inner.main_root),
            path: inner.main_root.clone(),
            added_at: inner.main_added_at,
        });
        out.extend(inner.projects.iter().cloned());
        out
    }

    /// Register a new project directory. Errors are client-facing strings.
    pub fn add(&self, path: &str) -> Result<ProjectInfo, String> {
        let canon = canonicalize_dir(path)?;
        let mut inner = self.inner.write().expect("project registry lock poisoned");
        if canon == inner.main_root {
            return Err("path is the main project".to_string());
        }
        if inner.projects.iter().any(|p| p.path == canon) {
            return Err(format!("project already registered: {}", canon.display()));
        }
        let info = ProjectInfo {
            name: dir_name(&canon),
            path: canon,
            added_at: Utc::now(),
        };
        inner.projects.push(info.clone());
        persist_locked(&inner)?;
        Ok(info)
    }

    /// Remove a project from the registry. The directory itself and its
    /// `.wgenty-code/` data (sessions, memory, checkpoints) are untouched.
    /// Removing a project with an in-flight run is safe: runs capture their
    /// session manager at start. Returns false when the path was not registered.
    pub fn remove(&self, path: &str) -> Result<bool, String> {
        let canon = canonicalize_dir_lossy(path);
        let mut inner = self.inner.write().expect("project registry lock poisoned");
        let before = inner.projects.len();
        inner.projects.retain(|p| p.path != canon);
        if inner.projects.len() == before {
            return Ok(false);
        }
        persist_locked(&inner)?;
        Ok(true)
    }

    /// Whitelist check: canonicalize `path` and return it when it is the main
    /// project or a registered one. `None` = not a known project (reject).
    pub fn resolve(&self, path: &str) -> Option<PathBuf> {
        let canon = canonicalize_dir_lossy(path);
        let inner = self.inner.read().expect("project registry lock poisoned");
        if canon == inner.main_root || inner.projects.iter().any(|p| p.path == canon) {
            Some(canon)
        } else {
            None
        }
    }

    /// Registered (non-main) project roots — used to fan out per-project
    /// session/memory/checkpoint lookups.
    pub fn registered_roots(&self) -> Vec<PathBuf> {
        self.inner
            .read()
            .expect("project registry lock poisoned")
            .projects
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

fn canonicalize_dir(path: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(path);
    let canon = p
        .canonicalize()
        .map_err(|_| format!("directory does not exist: {path}"))?;
    if !canon.is_dir() {
        return Err(format!("not a directory: {path}"));
    }
    Ok(canon)
}

/// Like [`canonicalize_dir`] but tolerant of vanished directories: falls back
/// to the lexically-absolute path so `remove`/`resolve` still match entries
/// whose directory was deleted after registration.
fn canonicalize_dir_lossy(path: &str) -> PathBuf {
    let p = PathBuf::from(path);
    p.canonicalize().unwrap_or(p)
}

/// Atomic persist: write tmp + rename so a crash mid-write can't corrupt the
/// registry. Called with the write lock held.
fn persist_locked(inner: &Inner) -> Result<(), String> {
    let tmp = inner.persist_path.with_extension("json.tmp");
    let content = serde_json::to_string_pretty(&inner.projects)
        .map_err(|e| format!("serialize projects: {e}"))?;
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

fn to_response(info: &ProjectInfo, is_main: bool) -> ProjectResponse {
    ProjectResponse {
        path: info.path.to_string_lossy().to_string(),
        name: info.name.clone(),
        is_main,
        // `.git` may be a directory (normal checkout) or a file (worktree) —
        // either way `exists()` covers both.
        is_git_repo: info.path.join(".git").exists(),
        added_at: info.added_at,
    }
}

/// GET /api/v1/projects — main project first, then registered projects.
pub async fn list_projects(State(state): State<Arc<DaemonState>>) -> Json<Vec<ProjectResponse>> {
    let main = state.projects.main_root();
    let out = state
        .projects
        .list()
        .iter()
        .map(|p| to_response(p, p.path == main))
        .collect();
    Json(out)
}

#[derive(Debug, Deserialize)]
pub struct AddProjectRequest {
    pub path: String,
}

/// POST /api/v1/projects — register a project directory. 201 on success,
/// 400 for a bad/duplicate path.
pub async fn add_project(
    State(state): State<Arc<DaemonState>>,
    Json(body): Json<AddProjectRequest>,
) -> Result<(StatusCode, Json<ProjectResponse>), (StatusCode, String)> {
    if body.path.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "path is required".to_string()));
    }
    let info = state
        .projects
        .add(body.path.trim())
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    Ok((StatusCode::CREATED, Json(to_response(&info, false))))
}

#[derive(Debug, Deserialize)]
pub struct RemoveProjectQuery {
    pub path: String,
}

/// DELETE /api/v1/projects?path=… — unregister a project. The directory and
/// its `.wgenty-code/` data stay on disk. 204 on success, 404 when the path
/// was not registered, 400 for the main project.
pub async fn remove_project(
    State(state): State<Arc<DaemonState>>,
    Query(q): Query<RemoveProjectQuery>,
) -> Result<StatusCode, (StatusCode, String)> {
    if state.projects.resolve(&q.path) == Some(state.projects.main_root()) {
        return Err((
            StatusCode::BAD_REQUEST,
            "cannot remove the main project".to_string(),
        ));
    }
    match state.projects.remove(&q.path) {
        Ok(true) => Ok(StatusCode::NO_CONTENT),
        Ok(false) => Err((
            StatusCode::NOT_FOUND,
            format!("project not registered: {}", q.path),
        )),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, tempfile::TempDir, ProjectRegistry) {
        let main = tempfile::tempdir().unwrap();
        let store = tempfile::tempdir().unwrap();
        let reg = ProjectRegistry::load(
            main.path().to_path_buf(),
            store.path().join("projects.json"),
        );
        (main, store, reg)
    }

    #[test]
    fn list_starts_with_main() {
        let (main, _store, reg) = setup();
        let list = reg.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].path, main.path().canonicalize().unwrap());
    }

    #[test]
    fn add_dedup_remove_roundtrip() {
        let (_main, _store, reg) = setup();
        let proj = tempfile::tempdir().unwrap();
        let added = reg.add(proj.path().to_str().unwrap()).unwrap();
        assert_eq!(added.path, proj.path().canonicalize().unwrap());
        assert_eq!(reg.list().len(), 2);

        // Duplicate add rejected.
        assert!(reg.add(proj.path().to_str().unwrap()).is_err());

        // Resolve whitelist.
        assert_eq!(
            reg.resolve(proj.path().to_str().unwrap()),
            Some(proj.path().canonicalize().unwrap())
        );
        assert!(reg.resolve("/definitely/not/registered").is_none());

        // Remove.
        assert!(reg.remove(proj.path().to_str().unwrap()).unwrap());
        assert!(!reg.remove(proj.path().to_str().unwrap()).unwrap());
        assert_eq!(reg.list().len(), 1);
    }

    #[test]
    fn add_rejects_missing_and_main() {
        let (main, _store, reg) = setup();
        assert!(reg.add("/no/such/dir").is_err());
        assert!(reg.add(main.path().to_str().unwrap()).is_err());
    }

    #[test]
    fn persists_across_reload() {
        let main = tempfile::tempdir().unwrap();
        let store = tempfile::tempdir().unwrap();
        let persist = store.path().join("projects.json");
        let proj = tempfile::tempdir().unwrap();

        let reg = ProjectRegistry::load(main.path().to_path_buf(), persist.clone());
        reg.add(proj.path().to_str().unwrap()).unwrap();
        drop(reg);

        let reg2 = ProjectRegistry::load(main.path().to_path_buf(), persist);
        assert_eq!(reg2.registered_roots().len(), 1);
        assert_eq!(
            reg2.registered_roots()[0],
            proj.path().canonicalize().unwrap()
        );
    }

    #[test]
    fn stale_entries_pruned_on_load() {
        let main = tempfile::tempdir().unwrap();
        let store = tempfile::tempdir().unwrap();
        let persist = store.path().join("projects.json");
        let proj = tempfile::tempdir().unwrap();

        let reg = ProjectRegistry::load(main.path().to_path_buf(), persist.clone());
        reg.add(proj.path().to_str().unwrap()).unwrap();
        drop(reg);
        // Directory vanishes before the next daemon start.
        drop(proj);

        let reg2 = ProjectRegistry::load(main.path().to_path_buf(), persist);
        assert!(reg2.registered_roots().is_empty());
    }
}
