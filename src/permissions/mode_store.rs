//! Per-project permission mode store.
//!
//! Replaces the former process-global `root_mode`/`effective_mode`. Each
//! project (identified by its canonical working directory) owns an independent
//! `RootPermissionMode`/`EffectiveMode` pair, so multiple projects sharing one
//! daemon no longer bleed permission decisions into each other.
//!
//! Lookups default to `Normal` when a project has no explicit entry, preserving
//! the previous default behaviour for unregistered sessions.

use crate::config::RootPermissionMode;
use crate::sandbox::EffectiveMode;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

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

/// Concurrent map of canonical working directory -> permission mode entry.
///
/// Cloning shares the underlying storage (Arc), so the daemon, TaskTool, and
/// run_loop all observe the same per-project modes.
#[derive(Clone)]
pub struct PermissionModeStore {
    modes: Arc<RwLock<HashMap<PathBuf, PermissionModeEntry>>>,
}

impl PermissionModeStore {
    pub fn new() -> Self {
        Self {
            modes: Arc::new(RwLock::new(HashMap::new())),
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

    /// Set or replace the entry for `workdir`.
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
}
