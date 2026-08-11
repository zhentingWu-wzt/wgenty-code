//! Trusted per-agent-session ownership for Work-Graph runtimes.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

use anyhow::{Context, Result};

use crate::agent::SessionId;
use crate::tools::checkpoint_store::CheckpointStore;

use super::{NodeRuntime, ProcessCommandExecutor, SessionCoordinator, SessionSource, VerifyGate};

#[derive(Clone)]
struct RuntimeEntry {
    runtime: Arc<NodeRuntime>,
    coordinator: Arc<RwLock<SessionCoordinator>>,
}

/// Lazily creates one isolated Work-Graph runtime for each trusted agent session.
///
/// Lookup keys are supplied by a trusted [`SessionId`], normally from a
/// [`ToolContext`](crate::agent::ToolContext). The store deliberately has no
/// input path that accepts a model-provided session identifier.
pub struct ExecutionSessionRuntimeStore {
    project_root: PathBuf,
    checkpoint_store: Arc<CheckpointStore>,
    auto_retry_max: u32,
    entries: Mutex<HashMap<SessionId, RuntimeEntry>>,
}

impl ExecutionSessionRuntimeStore {
    /// Builds an empty store whose per-session runtimes use the supplied
    /// project checkpoint store and retry limit.
    pub fn new(
        project_root: PathBuf,
        checkpoint_store: Arc<CheckpointStore>,
        auto_retry_max: u32,
    ) -> Self {
        Self {
            project_root,
            checkpoint_store,
            auto_retry_max,
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// Returns the runtime belonging to `session_id`, creating it once when
    /// the trusted session first uses a Work-Graph tool.
    pub fn runtime_for(&self, session_id: &SessionId) -> Result<Arc<NodeRuntime>> {
        Ok(self.entry_for(session_id)?.runtime)
    }

    /// Ensures that `session_id` owns one active graph turn, then returns its
    /// runtime. Repeated calls leave the existing turn intact.
    pub fn ensure_turn(&self, session_id: &SessionId) -> Result<Arc<NodeRuntime>> {
        let entry = self.entry_for(session_id)?;
        let mut coordinator = entry.coordinator.write().map_err(|error| {
            anyhow::anyhow!("execution-session coordinator write lock: {error}")
        })?;
        if coordinator.current_turn_id().is_none() {
            coordinator
                .begin_turn()
                .context("start initial work-graph turn")?;
        }
        Ok(entry.runtime)
    }

    fn entry_for(&self, session_id: &SessionId) -> Result<RuntimeEntry> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|error| anyhow::anyhow!("execution-session runtime store lock: {error}"))?;
        if let Some(entry) = entries.get(session_id) {
            return Ok(entry.clone());
        }

        let coordinator = Arc::new(RwLock::new(
            SessionCoordinator::new(
                session_id.as_str().to_string(),
                SessionSource::AgentSelf,
                &self.project_root,
                Arc::clone(&self.checkpoint_store),
            )
            .context("create execution session coordinator")?,
        ));
        let gate = Arc::new(VerifyGate::new_with_default_hooks(
            Arc::clone(&coordinator),
            Arc::new(ProcessCommandExecutor),
        ));
        let entry = RuntimeEntry {
            runtime: Arc::new(NodeRuntime::new_with_default_hooks(
                Arc::clone(&coordinator),
                gate,
                self.auto_retry_max,
            )),
            coordinator,
        };
        entries.insert(session_id.clone(), entry.clone());
        Ok(entry)
    }

    #[cfg(test)]
    fn turn_count_for_test(&self, session_id: &SessionId) -> usize {
        self.entries
            .lock()
            .expect("runtime store lock")
            .get(session_id)
            .expect("runtime exists")
            .coordinator
            .read()
            .expect("coordinator lock")
            .session()
            .turns
            .len()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tempfile::TempDir;

    use super::ExecutionSessionRuntimeStore;
    use crate::agent::SessionId;
    use crate::tools::checkpoint_store::CheckpointStore;

    fn test_store(dir: &TempDir) -> ExecutionSessionRuntimeStore {
        ExecutionSessionRuntimeStore::new(
            dir.path().to_path_buf(),
            Arc::new(CheckpointStore::new(dir.path())),
            2,
        )
    }

    #[test]
    fn runtime_store_reuses_one_runtime_per_session_and_isolates_other_sessions() {
        let dir = TempDir::new().expect("create tempdir");
        let store = test_store(&dir);

        let first = store
            .runtime_for(&SessionId::new("session-a"))
            .expect("create first runtime");
        let repeated = store
            .runtime_for(&SessionId::new("session-a"))
            .expect("reuse first runtime");
        let other = store
            .runtime_for(&SessionId::new("session-b"))
            .expect("create isolated runtime");

        assert!(Arc::ptr_eq(&first, &repeated));
        assert!(!Arc::ptr_eq(&first, &other));
    }

    #[test]
    fn ensure_turn_is_idempotent_for_one_session() {
        let dir = TempDir::new().expect("create tempdir");
        let store = test_store(&dir);
        let session_id = SessionId::new("session-a");

        store.ensure_turn(&session_id).expect("start graph turn");
        store
            .ensure_turn(&session_id)
            .expect("reuse active graph turn");

        assert_eq!(store.turn_count_for_test(&session_id), 1);
    }
}
