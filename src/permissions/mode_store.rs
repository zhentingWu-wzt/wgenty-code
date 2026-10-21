//! Per-project permission mode store.
//!
//! Replaces the former process-global `root_mode`/`effective_mode`. Each
//! project (identified by its canonical working directory) owns an independent
//! `RootPermissionMode`/`EffectiveMode` pair, so multiple projects sharing one
//! daemon no longer bleed permission decisions into each other.
//!
//! Lookups default to `Normal` when a project has no explicit entry, preserving
//! the previous default behaviour for unregistered sessions.
//!
//! Persistence (v1): entries live in a single user-owned JSON file
//! (`~/.wgenty-code/permission-modes.json`), keyed by canonical project path.
//! User-owned rather than project-owned on purpose: a repo-committed
//! `.wgenty-code/permission-mode.json` preset to `yolo` would silently arm
//! full-auto approval the moment a cloned repo is opened — the trust boundary
//! must be the user's config dir, not repo contents. `Plan` is a transient
//! workflow state (TUI starts every session from Normal), so it is normalized
//! away at persistence time instead of being restored after a restart.

use crate::config::RootPermissionMode;
use crate::sandbox::EffectiveMode;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

/// Current on-disk schema version.
pub const PERSIST_VERSION: u32 = 1;

/// File name inside the user config dir (`~/.wgenty-code/`).
pub const PERSIST_FILE: &str = "permission-modes.json";

/// A project's permission mode pair: the user-facing `RootPermissionMode`
/// (Normal/AcceptEdits/Yolo) and the derived sandbox `EffectiveMode`.
#[derive(Clone, Debug)]
pub struct PermissionModeEntry {
    pub root_mode: RootPermissionMode,
    pub effective_mode: EffectiveMode,
}

impl Default for PermissionModeEntry {
    fn default() -> Self {
        Self {
            root_mode: RootPermissionMode::Normal,
            effective_mode: EffectiveMode::Normal,
        }
    }
}

impl PermissionModeEntry {
    /// Persistence form: `Plan` is transient and must not survive a restart,
    /// so it collapses to the effective mode derived from `root_mode`.
    fn persisted(&self) -> PermissionModeEntry {
        if self.effective_mode == EffectiveMode::Plan {
            Self {
                root_mode: self.root_mode,
                effective_mode: EffectiveMode::from_root_permission_mode(self.root_mode),
            }
        } else {
            self.clone()
        }
    }
}

/// On-disk record for one project. Serde derives on both enums are
/// `rename_all = "snake_case"`, so values are `"normal" | "accept_edits" |
/// "yolo"` (and `"plan"`, which never round-trips out of a load).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedEntry {
    root_mode: RootPermissionMode,
    effective_mode: EffectiveMode,
}

/// Envelope written to `permission-modes.json`.
#[derive(Debug, Serialize, Deserialize)]
struct PersistedFile {
    version: u32,
    /// Canonical project path -> mode pair.
    modes: HashMap<String, PersistedEntry>,
}

/// Concurrent map of canonical working directory -> permission mode entry.
///
/// Cloning shares the underlying storage (Arc), so the daemon, TaskTool, and
/// run_loop all observe the same per-project modes.
#[derive(Clone)]
pub struct PermissionModeStore {
    modes: Arc<RwLock<HashMap<PathBuf, PermissionModeEntry>>>,
    /// Where the JSON snapshot is persisted. Injected so tests can point at a
    /// tempdir; production callers pass `Some(config_dir().join(PERSIST_FILE))`.
    persist_path: Option<PathBuf>,
}

impl PermissionModeStore {
    /// In-memory-only store (no persistence). Kept for existing callers that
    /// run without a daemon config dir (TaskTool tests, run_loop tests).
    pub fn new() -> Self {
        Self {
            modes: Arc::new(RwLock::new(HashMap::new())),
            persist_path: None,
        }
    }

    /// Persistent store rooted at `persist_path`. Loading is lazy-by-value:
    /// [`Self::load_persisted`] must be called (daemon startup) before the
    /// file's contents are visible through `get`.
    pub fn with_persist_path(persist_path: PathBuf) -> Self {
        Self {
            modes: Arc::new(RwLock::new(HashMap::new())),
            persist_path: Some(persist_path),
        }
    }

    /// Returns the entry for `workdir`, defaulting to `Normal` when the project
    /// has no explicit entry. Callers pass a canonical path (as produced by
    /// `DaemonState::effective_session_root`).
    pub fn get(&self, workdir: &Path) -> PermissionModeEntry {
        self.modes
            .read()
            .expect("permission_modes lock poisoned")
            .get(workdir)
            .cloned()
            .unwrap_or_default()
    }

    /// Set or replace the entry for `workdir` (memory only — persistence is
    /// driven by the daemon handler via [`Self::persist`]).
    pub fn set(
        &self,
        workdir: PathBuf,
        root_mode: RootPermissionMode,
        effective_mode: EffectiveMode,
    ) {
        self.modes
            .write()
            .expect("permission_modes lock poisoned")
            .insert(
                workdir,
                PermissionModeEntry {
                    root_mode,
                    effective_mode,
                },
            );
    }

    /// Atomically write the current entries to the persist path (tmp + rename,
    /// mirroring `ProjectRegistry::persist_locked`). Best-effort: errors are
    /// returned for the caller to log — in-memory state stays authoritative.
    pub fn persist(&self) -> std::io::Result<()> {
        let Some(path) = &self.persist_path else {
            return Ok(());
        };
        let file = self.snapshot_for_persist();
        let content = serde_json::to_string_pretty(&file)
            .map_err(|e| std::io::Error::other(format!("serialize permission modes: {e}")))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, content)?;
        std::fs::rename(&tmp, path)
    }

    /// Load entries from the persist path into memory, replacing anything set
    /// since boot. Missing file = fresh install (no-op). Corrupt or
    /// wrong-version files are logged and skipped — a hand-edited file must
    /// never block the daemon from starting.
    pub fn load_persisted(&self) {
        let Some(path) = &self.persist_path else {
            return;
        };
        let loaded = Self::read_file(path);
        let mut modes = self.modes.write().expect("permission_modes lock poisoned");
        match loaded {
            Ok(entries) => {
                modes.clear();
                modes.extend(entries);
            }
            Err(PersistError::Missing) => {}
            Err(PersistError::Corrupt(e)) => {
                tracing::warn!(error = %e, path = %path.display(), "corrupt permission-modes.json; starting fresh");
            }
        }
    }

    fn read_file(path: &Path) -> Result<HashMap<PathBuf, PermissionModeEntry>, PersistError> {
        let content = std::fs::read_to_string(path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => PersistError::Missing,
            _ => PersistError::Corrupt(e),
        })?;
        let file: PersistedFile = serde_json::from_str(&content)
            .map_err(|e| PersistError::Corrupt(std::io::Error::other(format!("parse: {e}"))))?;
        if file.version != PERSIST_VERSION {
            return Err(PersistError::Corrupt(std::io::Error::other(format!(
                "unsupported version: {} (expected {PERSIST_VERSION})",
                file.version
            ))));
        }
        let mut out = HashMap::with_capacity(file.modes.len());
        for (path_str, entry) in file.modes {
            // `Plan` never legitimately persists; a record carrying it was
            // hand-edited. Normalize to the derived mode (mirrors persist)
            // rather than restoring a mode the user never confirmed post-boot.
            let effective_mode = if entry.effective_mode == EffectiveMode::Plan {
                EffectiveMode::from_root_permission_mode(entry.root_mode)
            } else {
                entry.effective_mode
            };
            out.insert(
                PathBuf::from(path_str),
                PermissionModeEntry {
                    root_mode: entry.root_mode,
                    effective_mode,
                },
            );
        }
        Ok(out)
    }

    fn snapshot_for_persist(&self) -> PersistedFile {
        let modes = self.modes.read().expect("permission_modes lock poisoned");
        let modes = modes
            .iter()
            .map(|(path, entry)| {
                let persisted = entry.persisted();
                (
                    path.to_string_lossy().to_string(),
                    PersistedEntry {
                        root_mode: persisted.root_mode,
                        effective_mode: persisted.effective_mode,
                    },
                )
            })
            .collect();
        PersistedFile {
            version: PERSIST_VERSION,
            modes,
        }
    }
}

/// Distinguishes "no file yet" (silent no-op) from a real read/parse failure
/// (warn + start fresh) in [`PermissionModeStore::load_persisted`].
#[derive(Debug)]
enum PersistError {
    Missing,
    Corrupt(std::io::Error),
}

impl Default for PermissionModeStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unregistered_workdir_defaults_to_normal() {
        let store = PermissionModeStore::new();
        let entry = store.get(Path::new("/nonexistent/project"));
        assert_eq!(entry.root_mode, RootPermissionMode::Normal);
        assert_eq!(entry.effective_mode, EffectiveMode::Normal);
    }

    #[test]
    fn set_then_get_returns_entry() {
        let store = PermissionModeStore::new();
        let workdir = PathBuf::from("/projects/alpha");
        store.set(
            workdir.clone(),
            RootPermissionMode::Yolo,
            EffectiveMode::Yolo,
        );

        let entry = store.get(&workdir);
        assert_eq!(entry.root_mode, RootPermissionMode::Yolo);
        assert_eq!(entry.effective_mode, EffectiveMode::Yolo);
    }

    #[test]
    fn distinct_projects_are_isolated() {
        let store = PermissionModeStore::new();
        let alpha = PathBuf::from("/projects/alpha");
        let beta = PathBuf::from("/projects/beta");

        store.set(alpha.clone(), RootPermissionMode::Yolo, EffectiveMode::Yolo);
        assert_eq!(store.get(&beta).root_mode, RootPermissionMode::Normal);
        assert_eq!(store.get(&alpha).root_mode, RootPermissionMode::Yolo);
    }

    #[test]
    fn clone_shares_storage() {
        let store = PermissionModeStore::new();
        let clone = store.clone();
        let workdir = PathBuf::from("/projects/shared");

        store.set(
            workdir.clone(),
            RootPermissionMode::AcceptEdits,
            EffectiveMode::AcceptEdits,
        );
        assert_eq!(
            clone.get(&workdir).root_mode,
            RootPermissionMode::AcceptEdits
        );
    }

    // ── Persistence ───────────────────────────────────────────────────────

    fn temp_persist_path() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("permission-modes.json");
        (dir, path)
    }

    #[test]
    fn persist_then_load_round_trips_entries() {
        let (_guard, path) = temp_persist_path();
        let alpha = PathBuf::from("/projects/alpha");
        let beta = PathBuf::from("/projects/beta");

        let store = PermissionModeStore::with_persist_path(path.clone());
        store.set(alpha.clone(), RootPermissionMode::Yolo, EffectiveMode::Yolo);
        store.set(
            beta.clone(),
            RootPermissionMode::AcceptEdits,
            EffectiveMode::AcceptEdits,
        );
        store.persist().expect("persist");

        let restored = PermissionModeStore::with_persist_path(path);
        restored.load_persisted();
        assert_eq!(restored.get(&alpha).root_mode, RootPermissionMode::Yolo);
        assert_eq!(
            restored.get(&beta).root_mode,
            RootPermissionMode::AcceptEdits
        );
        // Unregistered paths still default to Normal.
        assert_eq!(
            restored.get(Path::new("/projects/gamma")).root_mode,
            RootPermissionMode::Normal
        );
    }

    #[test]
    fn persist_normalizes_plan_to_derived_mode() {
        let (_guard, path) = temp_persist_path();
        let workdir = PathBuf::from("/projects/plan-mode");

        let store = PermissionModeStore::with_persist_path(path.clone());
        // TUI Plan: root Normal + effective Plan (cannot be derived).
        store.set(
            workdir.clone(),
            RootPermissionMode::Normal,
            EffectiveMode::Plan,
        );
        store.persist().expect("persist");

        let on_disk: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("file written"))
                .expect("valid json");
        let key = workdir.to_string_lossy().to_string();
        let entry = &on_disk["modes"][key.as_str()];
        assert_eq!(entry["root_mode"], "normal");
        // Plan must NOT round-trip: normalized to the derived effective mode.
        assert_eq!(entry["effective_mode"], "normal");
    }

    #[test]
    fn load_missing_file_is_noop() {
        let (_guard, path) = temp_persist_path();
        let store = PermissionModeStore::with_persist_path(path);
        store.load_persisted();
        assert_eq!(
            store.get(Path::new("/any")).root_mode,
            RootPermissionMode::Normal
        );
    }

    #[test]
    fn load_corrupt_or_wrong_version_file_starts_fresh() {
        let (_guard, path) = temp_persist_path();

        // Garbage content.
        std::fs::write(&path, "{not json").unwrap();
        let store = PermissionModeStore::with_persist_path(path.clone());
        store.load_persisted();
        assert_eq!(
            store.get(Path::new("/any")).root_mode,
            RootPermissionMode::Normal
        );

        // Right shape, future version.
        std::fs::write(
            &path,
            serde_json::json!({"version": 99, "modes": {}}).to_string(),
        )
        .unwrap();
        let store = PermissionModeStore::with_persist_path(path);
        store.load_persisted();
        assert_eq!(
            store.get(Path::new("/any")).root_mode,
            RootPermissionMode::Normal
        );
    }

    #[test]
    fn load_normalizes_forged_plan_records() {
        let (_guard, path) = temp_persist_path();
        let workdir = "/projects/forged";
        std::fs::write(
            &path,
            serde_json::json!({
                "version": 1,
                "modes": { workdir: {"root_mode": "yolo", "effective_mode": "plan"} }
            })
            .to_string(),
        )
        .unwrap();

        let store = PermissionModeStore::with_persist_path(path);
        store.load_persisted();
        // A hand-edited Plan record is normalized to the derived mode
        // (yolo -> yolo), never restored as Plan.
        assert_eq!(
            store.get(Path::new(workdir)).root_mode,
            RootPermissionMode::Yolo
        );
        assert_eq!(
            store.get(Path::new(workdir)).effective_mode,
            EffectiveMode::Yolo
        );
    }

    #[test]
    fn new_store_persist_is_noop_without_path() {
        let store = PermissionModeStore::new();
        store.set(
            PathBuf::from("/x"),
            RootPermissionMode::Yolo,
            EffectiveMode::Yolo,
        );
        store.persist().expect("no-op without persist_path");
    }
}
