//! Canonical in-memory hierarchy store with strict local projections.
//!
//! Every agent sees only itself and its direct children. Parent, sibling,
//! grandchild, other-branch, cross-session, and missing targets are uniformly
//! rejected as `StoreError::NotVisible`.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::agent::identity::{AgentId, AgentLifecycleStatus, SessionId};

/// Canonical record for one agent execution in the hierarchy.
#[derive(Debug, Clone)]
pub struct AgentRecord {
    /// Session shared by the root agent and all descendants.
    pub session_id: SessionId,
    /// Identity of this agent execution.
    pub agent_id: AgentId,
    /// Parent agent identity, or `None` for the root agent.
    pub parent_id: Option<AgentId>,
    /// Trusted hierarchy depth, with the root at depth zero.
    pub depth: usize,
    /// Monotonic generation counter for recovery detection.
    pub generation: u64,
    /// Current lifecycle status.
    pub status: AgentLifecycleStatus,
    /// Human-readable task label assigned when this child was spawned.
    pub label: String,
    /// Creation timestamp.
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Last update timestamp.
    pub updated_at: chrono::DateTime<chrono::Utc>,
    /// Optional terminal summary written by the child.
    pub summary: Option<ChildSummary>,
}

impl AgentRecord {
    /// Creates a new record with defaults: generation 0, Pending status,
    /// current timestamps, and no summary.
    pub fn new(
        session_id: SessionId,
        agent_id: AgentId,
        parent_id: Option<AgentId>,
        depth: usize,
    ) -> Self {
        let now = Utc::now();
        Self {
            session_id,
            agent_id,
            parent_id,
            depth,
            generation: 0,
            status: AgentLifecycleStatus::Pending,
            label: String::new(),
            created_at: now,
            updated_at: now,
            summary: None,
        }
    }

    /// Assigns the human-readable task label for this agent record.
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// Sets the authoritative runtime generation captured when this agent is
    /// reserved.
    #[must_use]
    pub fn with_generation(mut self, generation: u64) -> Self {
        self.generation = generation;
        self
    }
}

/// Serializable self-projection visible to the caller.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfView {
    pub agent_id: AgentId,
    pub status: AgentLifecycleStatus,
}

/// Serializable direct-child projection visible to the parent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectChildView {
    pub agent_id: AgentId,
    pub status: AgentLifecycleStatus,
    #[serde(default)]
    pub label: String,
    pub summary: Option<String>,
}

/// Serializable local view: self plus direct children only.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalAgentView {
    pub self_view: SelfView,
    pub children: Vec<DirectChildView>,
}

/// Maximum trusted hierarchy depth included in the directory projection.
///
/// The coordinator's default hierarchy is a single level below the root
/// (`max_depth` default 1), so healthy trees stay far below this cap; it
/// only guards pathological hierarchies by keeping the polling payload
/// bounded. Nodes at the cap stay as leaves; deeper nodes are omitted.
const MAX_DIRECTORY_DEPTH: usize = 5;

/// Recursive directory entry for trusted UI tree projections: hierarchy
/// records only.
///
/// Progress cross-fill (tokens/elapsed/round) happens in the daemon handler
/// layer, mirroring `assemble_local_view`'s separation of concerns; this
/// type intentionally carries no messages or progress metrics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectoryEntry {
    pub agent_id: AgentId,
    pub status: AgentLifecycleStatus,
    /// Human-readable task label assigned when this agent was spawned.
    pub label: String,
    /// Terminal summary text, if the agent wrote one.
    pub summary: Option<String>,
    /// Trusted hierarchy depth of this agent (root at zero).
    pub depth: usize,
    /// Child entries, sorted by agent_id ascending.
    pub children: Vec<DirectoryEntry>,
}

/// Terminal summary written by a child agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChildSummary {
    /// Human-readable summary text.
    pub text: String,
    /// Optional machine-readable error code.
    pub error_code: Option<String>,
}

/// Errors returned by the hierarchy store.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum StoreError {
    /// The target is not visible from the current execution scope.
    #[error("agent is not visible from the current execution scope")]
    NotVisible,
    /// A record with the same (session, agent_id) already exists.
    #[error("agent record already exists")]
    AlreadyExists,
    /// An internal store invariant was violated.
    #[error("agent store invariant failed: {0}")]
    Invariant(String),
}

/// Internal state backing [`InMemoryAgentStore`].
#[derive(Debug, Default)]
struct StoreState {
    /// Unique record keyed by (session_id, agent_id).
    records: HashMap<(SessionId, AgentId), AgentRecord>,
    /// Child index keyed by (session_id, parent_id). Root children use parent `None`.
    children: HashMap<(SessionId, Option<AgentId>), Vec<AgentId>>,
}

/// Canonical in-memory agent hierarchy store.
///
/// All agent-facing read paths must go through `local_view` or
/// `authorize_target`, which enforce the strict local visibility boundary.
/// The `descendants` method is crate-private and reserved for coordinator
/// lifecycle management, never for agent-facing reads.
#[derive(Debug, Clone, Default)]
pub struct InMemoryAgentStore {
    state: Arc<RwLock<StoreState>>,
}

impl InMemoryAgentStore {
    /// Inserts a new agent record. Returns `AlreadyExists` on duplicate.
    pub async fn insert(&self, record: AgentRecord) -> Result<(), StoreError> {
        let mut state = self.state.write().await;
        let key = (record.session_id.clone(), record.agent_id.clone());
        if state.records.contains_key(&key) {
            return Err(StoreError::AlreadyExists);
        }
        let parent_key = (record.session_id.clone(), record.parent_id.clone());
        state
            .children
            .entry(parent_key)
            .or_default()
            .push(record.agent_id.clone());
        state.records.insert(key, record);
        Ok(())
    }

    /// Returns the local view for `caller`: self plus direct children only.
    pub async fn local_view(
        &self,
        session: &SessionId,
        caller: &AgentId,
    ) -> Result<LocalAgentView, StoreError> {
        let state = self.state.read().await;
        let record = state
            .records
            .get(&(session.clone(), caller.clone()))
            .ok_or(StoreError::NotVisible)?;

        let self_view = SelfView {
            agent_id: record.agent_id.clone(),
            status: record.status,
        };

        let child_ids = state
            .children
            .get(&(session.clone(), Some(caller.clone())))
            .cloned()
            .unwrap_or_default();

        let mut children: Vec<DirectChildView> = child_ids
            .iter()
            .filter_map(|cid| state.records.get(&(session.clone(), cid.clone())))
            .map(|r| DirectChildView {
                agent_id: r.agent_id.clone(),
                status: r.status,
                label: r.label.clone(),
                summary: r.summary.as_ref().map(|s| s.text.clone()),
            })
            .collect();

        // Stable order by agent_id for deterministic iteration.
        children.sort_by(|a, b| a.agent_id.as_str().cmp(b.agent_id.as_str()));

        Ok(LocalAgentView {
            self_view,
            children,
        })
    }

    /// Authorizes access to `target` from `caller`.
    ///
    /// Returns the full record only when `target` is the caller itself.
    /// All other targets, including direct children, parent, sibling,
    /// grandchild, absent, and cross-session, return `NotVisible`.
    ///
    /// Direct child visibility is provided by [`local_view`](Self::local_view),
    /// which returns projections without full records.
    pub async fn authorize_target(
        &self,
        session: &SessionId,
        caller: &AgentId,
        target: &AgentId,
    ) -> Result<AgentRecord, StoreError> {
        let state = self.state.read().await;

        // Only self access is authorized; all other targets are NotVisible.
        if target != caller {
            return Err(StoreError::NotVisible);
        }
        state
            .records
            .get(&(session.clone(), caller.clone()))
            .cloned()
            .ok_or(StoreError::NotVisible)
    }

    /// Returns a canonical record to trusted UI projection code.
    ///
    /// This bypasses agent-facing local visibility and must only be called
    /// after the UI viewer's authority for the requested scope is verified.
    pub(crate) async fn record_for_trusted_ui(
        &self,
        session: &SessionId,
        agent: &AgentId,
    ) -> Result<AgentRecord, StoreError> {
        self.state
            .read()
            .await
            .records
            .get(&(session.clone(), agent.clone()))
            .cloned()
            .ok_or(StoreError::NotVisible)
    }

    /// Returns canonical self and direct-child records for trusted UI
    /// projection code in one read transaction.
    ///
    /// This bypasses agent-facing visibility and must only be called for a
    /// root context or after UI navigation authority has been verified.
    pub(crate) async fn local_records_for_trusted_ui(
        &self,
        session: &SessionId,
        agent: &AgentId,
    ) -> Result<(AgentRecord, Vec<AgentRecord>), StoreError> {
        let state = self.state.read().await;
        let self_record = state
            .records
            .get(&(session.clone(), agent.clone()))
            .cloned()
            .ok_or(StoreError::NotVisible)?;
        let child_ids = state
            .children
            .get(&(session.clone(), Some(agent.clone())))
            .cloned()
            .unwrap_or_default();
        let mut children = Vec::with_capacity(child_ids.len());
        for child in child_ids {
            let record = state
                .records
                .get(&(session.clone(), child.clone()))
                .cloned()
                .ok_or_else(|| {
                    StoreError::Invariant(format!(
                        "child index references missing agent: {}",
                        child
                    ))
                })?;
            children.push(record);
        }
        children.sort_by(|a, b| a.agent_id.as_str().cmp(b.agent_id.as_str()));
        Ok((self_record, children))
    }

    /// Returns the whole subtree rooted at `root` within `session` as a
    /// recursive directory projection for trusted UI code, in one read
    /// transaction.
    ///
    /// Hierarchy records only (no progress cross-fill); the daemon handler
    /// layer decorates entries with tokens/elapsed/round data. The descent
    /// is bounded by [`MAX_DIRECTORY_DEPTH`] and guarded by a visited set
    /// against cycles and duplicated edges. Unlike
    /// [`local_records_for_trusted_ui`](Self::local_records_for_trusted_ui),
    /// which fails with [`StoreError::Invariant`] when the child index
    /// references a missing record, this projection skips such children
    /// with a warning (like `local_view`/`direct_children` do): the
    /// directory is a high-frequency polling surface that must stay robust
    /// to index/record races.
    ///
    /// This bypasses agent-facing visibility and must only be called for a
    /// root context or after UI navigation authority has been verified.
    pub(crate) async fn directory_for_trusted_ui(
        &self,
        session: &SessionId,
        root: &AgentId,
    ) -> Result<DirectoryEntry, StoreError> {
        let state = self.state.read().await;
        let root_record = state
            .records
            .get(&(session.clone(), root.clone()))
            .ok_or(StoreError::NotVisible)?;
        let mut visited = HashSet::new();
        visited.insert((session.clone(), root_record.agent_id.clone()));
        Ok(directory_entry_under(
            &state,
            session,
            root_record,
            &mut visited,
        ))
    }

    /// Returns all direct children of `parent` within `session`.
    pub async fn direct_children(
        &self,
        session: &SessionId,
        parent: &AgentId,
    ) -> Result<Vec<AgentRecord>, StoreError> {
        let state = self.state.read().await;
        let child_ids = state
            .children
            .get(&(session.clone(), Some(parent.clone())))
            .cloned()
            .unwrap_or_default();

        let mut children: Vec<AgentRecord> = child_ids
            .iter()
            .filter_map(|cid| state.records.get(&(session.clone(), cid.clone())))
            .cloned()
            .collect();

        // Stable order by agent_id for deterministic iteration.
        children.sort_by(|a, b| a.agent_id.as_str().cmp(b.agent_id.as_str()));
        Ok(children)
    }

    /// Returns all descendants of `root` within `session` (exclusive of root).
    ///
    /// This is crate-private and reserved for coordinator lifecycle
    /// management (recovery, cancellation). It must never be exposed to
    /// agent-facing read paths.
    pub(crate) async fn descendants(
        &self,
        session: &SessionId,
        root: &AgentId,
    ) -> Result<Vec<AgentRecord>, StoreError> {
        let state = self.state.read().await;
        let mut result = Vec::new();
        let mut queue: Vec<AgentId> = state
            .children
            .get(&(session.clone(), Some(root.clone())))
            .cloned()
            .unwrap_or_default();

        while let Some(current) = queue.pop() {
            if let Some(record) = state.records.get(&(session.clone(), current.clone())) {
                result.push(record.clone());
                if let Some(grandchildren) = state
                    .children
                    .get(&(session.clone(), Some(current.clone())))
                {
                    queue.extend(grandchildren.iter().cloned());
                }
            }
        }

        result.sort_by(|a, b| a.agent_id.as_str().cmp(b.agent_id.as_str()));
        Ok(result)
    }

    /// Updates the lifecycle status of an agent.
    pub(crate) async fn update_status(
        &self,
        session: &SessionId,
        agent: &AgentId,
        status: AgentLifecycleStatus,
    ) -> Result<(), StoreError> {
        let mut state = self.state.write().await;
        let record = state
            .records
            .get_mut(&(session.clone(), agent.clone()))
            .ok_or_else(|| StoreError::Invariant(format!("agent not found: {}", agent)))?;
        record.status = status;
        record.updated_at = Utc::now();
        Ok(())
    }

    /// Sets the terminal summary for an agent.
    pub(crate) async fn set_summary(
        &self,
        session: &SessionId,
        agent: &AgentId,
        summary: ChildSummary,
    ) -> Result<(), StoreError> {
        let mut state = self.state.write().await;
        let record = state
            .records
            .get_mut(&(session.clone(), agent.clone()))
            .ok_or_else(|| StoreError::Invariant(format!("agent not found: {}", agent)))?;
        record.summary = Some(summary);
        record.updated_at = Utc::now();
        Ok(())
    }
}

/// Builds one directory entry and recurses into its children while the
/// caller holds the store read lock.
///
/// Robustness rules (deliberate deviation from the strict
/// `local_records_for_trusted_ui` invariant): child ids that were already
/// visited (cycle/duplicated-edge guard), child ids without a record
/// (index/record race), and children deeper than [`MAX_DIRECTORY_DEPTH`]
/// are skipped instead of failing the whole projection.
fn directory_entry_under(
    state: &StoreState,
    session: &SessionId,
    record: &AgentRecord,
    visited: &mut HashSet<(SessionId, AgentId)>,
) -> DirectoryEntry {
    // Nodes at the cap stay as leaves: their children lists are empty, so
    // deeper nodes are omitted from the projection.
    let children = if record.depth >= MAX_DIRECTORY_DEPTH {
        Vec::new()
    } else {
        let child_ids = state
            .children
            .get(&(session.clone(), Some(record.agent_id.clone())))
            .cloned()
            .unwrap_or_default();
        let mut children = Vec::with_capacity(child_ids.len());
        for child_id in child_ids {
            let key = (session.clone(), child_id);
            if visited.contains(&key) {
                continue;
            }
            let Some(child) = state.records.get(&key) else {
                // Index/record race: skip defensively instead of failing the
                // whole tree. tracing::warn! is inert without a subscriber.
                tracing::warn!(
                    session_id = %session,
                    agent_id = %key.1,
                    "directory child index references missing agent; skipping"
                );
                continue;
            };
            if child.depth > MAX_DIRECTORY_DEPTH {
                continue;
            }
            visited.insert(key);
            children.push(directory_entry_under(state, session, child, visited));
        }
        // Same deterministic order as local_records_for_trusted_ui.
        children.sort_by(|a, b| a.agent_id.as_str().cmp(b.agent_id.as_str()));
        children
    };

    DirectoryEntry {
        agent_id: record.agent_id.clone(),
        status: record.status,
        label: record.label.clone(),
        summary: record.summary.as_ref().map(|s| s.text.clone()),
        depth: record.depth,
        children,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(session: &str, id: &str, parent: Option<&str>, depth: usize) -> AgentRecord {
        AgentRecord::new(
            SessionId::new(session),
            AgentId::new(id),
            parent.map(AgentId::new),
            depth,
        )
    }

    async fn seeded_store() -> InMemoryAgentStore {
        let store = InMemoryAgentStore::default();
        store.insert(record("s", "root", None, 0)).await.unwrap();
        store
            .insert(record("s", "child", Some("root"), 1))
            .await
            .unwrap();
        store
            .insert(record("s", "grandchild", Some("child"), 2))
            .await
            .unwrap();
        store
            .insert(record("s", "sibling", Some("root"), 1))
            .await
            .unwrap();
        store
            .insert(record("s", "other-branch", Some("root"), 1))
            .await
            .unwrap();
        store
            .insert(record("other", "other-root", None, 0))
            .await
            .unwrap();
        store
    }

    #[tokio::test]
    async fn local_view_contains_only_self_and_direct_children() {
        let store = InMemoryAgentStore::default();
        store.insert(record("s", "root", None, 0)).await.unwrap();
        store
            .insert(record("s", "a", Some("root"), 1))
            .await
            .unwrap();
        store
            .insert(record("s", "b", Some("root"), 1))
            .await
            .unwrap();
        store.insert(record("s", "a1", Some("a"), 2)).await.unwrap();

        let view = store
            .local_view(&SessionId::new("s"), &AgentId::new("a"))
            .await
            .unwrap();
        assert_eq!(view.self_view.agent_id.as_str(), "a");
        assert_eq!(
            view.children
                .iter()
                .map(|c| c.agent_id.as_str())
                .collect::<Vec<_>>(),
            vec!["a1"]
        );
    }

    #[tokio::test]
    async fn hidden_and_missing_targets_share_not_visible() {
        let store = seeded_store().await;
        for target in ["root", "sibling", "grandchild", "other-branch", "missing"] {
            assert_eq!(
                store
                    .authorize_target(
                        &SessionId::new("s"),
                        &AgentId::new("child"),
                        &AgentId::new(target),
                    )
                    .await
                    .err(),
                Some(StoreError::NotVisible),
            );
        }
        assert_eq!(
            store
                .authorize_target(
                    &SessionId::new("other"),
                    &AgentId::new("child"),
                    &AgentId::new("child"),
                )
                .await
                .err(),
            Some(StoreError::NotVisible),
        );
    }

    #[tokio::test]
    async fn self_access_is_visible() {
        let store = seeded_store().await;

        let self_rec = store
            .authorize_target(
                &SessionId::new("s"),
                &AgentId::new("child"),
                &AgentId::new("child"),
            )
            .await
            .unwrap();
        assert_eq!(self_rec.agent_id.as_str(), "child");
    }

    #[tokio::test]
    async fn root_sees_only_direct_children() {
        let store = seeded_store().await;
        let view = store
            .local_view(&SessionId::new("s"), &AgentId::new("root"))
            .await
            .unwrap();
        assert_eq!(view.self_view.agent_id.as_str(), "root");
        let child_ids: Vec<&str> = view.children.iter().map(|c| c.agent_id.as_str()).collect();
        // Direct children: child, sibling, other-branch (sorted by agent_id).
        assert_eq!(child_ids, vec!["child", "other-branch", "sibling"]);
        // Grandchild must not appear.
        assert!(!child_ids.contains(&"grandchild"));
    }

    #[tokio::test]
    async fn duplicate_insert_returns_already_exists() {
        let store = InMemoryAgentStore::default();
        store.insert(record("s", "root", None, 0)).await.unwrap();
        assert_eq!(
            store.insert(record("s", "root", None, 0)).await,
            Err(StoreError::AlreadyExists)
        );
    }

    #[tokio::test]
    async fn missing_agent_local_view_returns_not_visible() {
        let store = seeded_store().await;
        assert_eq!(
            store
                .local_view(&SessionId::new("s"), &AgentId::new("missing"))
                .await
                .err(),
            Some(StoreError::NotVisible)
        );
    }

    #[tokio::test]
    async fn descendants_returns_all_below_root_exclusive() {
        let store = seeded_store().await;
        let desc = store
            .descendants(&SessionId::new("s"), &AgentId::new("root"))
            .await
            .unwrap();
        let ids: Vec<&str> = desc.iter().map(|r| r.agent_id.as_str()).collect();
        assert!(ids.contains(&"child"));
        assert!(ids.contains(&"grandchild"));
        assert!(ids.contains(&"sibling"));
        assert!(ids.contains(&"other-branch"));
        assert!(!ids.contains(&"root"));
    }

    #[tokio::test]
    async fn update_status_changes_record() {
        let store = seeded_store().await;
        store
            .update_status(
                &SessionId::new("s"),
                &AgentId::new("child"),
                AgentLifecycleStatus::Running,
            )
            .await
            .unwrap();
        let view = store
            .local_view(&SessionId::new("s"), &AgentId::new("child"))
            .await
            .unwrap();
        assert_eq!(view.self_view.status, AgentLifecycleStatus::Running);
    }

    #[tokio::test]
    async fn directory_missing_root_returns_not_visible() {
        let store = seeded_store().await;
        // Empty session: no records at all.
        assert_eq!(
            store
                .directory_for_trusted_ui(&SessionId::new("empty"), &AgentId::new("root"))
                .await
                .err(),
            Some(StoreError::NotVisible)
        );
        // Known session, unknown agent.
        assert_eq!(
            store
                .directory_for_trusted_ui(&SessionId::new("s"), &AgentId::new("missing"))
                .await
                .err(),
            Some(StoreError::NotVisible)
        );
        // Cross-session root is equally invisible.
        assert_eq!(
            store
                .directory_for_trusted_ui(&SessionId::new("other"), &AgentId::new("root"))
                .await
                .err(),
            Some(StoreError::NotVisible)
        );
    }

    #[tokio::test]
    async fn directory_projects_nested_subtree_sorted_by_agent_id() {
        let store = InMemoryAgentStore::default();
        store.insert(record("s", "root", None, 0)).await.unwrap();
        // Inserted out of order to prove output sorting.
        store
            .insert(record("s", "beta", Some("root"), 1).with_label("Beta task"))
            .await
            .unwrap();
        store
            .insert(record("s", "alpha", Some("root"), 1))
            .await
            .unwrap();
        store
            .insert(record("s", "alpha-child", Some("alpha"), 2))
            .await
            .unwrap();

        let root = store
            .directory_for_trusted_ui(&SessionId::new("s"), &AgentId::new("root"))
            .await
            .unwrap();
        assert_eq!(root.agent_id.as_str(), "root");
        assert_eq!(root.depth, 0);
        assert!(root.summary.is_none());

        let child_ids: Vec<&str> = root.children.iter().map(|c| c.agent_id.as_str()).collect();
        assert_eq!(child_ids, vec!["alpha", "beta"]);

        let alpha = &root.children[0];
        assert_eq!(alpha.depth, 1);
        assert_eq!(alpha.label, "");
        let grandchild_ids: Vec<&str> =
            alpha.children.iter().map(|c| c.agent_id.as_str()).collect();
        assert_eq!(grandchild_ids, vec!["alpha-child"]);
        assert_eq!(alpha.children[0].depth, 2);

        let beta = &root.children[1];
        assert_eq!(beta.label, "Beta task");
        assert!(beta.children.is_empty());
    }

    #[tokio::test]
    async fn directory_truncates_children_at_max_depth() {
        let store = InMemoryAgentStore::default();
        // Chain deeper than MAX_DIRECTORY_DEPTH: root(0) -> d1 -> ... -> d6.
        let mut ids = vec!["root".to_string()];
        store.insert(record("s", "root", None, 0)).await.unwrap();
        for depth in 1..=6usize {
            let id = format!("d{depth}");
            let parent = ids[depth - 1].clone();
            store
                .insert(record("s", &id, Some(&parent), depth))
                .await
                .unwrap();
            ids.push(id);
        }

        let entry = store
            .directory_for_trusted_ui(&SessionId::new("s"), &AgentId::new("root"))
            .await
            .unwrap();
        let mut node = &entry;
        for depth in 0..=MAX_DIRECTORY_DEPTH {
            assert_eq!(node.depth, depth, "entry at depth {depth} must exist");
            if depth < MAX_DIRECTORY_DEPTH {
                assert_eq!(node.children.len(), 1);
                node = &node.children[0];
            }
        }
        // The cap node is kept as a leaf; d6 (depth 6) is omitted entirely.
        assert_eq!(node.agent_id.as_str(), "d5");
        assert!(node.children.is_empty());
    }

    #[tokio::test]
    async fn directory_keeps_completed_children_for_history() {
        let store = InMemoryAgentStore::default();
        store.insert(record("s", "root", None, 0)).await.unwrap();
        store
            .insert(record("s", "child", Some("root"), 1))
            .await
            .unwrap();
        store
            .update_status(
                &SessionId::new("s"),
                &AgentId::new("child"),
                AgentLifecycleStatus::Completed,
            )
            .await
            .unwrap();
        store
            .set_summary(
                &SessionId::new("s"),
                &AgentId::new("child"),
                ChildSummary {
                    text: "all done".to_string(),
                    error_code: None,
                },
            )
            .await
            .unwrap();

        let root = store
            .directory_for_trusted_ui(&SessionId::new("s"), &AgentId::new("root"))
            .await
            .unwrap();
        assert_eq!(root.children.len(), 1);
        let child = &root.children[0];
        assert_eq!(child.agent_id.as_str(), "child");
        assert_eq!(child.status, AgentLifecycleStatus::Completed);
        assert_eq!(child.summary.as_deref(), Some("all done"));
    }

    // The remaining directory tests corrupt the private `children` index
    // directly; same-module test code can reach store internals, which is
    // the only way to reproduce index/record races the projection must
    // tolerate.
    #[tokio::test]
    async fn directory_skips_dangling_child_index_entries() {
        let store = InMemoryAgentStore::default();
        store.insert(record("s", "root", None, 0)).await.unwrap();
        store
            .insert(record("s", "child", Some("root"), 1))
            .await
            .unwrap();
        {
            let mut state = store.state.write().await;
            state.children.insert(
                (SessionId::new("s"), Some(AgentId::new("root"))),
                vec![AgentId::new("ghost"), AgentId::new("child")],
            );
        }

        let root = store
            .directory_for_trusted_ui(&SessionId::new("s"), &AgentId::new("root"))
            .await
            .unwrap();
        let child_ids: Vec<&str> = root.children.iter().map(|c| c.agent_id.as_str()).collect();
        // The dangling "ghost" id is skipped instead of erroring.
        assert_eq!(child_ids, vec!["child"]);
    }

    #[tokio::test]
    async fn directory_prunes_cycle_back_edges() {
        let store = InMemoryAgentStore::default();
        store.insert(record("s", "root", None, 0)).await.unwrap();
        store
            .insert(record("s", "a", Some("root"), 1))
            .await
            .unwrap();
        store
            .insert(record("s", "b", Some("root"), 1))
            .await
            .unwrap();
        {
            // Forge a cycle a -> b -> a via the child index.
            let mut state = store.state.write().await;
            state.children.insert(
                (SessionId::new("s"), Some(AgentId::new("a"))),
                vec![AgentId::new("b")],
            );
            state.children.insert(
                (SessionId::new("s"), Some(AgentId::new("b"))),
                vec![AgentId::new("a")],
            );
        }

        let root = store
            .directory_for_trusted_ui(&SessionId::new("s"), &AgentId::new("root"))
            .await
            .unwrap();
        // Each agent id is emitted at most once: "b" was first visited
        // while descending from "a", so the duplicate root->b edge is
        // pruned, and the b->a back-edge terminates the descent.
        let child_ids: Vec<&str> = root.children.iter().map(|c| c.agent_id.as_str()).collect();
        assert_eq!(child_ids, vec!["a"]);
        let a = &root.children[0];
        let grandchild_ids: Vec<&str> = a.children.iter().map(|c| c.agent_id.as_str()).collect();
        assert_eq!(grandchild_ids, vec!["b"]);
        assert!(a.children[0].children.is_empty());
    }
}
