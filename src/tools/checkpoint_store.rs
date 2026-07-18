//! Per-turn file snapshot store for non-destructive checkpoint/rewind.
//!
//! Each agent turn creates a snapshot under `.wgenty-code/checkpoints/<turn-id>/`
//! with a `manifest.json` and `blobs/` subdir. Before a file-editing tool
//! modifies a file, its pre-edit content is captured into a blob and recorded
//! in the manifest. `rewind` restores files from the manifest WITHOUT touching
//! any file not tracked by that manifest, so unrelated untracked files survive.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const MAX_SNAPSHOT_BYTES: u64 = 5 * 1024 * 1024;

/// Default maximum number of per-turn snapshots to keep on disk.
pub const DEFAULT_KEEP_N: usize = 10;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FileState {
    Saved,
    Tombstone,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub path: String,
    pub blob: Option<String>,
    pub state: FileState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub turn_id: String,
    pub created_at: String,
    pub files: Vec<ManifestEntry>,
}

impl Manifest {
    fn new(turn_id: &str) -> Self {
        Self {
            turn_id: turn_id.to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            files: Vec::new(),
        }
    }

    fn has(&self, path: &str) -> bool {
        self.files.iter().any(|e| e.path == path)
    }
}

#[derive(Debug, Clone)]
pub struct TurnInfo {
    pub turn_id: String,
    pub created_at: String,
    pub file_count: usize,
}

pub struct CheckpointStore {
    root: PathBuf,
    project_root: PathBuf,
    keep_n: usize,
}

impl CheckpointStore {
    pub fn new(project_root: impl Into<PathBuf>) -> Self {
        Self::with_keep_n(project_root, DEFAULT_KEEP_N)
    }

    pub fn with_keep_n(project_root: impl Into<PathBuf>, keep_n: usize) -> Self {
        let project_root = project_root.into();
        let root = project_root.join(".wgenty-code").join("checkpoints");
        Self {
            root,
            project_root,
            keep_n: keep_n.max(1),
        }
    }

    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    pub fn keep_n(&self) -> usize {
        self.keep_n
    }

    fn turn_dir(&self, turn_id: &str) -> PathBuf {
        self.root.join(turn_id)
    }

    fn manifest_path(&self, turn_id: &str) -> PathBuf {
        self.turn_dir(turn_id).join("manifest.json")
    }

    fn blobs_dir(&self, turn_id: &str) -> PathBuf {
        self.turn_dir(turn_id).join("blobs")
    }

    fn read_manifest(&self, turn_id: &str) -> Result<Manifest> {
        let path = self.manifest_path(turn_id);
        let data = std::fs::read_to_string(&path)
            .with_context(|| format!("read manifest: {}", path.display()))?;
        let manifest: Manifest = serde_json::from_str(&data)
            .with_context(|| format!("parse manifest: {}", path.display()))?;
        Ok(manifest)
    }

    fn write_manifest(&self, manifest: &Manifest) -> Result<()> {
        let path = self.manifest_path(&manifest.turn_id);
        let data = serde_json::to_string_pretty(manifest).context("serialize manifest")?;
        std::fs::write(&path, data)
            .with_context(|| format!("write manifest: {}", path.display()))?;
        Ok(())
    }

    /// Ensure the turn snapshot dir + manifest exist. Returns `true` when the
    /// manifest was newly created (i.e. this is the first capture of the turn).
    fn ensure_turn(&self, turn_id: &str) -> Result<bool> {
        std::fs::create_dir_all(self.blobs_dir(turn_id))
            .with_context(|| format!("create turn dir: {}", turn_id))?;
        if !self.manifest_path(turn_id).exists() {
            self.write_manifest(&Manifest::new(turn_id))?;
            return Ok(true);
        }
        Ok(false)
    }

    /// Explicitly start a turn snapshot. Optional: [`try_capture_file`]
    /// lazily initializes the snapshot on first capture, so callers need not
    /// call this unless they want an empty snapshot recorded.
    ///
    /// When the turn directory is newly created, prunes older snapshots down
    /// to `keep_n`.
    pub fn begin_turn(&self, turn_id: &str) -> Result<()> {
        if self.ensure_turn(turn_id)? {
            if let Err(e) = self.prune(self.keep_n) {
                tracing::warn!(error = %e, "checkpoint prune failed");
            }
        }
        Ok(())
    }

    /// Capture the pre-edit content of `path` (relative to project root or
    /// absolute) into the turn snapshot. Idempotent per (turn, path). Lazily
    /// initializes the turn snapshot on first capture and prunes old turns.
    pub fn try_capture_file(&self, turn_id: &str, path: &str) -> Result<()> {
        if self.ensure_turn(turn_id)? {
            if let Err(e) = self.prune(self.keep_n) {
                tracing::warn!(error = %e, "checkpoint prune failed");
            }
        }
        let mut manifest = self.read_manifest(turn_id)?;
        let abs = normalize_abs(&self.project_root, path);
        let key = manifest_key(&self.project_root, &abs);
        if manifest.has(&key) {
            return Ok(());
        }
        let meta = match std::fs::metadata(&abs) {
            Ok(m) => m,
            Err(_) => {
                manifest.files.push(ManifestEntry {
                    path: key,
                    blob: None,
                    state: FileState::Tombstone,
                });
                self.write_manifest(&manifest)?;
                return Ok(());
            }
        };

        if meta.len() > MAX_SNAPSHOT_BYTES || is_binary_path(&abs) {
            manifest.files.push(ManifestEntry {
                path: key,
                blob: None,
                state: FileState::Skipped,
            });
            self.write_manifest(&manifest)?;
            return Ok(());
        }

        let content = std::fs::read(&abs)
            .with_context(|| format!("read file (capture): {}", abs.display()))?;
        let blob_name = format!("{:04}", manifest.files.len());
        let blob_path = self.blobs_dir(turn_id).join(&blob_name);
        if !blob_path.exists() {
            std::fs::write(&blob_path, &content)
                .with_context(|| format!("write blob: {}", blob_path.display()))?;
        }
        manifest.files.push(ManifestEntry {
            path: key,
            blob: Some(blob_name),
            state: FileState::Saved,
        });
        self.write_manifest(&manifest)?;
        Ok(())
    }

    pub fn capture_files(&self, turn_id: &str, paths: &[String]) -> Result<()> {
        for p in paths {
            if let Err(e) = self.try_capture_file(turn_id, p) {
                tracing::warn!(error = %e, file = %p, "checkpoint capture failed");
            }
        }
        Ok(())
    }

    pub fn rewind(&self, turn_id: &str) -> Result<String> {
        let manifest = self.read_manifest(turn_id)?;
        let mut restored = 0usize;
        let mut skipped = 0usize;
        let mut failed: Vec<String> = Vec::new();
        for entry in &manifest.files {
            let abs = resolve_manifest_path(&self.project_root, &entry.path);
            match entry.state {
                FileState::Saved => {
                    if let Some(blob_name) = &entry.blob {
                        let blob_path = self.blobs_dir(turn_id).join(blob_name);
                        match std::fs::read(&blob_path) {
                            Ok(content) => {
                                if let Some(parent) = abs.parent() {
                                    let _ = std::fs::create_dir_all(parent);
                                }
                                if std::fs::write(&abs, content).is_ok() {
                                    restored += 1;
                                } else {
                                    failed.push(entry.path.clone());
                                }
                            }
                            Err(_) => failed.push(entry.path.clone()),
                        }
                    } else {
                        failed.push(entry.path.clone());
                    }
                }
                FileState::Tombstone => {
                    if entry.blob.is_none() {
                        if abs.exists() {
                            let _ = std::fs::remove_file(&abs);
                        }
                        restored += 1;
                    } else if let Some(blob_name) = &entry.blob {
                        let blob_path = self.blobs_dir(turn_id).join(blob_name);
                        if let Ok(content) = std::fs::read(&blob_path) {
                            if let Some(parent) = abs.parent() {
                                let _ = std::fs::create_dir_all(parent);
                            }
                            let _ = std::fs::write(&abs, content);
                            restored += 1;
                        } else {
                            failed.push(entry.path.clone());
                        }
                    }
                }
                FileState::Skipped => {
                    skipped += 1;
                }
            }
        }
        Ok(format!(
            "Rewind {}: restored {}, skipped {}, failed {}: [{}]",
            turn_id,
            restored,
            skipped,
            failed.len(),
            failed.join(", ")
        ))
    }

    pub fn prune(&self, keep_n: usize) -> Result<usize> {
        if !self.root.exists() {
            return Ok(0);
        }
        let mut entries: Vec<(PathBuf, std::time::SystemTime)> = Vec::new();
        for entry in std::fs::read_dir(&self.root)
            .with_context(|| format!("read checkpoints dir: {}", self.root.display()))?
        {
            let entry = entry.context("iterate checkpoints dir")?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let mtime = entry
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            entries.push((path, mtime));
        }
        entries.sort_by_key(|b| std::cmp::Reverse(b.1));
        let mut deleted = 0;
        for (path, _) in entries.iter().skip(keep_n) {
            if std::fs::remove_dir_all(path).is_ok() {
                deleted += 1;
            }
        }
        Ok(deleted)
    }

    pub fn list(&self) -> Result<Vec<TurnInfo>> {
        if !self.root.exists() {
            return Ok(Vec::new());
        }
        let mut infos = Vec::new();
        for entry in std::fs::read_dir(&self.root)
            .with_context(|| format!("read checkpoints dir: {}", self.root.display()))?
        {
            let entry = entry.context("iterate checkpoints dir")?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let turn_id = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            if let Ok(manifest) = self.read_manifest(&turn_id) {
                infos.push(TurnInfo {
                    turn_id: manifest.turn_id,
                    created_at: manifest.created_at,
                    file_count: manifest.files.len(),
                });
            }
        }
        infos.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(infos)
    }
}

/// Resolve `path` (relative to project root or absolute) to an absolute path.
fn normalize_abs(project_root: &Path, path: &str) -> PathBuf {
    let p = Path::new(path);
    if p.is_absolute() {
        PathBuf::from(p)
    } else {
        project_root.join(p)
    }
}

/// Manifest key for `abs`: relative to project root when possible (portable),
/// else the absolute path string.
fn manifest_key(project_root: &Path, abs: &Path) -> String {
    match abs.strip_prefix(project_root) {
        Ok(rel) => rel.to_string_lossy().into_owned(),
        Err(_) => abs.to_string_lossy().into_owned(),
    }
}

/// Reconstruct the target path for a manifest entry (absolute key as-is,
/// relative key joined to project root).
fn resolve_manifest_path(project_root: &Path, key: &str) -> PathBuf {
    let p = Path::new(key);
    if p.is_absolute() {
        PathBuf::from(p)
    } else {
        project_root.join(p)
    }
}

impl crate::agent::CheckpointCapture for CheckpointStore {
    fn capture_file(&self, turn_id: &str, abs_path: &std::path::Path) {
        let path_str = abs_path.to_string_lossy();
        if let Err(e) = self.try_capture_file(turn_id, &path_str) {
            tracing::warn!(
                error = %e,
                file = %abs_path.display(),
                turn = %turn_id,
                "checkpoint capture failed"
            );
        }
    }
}

fn is_binary_path(path: &Path) -> bool {
    const BINARY_EXTS: &[&str] = &[
        "png", "jpg", "jpeg", "gif", "webp", "ico", "bmp", "tiff", "pdf", "zip", "gz", "tar",
        "bz2", "7z", "rar", "class", "jar", "so", "dylib", "dll", "exe", "wasm", "o", "a", "mp3",
        "mp4", "mov", "avi", "mkv", "flac", "wav", "ogg", "sqlite", "db", "lock",
    ];
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => BINARY_EXTS.contains(&ext.to_lowercase().as_str()),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_project() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    fn write_file(root: &Path, rel: &str, content: &str) {
        let abs = root.join(rel);
        if let Some(parent) = abs.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(abs, content).unwrap();
    }

    fn read_file(root: &Path, rel: &str) -> Option<String> {
        fs::read_to_string(root.join(rel)).ok()
    }

    #[test]
    fn begin_turn_creates_dirs_and_manifest() {
        let dir = temp_project();
        let store = CheckpointStore::new(dir.path());
        store.begin_turn("t1").unwrap();
        assert!(dir
            .path()
            .join(".wgenty-code/checkpoints/t1/blobs")
            .is_dir());
        assert!(dir
            .path()
            .join(".wgenty-code/checkpoints/t1/manifest.json")
            .is_file());
    }

    #[test]
    fn begin_turn_is_idempotent() {
        let dir = temp_project();
        let store = CheckpointStore::new(dir.path());
        store.begin_turn("t1").unwrap();
        store.begin_turn("t1").unwrap();
        assert!(dir
            .path()
            .join(".wgenty-code/checkpoints/t1/manifest.json")
            .is_file());
    }

    #[test]
    fn capture_saved_file_stores_blob_and_entry() {
        let dir = temp_project();
        let store = CheckpointStore::new(dir.path());
        write_file(dir.path(), "src/a.rs", "fn main() {}");
        store.begin_turn("t1").unwrap();
        store.try_capture_file("t1", "src/a.rs").unwrap();

        let m = store.read_manifest("t1").unwrap();
        assert_eq!(m.files.len(), 1);
        assert_eq!(m.files[0].state, FileState::Saved);
        assert!(m.files[0].blob.is_some());
        let blob = m.files[0].blob.clone().unwrap();
        let blob_content = fs::read_to_string(
            dir.path()
                .join(format!(".wgenty-code/checkpoints/t1/blobs/{}", blob)),
        )
        .unwrap();
        assert_eq!(blob_content, "fn main() {}");
    }

    #[test]
    fn capture_same_file_twice_is_noop() {
        let dir = temp_project();
        let store = CheckpointStore::new(dir.path());
        write_file(dir.path(), "a.txt", "hi");
        store.begin_turn("t1").unwrap();
        store.try_capture_file("t1", "a.txt").unwrap();
        store.try_capture_file("t1", "a.txt").unwrap();
        let m = store.read_manifest("t1").unwrap();
        assert_eq!(m.files.len(), 1);
    }

    #[test]
    fn capture_nonexistent_file_is_tombstone_without_blob() {
        let dir = temp_project();
        let store = CheckpointStore::new(dir.path());
        store.begin_turn("t1").unwrap();
        store.try_capture_file("t1", "new.txt").unwrap();
        let m = store.read_manifest("t1").unwrap();
        assert_eq!(m.files.len(), 1);
        assert_eq!(m.files[0].state, FileState::Tombstone);
        assert!(m.files[0].blob.is_none());
    }

    #[test]
    fn capture_binary_file_is_skipped() {
        let dir = temp_project();
        let store = CheckpointStore::new(dir.path());
        write_file(dir.path(), "img.png", "fake-png");
        store.begin_turn("t1").unwrap();
        store.try_capture_file("t1", "img.png").unwrap();
        let m = store.read_manifest("t1").unwrap();
        assert_eq!(m.files[0].state, FileState::Skipped);
        assert!(m.files[0].blob.is_none());
    }

    #[test]
    fn rewind_restores_saved_file_to_pre_edit_content() {
        let dir = temp_project();
        let store = CheckpointStore::new(dir.path());
        write_file(dir.path(), "a.txt", "original");
        store.begin_turn("t1").unwrap();
        store.try_capture_file("t1", "a.txt").unwrap();
        // simulate edit
        write_file(dir.path(), "a.txt", "modified");
        assert_eq!(read_file(dir.path(), "a.txt"), Some("modified".into()));
        let summary = store.rewind("t1").unwrap();
        assert!(summary.contains("restored 1"));
        assert_eq!(read_file(dir.path(), "a.txt"), Some("original".into()));
    }

    #[test]
    fn rewind_deletes_file_that_did_not_exist_pre_turn() {
        let dir = temp_project();
        let store = CheckpointStore::new(dir.path());
        store.begin_turn("t1").unwrap();
        store.try_capture_file("t1", "created.txt").unwrap();
        // simulate creation by the turn
        write_file(dir.path(), "created.txt", "new content");
        assert!(dir.path().join("created.txt").exists());
        store.rewind("t1").unwrap();
        assert!(!dir.path().join("created.txt").exists());
    }

    #[test]
    fn rewind_preserves_unrelated_untracked_file() {
        let dir = temp_project();
        let store = CheckpointStore::new(dir.path());
        write_file(dir.path(), "tracked.txt", "v1");
        write_file(dir.path(), "unrelated.txt", "leave me alone");
        store.begin_turn("t1").unwrap();
        store.try_capture_file("t1", "tracked.txt").unwrap();
        write_file(dir.path(), "tracked.txt", "v2");
        store.rewind("t1").unwrap();
        assert_eq!(read_file(dir.path(), "tracked.txt"), Some("v1".into()));
        assert_eq!(
            read_file(dir.path(), "unrelated.txt"),
            Some("leave me alone".into())
        );
    }

    #[test]
    fn rewind_reports_skipped_binary() {
        let dir = temp_project();
        let store = CheckpointStore::new(dir.path());
        write_file(dir.path(), "img.png", "binarydata");
        store.begin_turn("t1").unwrap();
        store.try_capture_file("t1", "img.png").unwrap();
        let summary = store.rewind("t1").unwrap();
        assert!(summary.contains("skipped 1"));
    }

    #[test]
    fn prune_keeps_newest_n() {
        let dir = temp_project();
        let store = CheckpointStore::new(dir.path());
        for i in 0..5 {
            let id = format!("t{}", i);
            store.begin_turn(&id).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
        let deleted = store.prune(2).unwrap();
        assert_eq!(deleted, 3);
        let remaining = store.list().unwrap();
        assert_eq!(remaining.len(), 2);
    }

    #[test]
    fn list_returns_newest_first() {
        let dir = temp_project();
        let store = CheckpointStore::new(dir.path());
        store.begin_turn("old").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        store.begin_turn("new").unwrap();
        let list = store.list().unwrap();
        assert_eq!(list.len(), 2);
        // newest first: created_at of "new" >= "old"
        assert!(list[0].created_at >= list[1].created_at);
    }

    #[test]
    fn list_empty_when_no_checkpoints() {
        let dir = temp_project();
        let store = CheckpointStore::new(dir.path());
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn capture_files_handles_multiple() {
        let dir = temp_project();
        let store = CheckpointStore::new(dir.path());
        write_file(dir.path(), "a.txt", "a");
        write_file(dir.path(), "b.txt", "b");
        store.begin_turn("t1").unwrap();
        store
            .capture_files("t1", &["a.txt".into(), "b.txt".into()])
            .unwrap();
        let m = store.read_manifest("t1").unwrap();
        assert_eq!(m.files.len(), 2);
    }
}
