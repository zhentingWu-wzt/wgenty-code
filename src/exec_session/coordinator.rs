//! Session coordinator: orchestrates the turn chain and links each turn to a
//! [`CheckpointStore`] snapshot entry.
//!
//! Task 1 scope: `begin_turn` / `end_turn` maintain the turn chain and persist
//! `session.json`. Git refs + untracked recording (Task 3), rollback (Task 4),
//! and verify-gate (Task 5) are layered on top in later tasks.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::tools::checkpoint_store::{CheckpointStore, FileState, Manifest};

use super::git::{record_git_state, run_git};
use super::hooks::{RollbackContext, SessionHooks};
use super::session::{SessionSource, SessionState, SessionStatus, TurnRecord};

/// Coordinates the inner ExecutionSession lifecycle for one session.
///
/// Holds a shared [`CheckpointStore`] (reused, not wrapped — file-blob capture
/// stays in `CheckpointStore`) and the session metadata dir
/// `<project>/.wgenty-code/snapshots/<session_id>/`.
pub struct SessionCoordinator {
    session: SessionState,
    session_dir: PathBuf,
    checkpoint_store: Arc<CheckpointStore>,
    project_root: PathBuf,
}

impl SessionCoordinator {
    /// Create a new session: ensures the session dir exists and writes the
    /// initial `session.json` (status `InProgress`, no turns).
    pub fn new(
        session_id: String,
        source: SessionSource,
        project_root: &Path,
        checkpoint_store: Arc<CheckpointStore>,
    ) -> Result<Self> {
        let session_dir = project_root
            .join(".wgenty-code")
            .join("snapshots")
            .join(&session_id);
        std::fs::create_dir_all(&session_dir)
            .with_context(|| format!("create session dir: {}", session_dir.display()))?;
        let session = SessionState::new(session_id, source);
        session.save(&session_dir)?;
        Ok(Self {
            session,
            session_dir,
            checkpoint_store,
            project_root: project_root.to_path_buf(),
        })
    }

    /// Start a new turn: records `parent` (previous `current_turn` or `None`),
    /// allocates a fresh `checkpoint_turn_id` and tells [`CheckpointStore`] to
    /// begin that snapshot, appends the turn to the chain, and sets
    /// `current_turn` to it. Returns the newly created turn record.
    ///
    /// `turn_id` follows the `turn-{n}` scheme where `n` is the new turn's
    /// index in the chain. `checkpoint_turn_id` is a UUID — `CheckpointStore`
    /// has no notion of a "current" turn id, so the coordinator owns the id
    /// allocation and passes it in.
    ///
    /// At the turn boundary, `HEAD` and untracked files are recorded via
    /// [`record_git_state`]; non-git projects degrade to `None` / empty (file
    /// rollback via CheckpointStore still applies). See spec §3.2.
    pub fn begin_turn(&mut self) -> Result<&TurnRecord> {
        let turn_id = format!("turn-{}", self.session.turns.len());
        let parent = self.session.current_turn.clone();
        let checkpoint_turn_id = uuid::Uuid::new_v4().to_string();
        self.checkpoint_store
            .begin_turn(&checkpoint_turn_id)
            .with_context(|| format!("checkpoint begin_turn: {}", checkpoint_turn_id))?;
        let (git_refs, untracked_files) = record_git_state(&self.project_root);
        let now = chrono::Utc::now().to_rfc3339();
        let turn = TurnRecord {
            turn_id: turn_id.clone(),
            parent,
            checkpoint_turn_id,
            git_refs,
            untracked_files,
            created_at: now.clone(),
        };
        self.session.turns.push(turn);
        self.session.current_turn = Some(turn_id);
        self.session.updated_at = now;
        self.session.save(&self.session_dir)?;
        Ok(self.session.turns.last().expect("just pushed"))
    }

    /// Seal the current turn. Task 1: persists `session.json` with refreshed
    /// `updated_at`. The turn record itself is complete from `begin_turn`;
    /// later tasks may attach post-turn metadata here. Errors if no turn is
    /// active.
    pub fn end_turn(&mut self) -> Result<()> {
        if self.session.current_turn.is_none() {
            anyhow::bail!("end_turn called with no active turn");
        }
        self.session.updated_at = chrono::Utc::now().to_rfc3339();
        self.session.save(&self.session_dir)
    }

    /// Borrow the current session state (read-only).
    pub fn session(&self) -> &SessionState {
        &self.session
    }

    /// Borrow the session dir (for later tasks / tests).
    pub fn session_dir(&self) -> &Path {
        &self.session_dir
    }

    /// Borrow the project root (for later tasks: rollback runs `git reset`
    /// here).
    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    /// Borrow the shared checkpoint store (for later tasks: capture / rewind).
    pub fn checkpoint_store(&self) -> &CheckpointStore {
        &self.checkpoint_store
    }

    /// Update session status (Task 5 / Task 6 transition into Completed /
    /// Unverified / Failed). Persists immediately.
    pub fn set_status(&mut self, status: SessionStatus) -> Result<()> {
        self.session.set_status(status);
        self.session.save(&self.session_dir)
    }

    /// Roll the workspace back to the state it had at the start of `turn_id`
    /// (spec §3.4). The rollback has three stages, applied in order:
    ///
    /// 1. **git reset --hard** to `turn.git_refs.head` - but only when the
    ///    current `HEAD` differs from the turn-start SHA. Skipped for non-git
    ///    projects (no `git_refs`) or when `HEAD` is already at the target.
    ///    This restores tracked files to the turn-start commit.
    /// 2. **CheckpointStore::rewind** to `turn.checkpoint_turn_id` - restores
    ///    pre-edit content for files captured during the turn, and deletes
    ///    files that were created (recorded as `Tombstone`). This covers
    ///    file-edit / file-write / apply-patch mutations.
    /// 3. **Delete newly-created untracked files** - files untracked *now*
    ///    that were NOT in `turn.untracked_files` (i.e. created during the
    ///    turn via non-checkpointed means, e.g. `exec_command`). Pre-existing
    ///    untracked files (present at turn start) are preserved. This does
    ///    NOT use `git clean -fd` - it diffs the lists so it never touches
    ///    files the user had before the turn.
    ///
    /// After the workspace is restored, `current_turn` moves to `turn_id` and
    /// `session.json` is persisted. The [`SessionHooks::rollback_triggered`]
    /// hook is invoked before the workspace changes (informational; does not
    /// gate the rollback).
    ///
    /// Returns a [`RollbackResult`] describing what was done. Errors propagate
    /// (e.g. `git reset` failure, rewind failure) - on error the workspace may
    /// be partially rolled back; the caller / outer layer decides recovery.
    pub fn rollback_to(
        &mut self,
        turn_id: &str,
        hooks: &dyn SessionHooks,
    ) -> Result<RollbackResult> {
        // Locate the target turn (clone so we can mutate self.session freely).
        let target = self
            .session
            .turns
            .iter()
            .find(|t| t.turn_id == turn_id)
            .cloned()
            .with_context(|| format!("rollback target turn not found: {turn_id}"))?;

        // Notify the hook before mutating the workspace.
        hooks.rollback_triggered(&RollbackContext {
            session_id: self.session.session_id.clone(),
            from_turn: self.session.current_turn.clone(),
            to_turn: turn_id.to_string(),
        });

        // Stage 1: git reset --hard (only if HEAD moved).
        let git_reset = self.reset_git_head_if_needed(&target)?;

        // Stage 2: CheckpointStore::rewind (restore pre-edit / delete tombstones).
        let restored_files = self.collect_rewind_files(&target.checkpoint_turn_id);
        self.checkpoint_store
            .rewind(&target.checkpoint_turn_id)
            .with_context(|| format!("checkpoint rewind: {}", target.checkpoint_turn_id))?;

        // Stage 3: delete newly-created untracked (current - turn-start set).
        let deleted_untracked = self.delete_new_untracked(&target.untracked_files)?;

        // Move the cursor and persist.
        self.session.current_turn = Some(turn_id.to_string());
        self.session.updated_at = chrono::Utc::now().to_rfc3339();
        self.session.save(&self.session_dir)?;

        Ok(RollbackResult {
            git_reset,
            restored_files,
            deleted_untracked,
        })
    }

    /// Stage 1 helper: `git reset --hard <target.head>` when the current HEAD
    /// differs from the turn-start SHA. Returns `true` if reset ran. No-op for
    /// non-git projects (`git_refs` is `None`).
    fn reset_git_head_if_needed(&self, target: &TurnRecord) -> Result<bool> {
        let target_refs = match &target.git_refs {
            Some(refs) => refs,
            None => return Ok(false),
        };
        // Read current HEAD; degrade to "unknown" on error (skip reset).
        let current_head = run_git(&self.project_root, &["rev-parse", "HEAD"]).ok();
        if current_head.as_deref() == Some(target_refs.head.as_str()) {
            return Ok(false);
        }
        run_git(&self.project_root, &["reset", "--hard", &target_refs.head]).with_context(
            || {
                format!(
                    "git reset --hard {} (rollback to {})",
                    target_refs.head, target.turn_id
                )
            },
        )?;
        Ok(true)
    }

    /// Stage 2 helper: read the checkpoint manifest directly (without mutating
    /// `CheckpointStore`) to enumerate the files `rewind` will touch. Returns
    /// relative paths for entries with state `Saved` or `Tombstone`; `Skipped`
    /// entries are excluded (rewind does nothing with them). Best-effort: an
    /// unreadable / missing manifest yields an empty list - `rewind` itself is
    /// the source of truth and will error if the snapshot is truly gone.
    fn collect_rewind_files(&self, checkpoint_turn_id: &str) -> Vec<PathBuf> {
        let manifest_path = self
            .project_root
            .join(".wgenty-code")
            .join("checkpoints")
            .join(checkpoint_turn_id)
            .join("manifest.json");
        let data = match std::fs::read_to_string(&manifest_path) {
            Ok(d) => d,
            Err(_) => return Vec::new(),
        };
        let manifest: Manifest = match serde_json::from_str(&data) {
            Ok(m) => m,
            Err(_) => return Vec::new(),
        };
        manifest
            .files
            .into_iter()
            .filter(|e| e.state != FileState::Skipped)
            .map(|e| PathBuf::from(e.path))
            .collect()
    }

    /// Stage 3 helper: delete untracked files that exist now but were NOT
    /// present at the target turn's start. `turn_start_untracked` is the
    /// baseline (preserved set). Returns the relative paths of files deleted.
    /// No-op for non-git projects (`record_git_state` returns an empty current
    /// list, so the diff is empty).
    fn delete_new_untracked(&self, turn_start_untracked: &[String]) -> Result<Vec<PathBuf>> {
        let baseline: HashSet<&str> = turn_start_untracked.iter().map(|s| s.as_str()).collect();
        let (_refs, current_untracked) = record_git_state(&self.project_root);
        let mut deleted = Vec::new();
        for rel in &current_untracked {
            if baseline.contains(rel.as_str()) {
                continue;
            }
            let abs = self.project_root.join(rel);
            if abs.exists() {
                std::fs::remove_file(&abs)
                    .with_context(|| format!("delete untracked: {}", abs.display()))?;
                deleted.push(PathBuf::from(rel));
            }
        }
        Ok(deleted)
    }
}

/// Outcome of [`SessionCoordinator::rollback_to`].
///
/// All paths are workspace-relative (as `git` / the checkpoint manifest
/// report them). `git_reset` is `true` only when a `git reset --hard` actually
/// ran; `false` when it was skipped (non-git project, or HEAD already at the
/// target SHA).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollbackResult {
    pub git_reset: bool,
    pub restored_files: Vec<PathBuf>,
    pub deleted_untracked: Vec<PathBuf>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec_session::NoHooks;
    use tempfile::tempdir;

    fn make_coordinator(dir: &Path) -> SessionCoordinator {
        let store = Arc::new(CheckpointStore::new(dir));
        SessionCoordinator::new("es-test".into(), SessionSource::AgentSelf, dir, store).unwrap()
    }

    #[test]
    fn new_creates_session_dir_and_json() {
        let dir = tempdir().unwrap();
        let coord = make_coordinator(dir.path());
        assert!(coord.session_dir().join("session.json").exists());
        assert_eq!(coord.session().status, SessionStatus::InProgress);
        assert!(coord.session().turns.is_empty());
        assert_eq!(coord.session().current_turn, None);
        // session_dir is <project>/.wgenty-code/snapshots/<session_id>/
        assert!(coord
            .session_dir()
            .ends_with(".wgenty-code/snapshots/es-test"));
    }

    #[test]
    fn begin_turn_appends_and_sets_current() {
        let dir = tempdir().unwrap();
        let mut coord = make_coordinator(dir.path());
        let turn_id = coord.begin_turn().unwrap().turn_id.clone();
        assert_eq!(turn_id, "turn-0");
        assert_eq!(coord.session().turns.len(), 1);
        assert_eq!(coord.session().current_turn.as_deref(), Some("turn-0"));
        let t = &coord.session().turns[0];
        assert_eq!(t.turn_id, "turn-0");
        assert_eq!(t.parent, None);
        assert!(!t.checkpoint_turn_id.is_empty());
        // Non-git tempdir: record_git_state degrades to None / empty.
        assert_eq!(t.git_refs, None);
        assert!(t.untracked_files.is_empty());
    }

    #[test]
    fn begin_turn_links_checkpoint_store() {
        let dir = tempdir().unwrap();
        let mut coord = make_coordinator(dir.path());
        coord.begin_turn().unwrap();
        let ct_id = coord.session().turns[0].checkpoint_turn_id.clone();
        // CheckpointStore should have created the turn dir + manifest.
        let checkpoint_root = dir.path().join(".wgenty-code").join("checkpoints");
        assert!(checkpoint_root.join(&ct_id).join("manifest.json").exists());
    }

    #[test]
    fn end_turn_persists_and_errors_without_active_turn() {
        let dir = tempdir().unwrap();
        let mut coord = make_coordinator(dir.path());
        let err = coord.end_turn().unwrap_err();
        assert!(format!("{err}").contains("no active turn"));
        coord.begin_turn().unwrap();
        coord.end_turn().unwrap();
        let loaded = SessionState::load(coord.session_dir()).unwrap();
        assert_eq!(loaded.turns.len(), 1);
        assert_eq!(loaded.current_turn.as_deref(), Some("turn-0"));
    }

    #[test]
    fn consecutive_begin_end_form_chain() {
        let dir = tempdir().unwrap();
        let mut coord = make_coordinator(dir.path());
        coord.begin_turn().unwrap();
        coord.end_turn().unwrap();
        coord.begin_turn().unwrap();
        coord.end_turn().unwrap();
        coord.begin_turn().unwrap();
        coord.end_turn().unwrap();
        assert_eq!(coord.session().turns.len(), 3);
        assert_eq!(coord.session().turns[0].parent, None);
        assert_eq!(coord.session().turns[1].parent.as_deref(), Some("turn-0"));
        assert_eq!(coord.session().turns[2].parent.as_deref(), Some("turn-1"));
        assert_eq!(coord.session().current_turn.as_deref(), Some("turn-2"));
        let ids: Vec<_> = coord
            .session()
            .turns
            .iter()
            .map(|t| &t.checkpoint_turn_id)
            .collect();
        assert_eq!(ids.len(), 3);
        assert!(ids.windows(2).all(|w| w[0] != w[1]));
    }

    #[test]
    fn session_json_persists_across_writes() {
        let dir = tempdir().unwrap();
        let mut coord = make_coordinator(dir.path());
        coord.begin_turn().unwrap();
        coord.end_turn().unwrap();
        let loaded = SessionState::load(coord.session_dir()).unwrap();
        assert_eq!(loaded.session_id, "es-test");
        assert_eq!(loaded.turns.len(), 1);
        assert_eq!(loaded.current_turn.as_deref(), Some("turn-0"));
    }

    #[test]
    fn set_status_persists() {
        let dir = tempdir().unwrap();
        let mut coord = make_coordinator(dir.path());
        coord.set_status(SessionStatus::Failed).unwrap();
        assert_eq!(coord.session().status, SessionStatus::Failed);
        let loaded = SessionState::load(coord.session_dir()).unwrap();
        assert_eq!(loaded.status, SessionStatus::Failed);
    }

    #[test]
    fn no_tmp_residue_after_begin_end() {
        let dir = tempdir().unwrap();
        let mut coord = make_coordinator(dir.path());
        coord.begin_turn().unwrap();
        coord.end_turn().unwrap();
        assert!(
            !coord.session_dir().join("session.json.tmp").exists(),
            "stale .tmp left behind"
        );
    }

    /// Initialize a git repo with one commit so `HEAD` exists. Writes a
    /// `.gitignore` so checkpoint artifacts and test scaffolding do not
    /// pollute the untracked list.
    fn init_git_repo(dir: &Path) {
        use std::process::Command;
        let cmds: &[&[&str]] = &[
            &["init"],
            &["config", "user.email", "test@wgenty.local"],
            &["config", "user.name", "wgenty test"],
        ];
        for args in cmds {
            let status = Command::new("git")
                .args(*args)
                .current_dir(dir)
                .status()
                .expect("spawn git");
            assert!(status.success(), "git {:?} failed", args);
        }
        // Ignore checkpoint artifacts + tmp scratch files.
        std::fs::write(dir.join(".gitignore"), ".wgenty-code/\n*.tmp\n").unwrap();
        std::fs::write(dir.join("seed.txt"), "seed\n").unwrap();
        let status = Command::new("git")
            .args(["add", "."])
            .current_dir(dir)
            .status()
            .expect("git add");
        assert!(status.success(), "git add failed");
        let status = Command::new("git")
            .args(["commit", "-m", "seed"])
            .current_dir(dir)
            .status()
            .expect("git commit");
        assert!(status.success(), "git commit failed");
    }

    #[test]
    fn begin_turn_records_git_head_in_git_repo() {
        let dir = tempdir().unwrap();
        init_git_repo(dir.path());
        let mut coord = make_coordinator(dir.path());
        coord.begin_turn().unwrap();
        let t = &coord.session().turns[0];
        let refs = t
            .git_refs
            .as_ref()
            .expect("git repo should record HEAD at begin_turn");
        assert!(!refs.head.is_empty(), "HEAD sha should be non-empty");
        assert!(
            refs.head.chars().all(|c| c.is_ascii_hexdigit()),
            "HEAD should be hex: {}",
            refs.head
        );
    }

    #[test]
    fn begin_turn_records_untracked_files_in_git_repo() {
        let dir = tempdir().unwrap();
        init_git_repo(dir.path());
        std::fs::write(dir.path().join("scratch.log"), "x\n").unwrap();
        let mut coord = make_coordinator(dir.path());
        coord.begin_turn().unwrap();
        let t = &coord.session().turns[0];
        assert!(
            t.untracked_files.iter().any(|f| f == "scratch.log"),
            "scratch.log should be recorded as untracked: {:?}",
            t.untracked_files
        );
        assert!(
            !t.untracked_files.iter().any(|f| f == "seed.txt"),
            "seed.txt is tracked, must not be untracked: {:?}",
            t.untracked_files
        );
        // .wgenty-code/ is gitignored, so checkpoint artifacts must not appear.
        assert!(
            !t.untracked_files
                .iter()
                .any(|f| f.starts_with(".wgenty-code/")),
            ".wgenty-code/ is gitignored, must not appear: {:?}",
            t.untracked_files
        );
    }

    // ---- Task 4: rollback_to tests ----

    /// Run `git` in `dir` and assert success; return trimmed stdout.
    fn git_run(dir: &Path, args: &[&str]) -> String {
        use std::process::Command;
        let output = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("spawn git");
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    /// `git commit -am <msg>` helper for test setup.
    fn git_commit(dir: &Path, msg: &str) {
        git_run(dir, &["add", "."]);
        git_run(dir, &["commit", "-m", msg]);
    }

    #[test]
    fn rollback_resets_git_head_when_commit_made() {
        let dir = tempdir().unwrap();
        init_git_repo(dir.path());
        let mut coord = make_coordinator(dir.path());
        coord.begin_turn().unwrap(); // turn-0, head = sha0
        let sha0 = coord.session().turns[0]
            .git_refs
            .as_ref()
            .expect("git_refs")
            .head
            .clone();
        // Modify a tracked file and commit -> HEAD advances.
        std::fs::write(dir.path().join("seed.txt"), "modified\n").unwrap();
        git_commit(dir.path(), "change seed");
        let sha1 = git_run(dir.path(), &["rev-parse", "HEAD"]);
        assert_ne!(sha0, sha1, "commit should advance HEAD");

        coord.begin_turn().unwrap(); // turn-1, head = sha1
        let result = coord.rollback_to("turn-0", &NoHooks).unwrap();
        assert!(result.git_reset, "git reset --hard should have run");
        // HEAD is back to turn-0's SHA.
        let head_after = git_run(dir.path(), &["rev-parse", "HEAD"]);
        assert_eq!(head_after, sha0, "HEAD should be reset to turn-0 start");
        // The committed change is reverted.
        assert_eq!(
            std::fs::read_to_string(dir.path().join("seed.txt")).unwrap(),
            "seed\n",
            "tracked file should be restored to turn-0 commit"
        );
    }

    #[test]
    fn rollback_skips_git_reset_when_head_unchanged() {
        let dir = tempdir().unwrap();
        init_git_repo(dir.path());
        let mut coord = make_coordinator(dir.path());
        coord.begin_turn().unwrap(); // turn-0
        let sha0 = coord.session().turns[0]
            .git_refs
            .as_ref()
            .expect("git_refs")
            .head
            .clone();
        // Checkpointed edit to a tracked file (no commit -> HEAD unchanged).
        let ct0 = coord.session().turns[0].checkpoint_turn_id.clone();
        coord
            .checkpoint_store()
            .try_capture_file(&ct0, "seed.txt")
            .unwrap();
        std::fs::write(dir.path().join("seed.txt"), "edited\n").unwrap();
        coord.begin_turn().unwrap(); // turn-1, head still sha0
        let result = coord.rollback_to("turn-0", &NoHooks).unwrap();
        assert!(
            !result.git_reset,
            "git reset should be skipped when HEAD unchanged"
        );
        assert_eq!(
            git_run(dir.path(), &["rev-parse", "HEAD"]),
            sha0,
            "HEAD must not move"
        );
        // Rewind still restored the checkpointed pre-edit content.
        assert_eq!(
            std::fs::read_to_string(dir.path().join("seed.txt")).unwrap(),
            "seed\n",
            "rewind should restore pre-edit content even without git reset"
        );
        assert!(
            result
                .restored_files
                .iter()
                .any(|p| p.to_string_lossy() == "seed.txt"),
            "restored_files should list seed.txt: {:?}",
            result.restored_files
        );
    }

    #[test]
    fn rollback_rewinds_pre_edit_and_deletes_tombstone() {
        let dir = tempdir().unwrap();
        init_git_repo(dir.path());
        // a.txt is a TRACKED file at turn-0 start (committed before begin_turn).
        // This is the "file existed, was edited" case -> Saved on rewind.
        std::fs::write(dir.path().join("a.txt"), "original\n").unwrap();
        git_run(dir.path(), &["add", "a.txt"]);
        git_run(dir.path(), &["commit", "-m", "add a.txt"]);
        let mut coord = make_coordinator(dir.path());
        coord.begin_turn().unwrap(); // turn-0
        let ct0 = coord.session().turns[0].checkpoint_turn_id.clone();
        // File A (tracked): pre-edit captured, then modified.
        coord
            .checkpoint_store()
            .try_capture_file(&ct0, "a.txt")
            .unwrap();
        std::fs::write(dir.path().join("a.txt"), "modified\n").unwrap();
        // File B (new): does not exist at capture -> Tombstone; then created.
        coord
            .checkpoint_store()
            .try_capture_file(&ct0, "b_new.txt")
            .unwrap();
        std::fs::write(dir.path().join("b_new.txt"), "created\n").unwrap();
        coord.begin_turn().unwrap(); // turn-1
        let result = coord.rollback_to("turn-0", &NoHooks).unwrap();
        // Pre-edit content restored (tracked file, rewind restores it; not
        // touched by delete-untracked since it's tracked).
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            "original\n",
            "rewind should restore a.txt pre-edit content"
        );
        // Tombstoned new file deleted (by rewind; also would be caught by
        // delete-untracked since it wasn't in turn-0 baseline).
        assert!(
            !dir.path().join("b_new.txt").exists(),
            "rewind should delete the tombstoned new file"
        );
        let restored: Vec<String> = result
            .restored_files
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        assert!(
            restored.iter().any(|f| f == "a.txt"),
            "a.txt in restored_files: {:?}",
            restored
        );
        assert!(
            restored.iter().any(|f| f == "b_new.txt"),
            "b_new.txt in restored_files: {:?}",
            restored
        );
    }

    #[test]
    fn rollback_deletes_new_untracked_preserves_pre_existing() {
        let dir = tempdir().unwrap();
        init_git_repo(dir.path());
        // Pre-existing untracked file before turn-0.
        std::fs::write(dir.path().join("pre.log"), "pre\n").unwrap();
        let mut coord = make_coordinator(dir.path());
        coord.begin_turn().unwrap(); // turn-0, untracked = ["pre.log"]
        assert!(
            coord.session().turns[0]
                .untracked_files
                .iter()
                .any(|f| f == "pre.log"),
            "pre.log should be in turn-0 untracked baseline"
        );
        // New untracked file created during turn-0 (non-checkpointed).
        std::fs::write(dir.path().join("new.log"), "new\n").unwrap();
        coord.begin_turn().unwrap(); // turn-1
        let result = coord.rollback_to("turn-0", &NoHooks).unwrap();
        assert!(
            !dir.path().join("new.log").exists(),
            "new untracked file should be deleted"
        );
        assert!(
            dir.path().join("pre.log").exists(),
            "pre-existing untracked file must be preserved"
        );
        let deleted: Vec<String> = result
            .deleted_untracked
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        assert!(
            deleted.iter().any(|f| f == "new.log"),
            "new.log in deleted_untracked: {:?}",
            deleted
        );
        assert!(
            !deleted.iter().any(|f| f == "pre.log"),
            "pre.log must NOT be in deleted_untracked: {:?}",
            deleted
        );
    }

    #[test]
    fn rollback_in_non_git_project_only_rewinds() {
        let dir = tempdir().unwrap();
        // No git init - non-git project.
        let mut coord = make_coordinator(dir.path());
        coord.begin_turn().unwrap(); // turn-0, git_refs None, untracked []
        assert_eq!(coord.session().turns[0].git_refs, None);
        let ct0 = coord.session().turns[0].checkpoint_turn_id.clone();
        std::fs::write(dir.path().join("x.txt"), "original\n").unwrap();
        coord
            .checkpoint_store()
            .try_capture_file(&ct0, "x.txt")
            .unwrap();
        std::fs::write(dir.path().join("x.txt"), "edited\n").unwrap();
        coord.begin_turn().unwrap(); // turn-1
        let result = coord.rollback_to("turn-0", &NoHooks).unwrap();
        assert!(!result.git_reset, "non-git project must skip git reset");
        assert!(
            result.deleted_untracked.is_empty(),
            "non-git project has no untracked deletion"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("x.txt")).unwrap(),
            "original\n",
            "rewind should still restore pre-edit in non-git project"
        );
    }

    #[test]
    fn rollback_invokes_hook_with_from_and_to() {
        use std::sync::Mutex;

        #[derive(Default)]
        struct RecordingHook {
            seen: Mutex<Vec<RollbackContext>>,
        }
        impl SessionHooks for RecordingHook {
            fn rollback_triggered(&self, ctx: &RollbackContext) {
                self.seen.lock().unwrap().push(ctx.clone());
            }
        }

        let dir = tempdir().unwrap();
        let mut coord = make_coordinator(dir.path());
        coord.begin_turn().unwrap(); // turn-0
        coord.begin_turn().unwrap(); // turn-1, current = turn-1
        let hooks = RecordingHook::default();
        coord.rollback_to("turn-0", &hooks).unwrap();
        let seen = hooks.seen.lock().unwrap();
        assert_eq!(seen.len(), 1, "rollback_triggered should fire once");
        assert_eq!(seen[0].from_turn.as_deref(), Some("turn-1"));
        assert_eq!(seen[0].to_turn, "turn-0");
        assert_eq!(seen[0].session_id, "es-test");
    }

    #[test]
    fn rollback_updates_current_turn_and_persists() {
        let dir = tempdir().unwrap();
        let mut coord = make_coordinator(dir.path());
        coord.begin_turn().unwrap(); // turn-0
        coord.begin_turn().unwrap(); // turn-1
        assert_eq!(coord.session().current_turn.as_deref(), Some("turn-1"));
        coord.rollback_to("turn-0", &NoHooks).unwrap();
        assert_eq!(
            coord.session().current_turn.as_deref(),
            Some("turn-0"),
            "current_turn should move back to turn-0"
        );
        let loaded = SessionState::load(coord.session_dir()).unwrap();
        assert_eq!(loaded.current_turn.as_deref(), Some("turn-0"));
    }

    #[test]
    fn rollback_unknown_turn_errors() {
        let dir = tempdir().unwrap();
        let mut coord = make_coordinator(dir.path());
        coord.begin_turn().unwrap(); // turn-0
        let err = coord.rollback_to("turn-99", &NoHooks).unwrap_err();
        assert!(
            format!("{err}").contains("not found"),
            "error should mention not found: {err}"
        );
        // current_turn unchanged.
        assert_eq!(coord.session().current_turn.as_deref(), Some("turn-0"));
    }
}
