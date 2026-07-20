//! Session-level state for the ExecutionSession inner layer.
//!
//! [`SessionState`] is the JSON-serializable record persisted at
//! `<project>/.wgenty-code/snapshots/<session_id>/session.json`. It tracks the
//! turn chain, session status, and a `current_turn` cursor that the outer
//! ExecutionSession reads on cross-process resume. File-blob snapshots are NOT
//! stored here — they live in
//! [`crate::tools::checkpoint_store::CheckpointStore`]; this struct only holds
//! the metadata that links turns to checkpoint snapshots.
//!
//! Task 1 scope: state shape + atomic load/save + status transitions. Git
//! refs / untracked recording (Task 3), rollback (Task 4), and verify-gate
//! (Task 5) layer on top in later tasks.

use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const SESSION_FILE: &str = "session.json";
const SESSION_TMP: &str = "session.json.tmp";

/// Who initiated the session. ExecutionSession does not probe whether any
/// flow-orchestration skill is installed — the caller decides the source and
/// supplies hooks. The decoupling invariant: this module contains no
/// references to specific skill names beyond this enum variant.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SessionSource {
    Comet,
    AgentSelf,
    UserDirect,
}

/// Lifecycle status of a session.
///
/// - `InProgress` — default; turns may be begun/ended.
/// - `Completed` — `verify_and_complete` (Task 5) passed.
/// - `Unverified` — agent ended without calling verify (Task 6 fallback).
/// - `Failed` — repeated verify failures exceeded the retry budget.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    InProgress,
    Completed,
    Unverified,
    Failed,
}

/// Git references captured at a turn boundary. `head` is the SHA recorded at
/// `begin_turn`; rollback (Task 4) `git reset --hard`s back to it. `None` in
/// non-git projects (graceful degradation — file rollback via CheckpointStore
/// still works).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitRefs {
    pub head: String,
}

/// One entry in the session turn chain. Created at `begin_turn`, sealed at
/// `end_turn`. `checkpoint_turn_id` links this turn to its file-snapshot entry
/// in `CheckpointStore`. `git_refs` / `untracked_files` are filled by Task 3;
/// they are `None` / empty for Task 1.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TurnRecord {
    pub turn_id: String,
    pub parent: Option<String>,
    pub checkpoint_turn_id: String,
    pub git_refs: Option<GitRefs>,
    pub untracked_files: Vec<String>,
    pub created_at: String,
}

/// Persistent session metadata: the turn chain, status, and current-turn
/// cursor. Serialized as `session.json` in the session dir.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionState {
    pub session_id: String,
    pub source: SessionSource,
    pub status: SessionStatus,
    pub created_at: String,
    pub updated_at: String,
    pub turns: Vec<TurnRecord>,
    pub current_turn: Option<String>,
    /// Node chain for the outer-layer node state machine. Backward compat:
    /// old sessions without this field deserialize as empty.
    #[serde(default)]
    pub node_states: Vec<crate::exec_session::node::Node>,
    /// Current node cursor (outer layer). `None` when no node is active.
    #[serde(default)]
    pub current_node: Option<String>,
}

impl SessionState {
    /// Create a fresh session in `InProgress` with no turns and no current
    /// cursor.
    pub fn new(session_id: String, source: SessionSource) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        Self {
            session_id,
            source,
            status: SessionStatus::InProgress,
            created_at: now.clone(),
            updated_at: now,
            turns: Vec::new(),
            current_turn: None,
            node_states: Vec::new(),
            current_node: None,
        }
    }

    /// Load `session.json` from `dir`. Returns an error if the file is missing
    /// or unparseable; the resume caller is expected to degrade on error
    /// (outer ExecutionSession, future task).
    pub fn load(dir: &Path) -> Result<Self> {
        let path = dir.join(SESSION_FILE);
        let data = std::fs::read_to_string(&path)
            .with_context(|| format!("read session.json: {}", path.display()))?;
        serde_json::from_str(&data)
            .with_context(|| format!("parse session.json: {}", path.display()))
    }

    /// Atomically write `session.json` via tmp + rename.
    ///
    /// L1 assumption: single writer per session (no concurrent save). A crash
    /// between `write(tmp)` and `rename` leaves a stale `session.json.tmp` but
    /// the previous `session.json` (if any) is untouched; the next save
    /// overwrites the stale tmp. `rename` is atomic on the same filesystem.
    pub fn save(&self, dir: &Path) -> Result<()> {
        let tmp = dir.join(SESSION_TMP);
        let final_path = dir.join(SESSION_FILE);
        let data = serde_json::to_string_pretty(self).context("serialize session.json")?;
        std::fs::write(&tmp, &data)
            .with_context(|| format!("write session.json.tmp: {}", tmp.display()))?;
        std::fs::rename(&tmp, &final_path).with_context(|| {
            format!(
                "rename session.json.tmp -> session.json: {}",
                final_path.display()
            )
        })?;
        Ok(())
    }

    /// Update status and refresh `updated_at`.
    pub fn set_status(&mut self, status: SessionStatus) {
        self.status = status;
        self.updated_at = chrono::Utc::now().to_rfc3339();
    }

    /// Borrow the turn currently pointed to by `current_turn`, if any.
    /// Returns `None` when `current_turn` is `None` or points at a missing
    /// turn id (should not happen in normal flow, but defensive).
    pub fn current_turn_record(&self) -> Option<&TurnRecord> {
        let id = self.current_turn.as_ref()?;
        self.turns.iter().find(|t| &t.turn_id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn new_initializes_in_progress_empty_turns() {
        let s = SessionState::new("es-test".into(), SessionSource::AgentSelf);
        assert_eq!(s.session_id, "es-test");
        assert_eq!(s.source, SessionSource::AgentSelf);
        assert_eq!(s.status, SessionStatus::InProgress);
        assert!(s.turns.is_empty());
        assert_eq!(s.current_turn, None);
        assert_eq!(s.created_at, s.updated_at);
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = tempdir().unwrap();
        let mut s = SessionState::new("es-rt".into(), SessionSource::Comet);
        s.turns.push(TurnRecord {
            turn_id: "turn-0".into(),
            parent: None,
            checkpoint_turn_id: "ct-abc".into(),
            git_refs: Some(GitRefs {
                head: "deadbeef".into(),
            }),
            untracked_files: vec!["temp.tmp".into()],
            created_at: "2026-07-19T00:00:00+00:00".into(),
        });
        s.current_turn = Some("turn-0".into());
        s.save(dir.path()).unwrap();

        let loaded = SessionState::load(dir.path()).unwrap();
        assert_eq!(loaded, s);
    }

    #[test]
    fn load_missing_file_is_error() {
        let dir = tempdir().unwrap();
        let err = SessionState::load(dir.path()).unwrap_err();
        assert!(format!("{err}").contains("session.json"));
    }

    #[test]
    fn save_leaves_no_tmp_residue() {
        let dir = tempdir().unwrap();
        let s = SessionState::new("es-tmp".into(), SessionSource::UserDirect);
        s.save(dir.path()).unwrap();
        assert!(dir.path().join(SESSION_FILE).exists());
        assert!(
            !dir.path().join(SESSION_TMP).exists(),
            "stale .tmp left behind"
        );
    }

    #[test]
    fn save_is_idempotent_and_overwrites() {
        let dir = tempdir().unwrap();
        let mut s = SessionState::new("es-idem".into(), SessionSource::AgentSelf);
        s.save(dir.path()).unwrap();
        s.set_status(SessionStatus::Completed);
        s.save(dir.path()).unwrap();
        let loaded = SessionState::load(dir.path()).unwrap();
        assert_eq!(loaded.status, SessionStatus::Completed);
        assert!(!dir.path().join(SESSION_TMP).exists());
    }

    #[test]
    fn source_serde_kebab_case() {
        assert_eq!(
            serde_json::to_string(&SessionSource::AgentSelf).unwrap(),
            "\"agent-self\""
        );
        assert_eq!(
            serde_json::to_string(&SessionSource::Comet).unwrap(),
            "\"comet\""
        );
        assert_eq!(
            serde_json::to_string(&SessionSource::UserDirect).unwrap(),
            "\"user-direct\""
        );
        let s: SessionSource = serde_json::from_str("\"user-direct\"").unwrap();
        assert_eq!(s, SessionSource::UserDirect);
    }

    #[test]
    fn status_serde_snake_case() {
        assert_eq!(
            serde_json::to_string(&SessionStatus::InProgress).unwrap(),
            "\"in_progress\""
        );
        let s: SessionStatus = serde_json::from_str("\"unverified\"").unwrap();
        assert_eq!(s, SessionStatus::Unverified);
    }

    #[test]
    fn git_refs_and_turn_record_serde_round_trip() {
        let t = TurnRecord {
            turn_id: "turn-1".into(),
            parent: Some("turn-0".into()),
            checkpoint_turn_id: "ct-xyz".into(),
            git_refs: Some(GitRefs {
                head: "abc123".into(),
            }),
            untracked_files: vec!["a.tmp".into(), "b.tmp".into()],
            created_at: "2026-07-19T01:00:00+00:00".into(),
        };
        let json = serde_json::to_string(&t).unwrap();
        let back: TurnRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn turn_record_with_none_git_refs_serde() {
        let t = TurnRecord {
            turn_id: "turn-0".into(),
            parent: None,
            checkpoint_turn_id: "ct-0".into(),
            git_refs: None,
            untracked_files: vec![],
            created_at: "2026-07-19T00:00:00+00:00".into(),
        };
        let json = serde_json::to_string(&t).unwrap();
        assert!(json.contains("\"git_refs\":null"));
        assert!(json.contains("\"untracked_files\":[]"));
        let back: TurnRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn set_status_refreshes_updated_at() {
        let mut s = SessionState::new("es-st".into(), SessionSource::AgentSelf);
        let before = s.updated_at.clone();
        // rfc3339 has second precision; sleep past the boundary.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        s.set_status(SessionStatus::Completed);
        assert_eq!(s.status, SessionStatus::Completed);
        assert_ne!(s.updated_at, before);
    }

    #[test]
    fn current_turn_record_finds_match() {
        let mut s = SessionState::new("es-cur".into(), SessionSource::AgentSelf);
        s.turns.push(TurnRecord {
            turn_id: "turn-0".into(),
            parent: None,
            checkpoint_turn_id: "ct-0".into(),
            git_refs: None,
            untracked_files: vec![],
            created_at: "t0".into(),
        });
        s.turns.push(TurnRecord {
            turn_id: "turn-1".into(),
            parent: Some("turn-0".into()),
            checkpoint_turn_id: "ct-1".into(),
            git_refs: None,
            untracked_files: vec![],
            created_at: "t1".into(),
        });
        s.current_turn = Some("turn-1".into());
        assert_eq!(s.current_turn_record().unwrap().turn_id, "turn-1");

        s.current_turn = Some("nonexistent".into());
        assert!(s.current_turn_record().is_none());

        s.current_turn = None;
        assert!(s.current_turn_record().is_none());
    }

    #[test]
    fn new_initializes_empty_node_states() {
        let s = SessionState::new("es-nodes".into(), SessionSource::AgentSelf);
        assert!(s.node_states.is_empty());
        assert_eq!(s.current_node, None);
    }

    #[test]
    fn backward_compat_old_json_without_node_fields() {
        // Simulate an old session.json created before node_states/current_node
        // existed. serde(default) must fill in empty Vec and None.
        let old_json = r#"{
            "session_id": "es-old",
            "source": "agent-self",
            "status": "in_progress",
            "created_at": "2026-07-19T00:00:00+00:00",
            "updated_at": "2026-07-19T00:00:00+00:00",
            "turns": [],
            "current_turn": null
        }"#;
        let s: SessionState = serde_json::from_str(old_json).expect("deserialize old format");
        assert_eq!(s.session_id, "es-old");
        assert!(s.node_states.is_empty());
        assert_eq!(s.current_node, None);
    }

    #[test]
    fn node_states_round_trip_through_save_load() {
        let dir = tempdir().unwrap();
        let mut s = SessionState::new("es-ns".into(), SessionSource::AgentSelf);
        s.node_states.push(crate::exec_session::node::Node {
            id: "n1".into(),
            contract: crate::exec_session::node::NodeContract {
                goal: "test goal".into(),
                verify_commands: vec!["echo ok".into()],
                expected_files: vec![],
            },
            status: crate::exec_session::node::NodeStatus::Verified,
            start_turn_id: "turn-0".into(),
            retry_count: 0,
            verify_log_path: "log.json".into(),
            created_at: "2026-07-20T00:00:00+00:00".into(),
        });
        s.current_node = Some("n1".into());
        s.save(dir.path()).unwrap();

        let loaded = SessionState::load(dir.path()).unwrap();
        assert_eq!(loaded.node_states.len(), 1);
        assert_eq!(loaded.node_states[0].id, "n1");
        assert_eq!(
            loaded.node_states[0].status,
            crate::exec_session::node::NodeStatus::Verified
        );
        assert_eq!(loaded.current_node, Some("n1".into()));
    }
}
