//! Session coordinator: orchestrates the turn chain and links each turn to a
//! [`CheckpointStore`] snapshot entry.
//!
//! Task 1 scope: `begin_turn` / `end_turn` maintain the turn chain and persist
//! `session.json`. Git refs + untracked recording (Task 3), rollback (Task 4),
//! and verify-gate (Task 5) are layered on top in later tasks.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::tools::checkpoint_store::CheckpointStore;

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
    pub fn begin_turn(&mut self) -> Result<&TurnRecord> {
        let turn_id = format!("turn-{}", self.session.turns.len());
        let parent = self.session.current_turn.clone();
        let checkpoint_turn_id = uuid::Uuid::new_v4().to_string();
        self.checkpoint_store
            .begin_turn(&checkpoint_turn_id)
            .with_context(|| format!("checkpoint begin_turn: {}", checkpoint_turn_id))?;
        let now = chrono::Utc::now().to_rfc3339();
        let turn = TurnRecord {
            turn_id: turn_id.clone(),
            parent,
            checkpoint_turn_id,
            // Task 3 fills these at the turn boundary.
            git_refs: None,
            untracked_files: Vec::new(),
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
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
