use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::RwLock;
use tokio::time::Instant;

use super::{AgentId, ChildResult, ChildTerminalStatus, SessionId};

/// Identifies one batch of direct-child work and serializes as a plain string.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TaskGroupId(String);

impl TaskGroupId {
    /// Returns the identifier's string wire representation.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A terminal direct-child result batch claimed for exactly-once delivery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskGroupDelivery {
    /// Identity of the claimed group.
    pub group_id: TaskGroupId,
    /// Session generation in which the group was created.
    pub generation: u64,
    /// Direct-child results ordered lexicographically by child ID.
    pub results: Vec<ChildResult>,
}

/// Errors produced while mutating task-group membership and results.
#[derive(Debug, Error)]
pub enum TaskGroupError {
    /// The caller attempted to create or mutate state from an obsolete generation.
    #[error(
        "session `{session_id}` is at generation {expected}, but generation {actual} was supplied"
    )]
    StaleGeneration {
        session_id: SessionId,
        expected: u64,
        actual: u64,
    },
    /// The requested group is not present in this store.
    #[error("task group `{group_id}` does not exist")]
    GroupNotFound { group_id: String },
    /// Membership cannot change after cancellation or delivery.
    #[error("task group `{group_id}` is closed and cannot accept updates for child `{child_id}`")]
    GroupClosed { group_id: String, child_id: String },
    /// A direct child may be added to a group only once.
    #[error("child `{child_id}` is already registered in task group `{group_id}`")]
    ChildAlreadyRegistered { group_id: String, child_id: String },
    /// Results are accepted only from children directly registered in the group.
    #[error("child `{child_id}` is not registered in task group `{group_id}`")]
    ChildNotRegistered { group_id: String, child_id: String },
    /// A direct child may publish only one terminal result.
    #[error("child `{child_id}` already has a terminal result in task group `{group_id}`")]
    ResultAlreadyRecorded { group_id: String, child_id: String },
}

/// In-memory generation and delivery state for direct-child task groups.
#[derive(Default)]
pub struct TaskGroupStore {
    inner: RwLock<TaskGroupState>,
}

#[derive(Default)]
struct TaskGroupState {
    generations: HashMap<SessionId, u64>,
    groups: HashMap<TaskGroupId, GroupRecord>,
}

struct GroupRecord {
    id: TaskGroupId,
    session_id: SessionId,
    owner_id: AgentId,
    origin_turn_id: Option<String>,
    generation: u64,
    deadline_at: Instant,
    child_ids: HashSet<AgentId>,
    results: HashMap<AgentId, ChildResult>,
    lifecycle: GroupLifecycle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GroupLifecycle {
    Active,
    Expired,
}

impl TaskGroupStore {
    /// Creates or reuses the unclaimed group for one root owner and turn.
    pub async fn create_for_root_turn(
        &self,
        session_id: SessionId,
        owner_id: AgentId,
        origin_turn_id: impl Into<String>,
        generation: u64,
        deadline_at: Instant,
    ) -> Result<TaskGroupId, TaskGroupError> {
        let origin_turn_id = origin_turn_id.into();
        let mut state = self.inner.write().await;
        Self::initialize_or_validate_generation(&mut state, &session_id, generation)?;

        if let Some(group) = state.groups.values().find(|group| {
            group.session_id == session_id
                && group.owner_id == owner_id
                && group.origin_turn_id.as_deref() == Some(origin_turn_id.as_str())
                && group.generation == generation
        }) {
            return Ok(group.id.clone());
        }

        Ok(Self::insert_group(
            &mut state,
            session_id,
            owner_id,
            Some(origin_turn_id),
            generation,
            deadline_at,
        ))
    }

    /// Creates or reuses the unclaimed direct-child group for one parent.
    pub async fn create_for_parent(
        &self,
        session_id: SessionId,
        owner_id: AgentId,
        generation: u64,
        deadline_at: Instant,
    ) -> Result<TaskGroupId, TaskGroupError> {
        let mut state = self.inner.write().await;
        Self::initialize_or_validate_generation(&mut state, &session_id, generation)?;

        if let Some(group) = state.groups.values().find(|group| {
            group.session_id == session_id
                && group.owner_id == owner_id
                && group.origin_turn_id.is_none()
                && group.generation == generation
        }) {
            return Ok(group.id.clone());
        }

        Ok(Self::insert_group(
            &mut state,
            session_id,
            owner_id,
            None,
            generation,
            deadline_at,
        ))
    }

    /// Registers one direct child as part of a group.
    pub async fn add_child(
        &self,
        group_id: &TaskGroupId,
        child_id: AgentId,
    ) -> Result<(), TaskGroupError> {
        let mut state = self.inner.write().await;
        Self::ensure_group_current(&state, group_id)?;
        let group = Self::group_mut(&mut state, group_id)?;
        if group.lifecycle != GroupLifecycle::Active {
            return Err(TaskGroupError::GroupClosed {
                group_id: group_id.as_str().to_owned(),
                child_id: child_id.as_str().to_owned(),
            });
        }
        if !group.child_ids.insert(child_id.clone()) {
            return Err(TaskGroupError::ChildAlreadyRegistered {
                group_id: group_id.as_str().to_owned(),
                child_id: child_id.as_str().to_owned(),
            });
        }
        Ok(())
    }

    /// Records the terminal result of one directly registered child.
    pub async fn record_result(
        &self,
        group_id: &TaskGroupId,
        result: ChildResult,
    ) -> Result<(), TaskGroupError> {
        let mut state = self.inner.write().await;
        Self::ensure_group_current(&state, group_id)?;
        let group = Self::group_mut(&mut state, group_id)?;
        if group.lifecycle != GroupLifecycle::Active {
            return Err(TaskGroupError::GroupClosed {
                group_id: group_id.as_str().to_owned(),
                child_id: result.child_id.as_str().to_owned(),
            });
        }
        if !group.child_ids.contains(&result.child_id) {
            return Err(TaskGroupError::ChildNotRegistered {
                group_id: group_id.as_str().to_owned(),
                child_id: result.child_id.as_str().to_owned(),
            });
        }
        if group.results.contains_key(&result.child_id) {
            return Err(TaskGroupError::ResultAlreadyRecorded {
                group_id: group_id.as_str().to_owned(),
                child_id: result.child_id.as_str().to_owned(),
            });
        }
        group.results.insert(result.child_id.clone(), result);
        Ok(())
    }

    /// Converts every unfinished child in an expired active group to a timeout.
    pub async fn expire_due_groups(&self, now: Instant) -> Result<usize, TaskGroupError> {
        let mut state = self.inner.write().await;
        let mut expired_children = 0;

        for group in state
            .groups
            .values_mut()
            .filter(|group| group.lifecycle == GroupLifecycle::Active && group.deadline_at <= now)
        {
            let unfinished: Vec<_> = group
                .child_ids
                .iter()
                .filter(|child_id| !group.results.contains_key(*child_id))
                .cloned()
                .collect();
            expired_children += unfinished.len();
            for child_id in unfinished {
                group.results.insert(
                    child_id.clone(),
                    ChildResult {
                        child_id,
                        status: ChildTerminalStatus::Failed,
                        summary: String::new(),
                        error_code: Some("timeout".to_owned()),
                        partial_result: None,
                    },
                );
            }
            group.lifecycle = GroupLifecycle::Expired;
        }
        state.groups.retain(|_, group| {
            group.lifecycle != GroupLifecycle::Expired || !group.child_ids.is_empty()
        });

        Ok(expired_children)
    }

    /// Atomically claims one ready group in the requested current generation.
    pub async fn claim_ready(
        &self,
        session_id: &SessionId,
        generation: u64,
    ) -> Result<Option<TaskGroupDelivery>, TaskGroupError> {
        let mut state = self.inner.write().await;
        if state
            .generations
            .get(session_id)
            .copied()
            .unwrap_or_default()
            != generation
        {
            return Ok(None);
        }

        let group_id = state
            .groups
            .values()
            .filter(|group| {
                group.session_id == *session_id
                    && group.generation == generation
                    && Self::is_ready(group)
            })
            .map(|group| group.id.clone())
            .min_by(|left, right| left.as_str().cmp(right.as_str()));
        let Some(group_id) = group_id else {
            return Ok(None);
        };

        Self::remove_delivery(&mut state, &group_id).map(Some)
    }

    /// Atomically claims one ready group owned directly by `owner_id`.
    pub async fn claim_ready_for_owner(
        &self,
        session_id: &SessionId,
        owner_id: &AgentId,
        generation: u64,
    ) -> Result<Option<TaskGroupDelivery>, TaskGroupError> {
        let mut state = self.inner.write().await;
        if state
            .generations
            .get(session_id)
            .copied()
            .unwrap_or_default()
            != generation
        {
            return Ok(None);
        }

        let group_id = state
            .groups
            .values()
            .filter(|group| {
                group.session_id == *session_id
                    && group.owner_id == *owner_id
                    && group.generation == generation
                    && Self::is_ready(group)
            })
            .map(|group| group.id.clone())
            .min_by(|left, right| left.as_str().cmp(right.as_str()));
        let Some(group_id) = group_id else {
            return Ok(None);
        };

        Self::remove_delivery(&mut state, &group_id).map(Some)
    }

    /// Atomically claims one exact ready group when its ownership matches.
    pub async fn claim_specific(
        &self,
        group_id: &TaskGroupId,
        session_id: &SessionId,
        owner_id: &AgentId,
        generation: u64,
    ) -> Result<Option<TaskGroupDelivery>, TaskGroupError> {
        let mut state = self.inner.write().await;
        let matches = state.groups.get(group_id).is_some_and(|group| {
            group.session_id == *session_id
                && group.owner_id == *owner_id
                && group.generation == generation
                && state
                    .generations
                    .get(session_id)
                    .copied()
                    .unwrap_or_default()
                    == generation
                && Self::is_ready(group)
        });
        if !matches {
            return Ok(None);
        }

        Self::remove_delivery(&mut state, group_id).map(Some)
    }

    fn is_ready(group: &GroupRecord) -> bool {
        !group.child_ids.is_empty() && group.child_ids.len() == group.results.len()
    }

    fn remove_delivery(
        state: &mut TaskGroupState,
        group_id: &TaskGroupId,
    ) -> Result<TaskGroupDelivery, TaskGroupError> {
        let group = state
            .groups
            .remove(group_id)
            .ok_or_else(|| TaskGroupError::GroupNotFound {
                group_id: group_id.as_str().to_owned(),
            })?;
        let mut results: Vec<_> = group.results.values().cloned().collect();
        results.sort_by(|left, right| left.child_id.as_str().cmp(right.child_id.as_str()));

        Ok(TaskGroupDelivery {
            group_id: group.id,
            generation: group.generation,
            results,
        })
    }

    /// Returns the current generation for a session, defaulting to zero.
    pub async fn current_generation(&self, session_id: &SessionId) -> u64 {
        self.inner
            .read()
            .await
            .generations
            .get(session_id)
            .copied()
            .unwrap_or_default()
    }

    /// Advances a session generation and removes every older unclaimed group.
    pub async fn advance_generation(&self, session_id: &SessionId) -> u64 {
        let mut state = self.inner.write().await;
        let generation = {
            let generation = state.generations.entry(session_id.clone()).or_default();
            *generation += 1;
            *generation
        };
        state
            .groups
            .retain(|_, group| group.session_id != *session_id || group.generation >= generation);
        generation
    }

    /// Cancels and removes all groups for exactly one session generation.
    pub async fn cancel_generation(&self, session_id: &SessionId, generation: u64) -> usize {
        let mut state = self.inner.write().await;
        let original_len = state.groups.len();
        state
            .groups
            .retain(|_, group| group.session_id != *session_id || group.generation != generation);
        original_len - state.groups.len()
    }

    fn insert_group(
        state: &mut TaskGroupState,
        session_id: SessionId,
        owner_id: AgentId,
        origin_turn_id: Option<String>,
        generation: u64,
        deadline_at: Instant,
    ) -> TaskGroupId {
        let id = TaskGroupId(uuid::Uuid::new_v4().to_string());
        state.groups.insert(
            id.clone(),
            GroupRecord {
                id: id.clone(),
                session_id,
                owner_id,
                origin_turn_id,
                generation,
                deadline_at,
                child_ids: HashSet::new(),
                results: HashMap::new(),
                lifecycle: GroupLifecycle::Active,
            },
        );
        id
    }

    fn ensure_generation(
        state: &TaskGroupState,
        session_id: &SessionId,
        generation: u64,
    ) -> Result<(), TaskGroupError> {
        let current = state
            .generations
            .get(session_id)
            .copied()
            .unwrap_or_default();
        if current != generation {
            return Err(TaskGroupError::StaleGeneration {
                session_id: session_id.clone(),
                expected: current,
                actual: generation,
            });
        }
        Ok(())
    }

    fn initialize_or_validate_generation(
        state: &mut TaskGroupState,
        session_id: &SessionId,
        generation: u64,
    ) -> Result<(), TaskGroupError> {
        match state.generations.get(session_id).copied() {
            Some(current) if current != generation => Err(TaskGroupError::StaleGeneration {
                session_id: session_id.clone(),
                expected: current,
                actual: generation,
            }),
            Some(_) => Ok(()),
            None => {
                state.generations.insert(session_id.clone(), generation);
                Ok(())
            }
        }
    }

    fn ensure_group_current(
        state: &TaskGroupState,
        group_id: &TaskGroupId,
    ) -> Result<(), TaskGroupError> {
        let group = state
            .groups
            .get(group_id)
            .ok_or_else(|| TaskGroupError::GroupNotFound {
                group_id: group_id.as_str().to_owned(),
            })?;
        Self::ensure_generation(state, &group.session_id, group.generation)
    }

    fn group_mut<'a>(
        state: &'a mut TaskGroupState,
        group_id: &TaskGroupId,
    ) -> Result<&'a mut GroupRecord, TaskGroupError> {
        state
            .groups
            .get_mut(group_id)
            .ok_or_else(|| TaskGroupError::GroupNotFound {
                group_id: group_id.as_str().to_owned(),
            })
    }

    #[cfg(test)]
    async fn group_count(&self) -> usize {
        self.inner.read().await.groups.len()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use super::{TaskGroupError, TaskGroupId, TaskGroupStore};
    use crate::agent::{AgentId, ChildResult, ChildTerminalStatus, SessionId};
    use tokio::sync::Barrier;

    fn result(child_id: &str, status: ChildTerminalStatus) -> ChildResult {
        ChildResult {
            child_id: AgentId::new(child_id),
            status,
            summary: format!("result from {child_id}"),
            error_code: None,
            partial_result: None,
        }
    }

    async fn create_root_group(
        store: &TaskGroupStore,
        turn_id: &str,
        generation: u64,
        deadline_at: tokio::time::Instant,
    ) -> TaskGroupId {
        store
            .create_for_root_turn(
                SessionId::new("s"),
                AgentId::new("root"),
                turn_id,
                generation,
                deadline_at,
            )
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn group_becomes_ready_only_after_every_direct_child_is_terminal() {
        let store = TaskGroupStore::default();
        let group = create_root_group(
            &store,
            "turn-1",
            0,
            tokio::time::Instant::now() + Duration::from_secs(30),
        )
        .await;
        store.add_child(&group, AgentId::new("a")).await.unwrap();
        store.add_child(&group, AgentId::new("b")).await.unwrap();

        store
            .record_result(&group, result("a", ChildTerminalStatus::Completed))
            .await
            .unwrap();
        assert!(store
            .claim_ready(&SessionId::new("s"), 0)
            .await
            .unwrap()
            .is_none());

        store
            .record_result(&group, result("b", ChildTerminalStatus::Failed))
            .await
            .unwrap();
        let delivery = store
            .claim_ready(&SessionId::new("s"), 0)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(delivery.results.len(), 2);
    }

    #[tokio::test]
    async fn fresh_session_can_start_and_deliver_at_a_nonzero_generation() {
        let store = TaskGroupStore::default();
        let group = create_root_group(
            &store,
            "turn-1",
            3,
            tokio::time::Instant::now() + Duration::from_secs(30),
        )
        .await;
        store.add_child(&group, AgentId::new("a")).await.unwrap();
        store
            .record_result(&group, result("a", ChildTerminalStatus::Completed))
            .await
            .unwrap();

        assert_eq!(store.current_generation(&SessionId::new("s")).await, 3);
        let delivery = store
            .claim_ready(&SessionId::new("s"), 3)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(delivery.generation, 3);
        assert_eq!(delivery.results.len(), 1);
    }

    #[tokio::test]
    async fn ready_group_can_be_claimed_exactly_once() {
        let store = TaskGroupStore::default();
        let group = create_root_group(
            &store,
            "turn-1",
            0,
            tokio::time::Instant::now() + Duration::from_secs(30),
        )
        .await;
        store.add_child(&group, AgentId::new("a")).await.unwrap();
        store
            .record_result(&group, result("a", ChildTerminalStatus::Completed))
            .await
            .unwrap();

        assert!(store
            .claim_ready(&SessionId::new("s"), 0)
            .await
            .unwrap()
            .is_some());
        assert!(store
            .claim_ready(&SessionId::new("s"), 0)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn stale_generation_is_not_deliverable_after_advance() {
        let store = TaskGroupStore::default();
        let group = create_root_group(
            &store,
            "turn-1",
            0,
            tokio::time::Instant::now() + Duration::from_secs(30),
        )
        .await;
        store.add_child(&group, AgentId::new("a")).await.unwrap();
        store
            .record_result(&group, result("a", ChildTerminalStatus::Completed))
            .await
            .unwrap();

        assert_eq!(store.advance_generation(&SessionId::new("s")).await, 1);
        assert!(store
            .claim_ready(&SessionId::new("s"), 0)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn current_generation_defaults_to_zero_and_tracks_advances() {
        let store = TaskGroupStore::default();
        let session_id = SessionId::new("new-session");

        assert_eq!(store.current_generation(&session_id).await, 0);
        assert_eq!(store.advance_generation(&session_id).await, 1);
        assert_eq!(store.current_generation(&session_id).await, 1);
    }

    #[tokio::test]
    async fn cancelling_a_generation_suppresses_its_ready_groups() {
        let store = TaskGroupStore::default();
        let group = create_root_group(
            &store,
            "turn-1",
            0,
            tokio::time::Instant::now() + Duration::from_secs(30),
        )
        .await;
        store.add_child(&group, AgentId::new("a")).await.unwrap();
        store
            .record_result(&group, result("a", ChildTerminalStatus::Completed))
            .await
            .unwrap();

        assert_eq!(store.cancel_generation(&SessionId::new("s"), 0).await, 1);
        assert!(store
            .claim_ready(&SessionId::new("s"), 0)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn deadline_marks_unfinished_children_as_timeout_failures() {
        let store = TaskGroupStore::default();
        let group = create_root_group(
            &store,
            "turn-1",
            0,
            tokio::time::Instant::now() - Duration::from_secs(1),
        )
        .await;
        store.add_child(&group, AgentId::new("a")).await.unwrap();

        store
            .expire_due_groups(tokio::time::Instant::now())
            .await
            .unwrap();

        let delivery = store
            .claim_ready(&SessionId::new("s"), 0)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(delivery.results[0].status, ChildTerminalStatus::Failed);
        assert_eq!(delivery.results[0].error_code.as_deref(), Some("timeout"));
        assert!(delivery.results[0].summary.is_empty());
        assert!(delivery.results[0].partial_result.is_none());
    }

    #[tokio::test]
    async fn results_are_ordered_deterministically_by_child_id() {
        let store = TaskGroupStore::default();
        let group = create_root_group(
            &store,
            "turn-1",
            0,
            tokio::time::Instant::now() + Duration::from_secs(30),
        )
        .await;
        for child_id in ["charlie", "alpha", "bravo"] {
            store
                .add_child(&group, AgentId::new(child_id))
                .await
                .unwrap();
        }
        for child_id in ["bravo", "charlie", "alpha"] {
            store
                .record_result(&group, result(child_id, ChildTerminalStatus::Completed))
                .await
                .unwrap();
        }

        let delivery = store
            .claim_ready(&SessionId::new("s"), 0)
            .await
            .unwrap()
            .unwrap();
        let child_ids: Vec<_> = delivery
            .results
            .iter()
            .map(|result| result.child_id.as_str())
            .collect();
        assert_eq!(child_ids, ["alpha", "bravo", "charlie"]);
    }

    #[tokio::test]
    async fn same_root_turn_reuses_group_but_different_turn_does_not() {
        let store = TaskGroupStore::default();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);

        let first = create_root_group(&store, "turn-1", 0, deadline).await;
        let reused = create_root_group(&store, "turn-1", 0, deadline).await;
        let different_turn = create_root_group(&store, "turn-2", 0, deadline).await;

        assert_eq!(first, reused);
        assert_ne!(first, different_turn);
    }

    #[tokio::test]
    async fn parent_group_contains_only_directly_added_children() {
        let store = TaskGroupStore::default();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        let parent = store
            .create_for_parent(SessionId::new("s"), AgentId::new("parent"), 0, deadline)
            .await
            .unwrap();
        let child_group = store
            .create_for_parent(SessionId::new("s"), AgentId::new("child"), 0, deadline)
            .await
            .unwrap();
        store
            .add_child(&parent, AgentId::new("child"))
            .await
            .unwrap();
        store
            .add_child(&child_group, AgentId::new("grandchild"))
            .await
            .unwrap();
        store
            .record_result(&parent, result("child", ChildTerminalStatus::Completed))
            .await
            .unwrap();
        store
            .record_result(
                &child_group,
                result("grandchild", ChildTerminalStatus::Completed),
            )
            .await
            .unwrap();

        let first = store
            .claim_ready(&SessionId::new("s"), 0)
            .await
            .unwrap()
            .unwrap();
        let second = store
            .claim_ready(&SessionId::new("s"), 0)
            .await
            .unwrap()
            .unwrap();
        let deliveries = [first, second];
        let parent_delivery = deliveries
            .iter()
            .find(|delivery| delivery.group_id == parent)
            .unwrap();

        assert_eq!(parent_delivery.results.len(), 1);
        assert_eq!(parent_delivery.results[0].child_id.as_str(), "child");
    }

    #[tokio::test]
    async fn root_creation_rejects_a_stale_generation_after_advance() {
        let store = TaskGroupStore::default();
        let session_id = SessionId::new("s");
        assert_eq!(store.advance_generation(&session_id).await, 1);

        let error = store
            .create_for_root_turn(
                session_id.clone(),
                AgentId::new("root"),
                "turn-1",
                0,
                tokio::time::Instant::now() + Duration::from_secs(30),
            )
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            TaskGroupError::StaleGeneration {
                session_id: stale_session,
                expected: 1,
                actual: 0,
            } if stale_session == session_id
        ));
    }

    #[tokio::test]
    async fn parent_creation_rejects_a_mismatch_after_generation_is_initialized() {
        let store = TaskGroupStore::default();
        let session_id = SessionId::new("s");
        store
            .create_for_root_turn(
                session_id.clone(),
                AgentId::new("root"),
                "turn-1",
                0,
                tokio::time::Instant::now() + Duration::from_secs(30),
            )
            .await
            .unwrap();

        let error = store
            .create_for_parent(
                session_id.clone(),
                AgentId::new("parent"),
                1,
                tokio::time::Instant::now() + Duration::from_secs(30),
            )
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            TaskGroupError::StaleGeneration {
                session_id: stale_session,
                expected: 0,
                actual: 1,
            } if stale_session == session_id
        ));
    }

    #[tokio::test]
    async fn expired_group_rejects_new_children_and_late_results() {
        let store = TaskGroupStore::default();
        let group = create_root_group(
            &store,
            "turn-1",
            0,
            tokio::time::Instant::now() - Duration::from_secs(1),
        )
        .await;
        store.add_child(&group, AgentId::new("a")).await.unwrap();
        store
            .expire_due_groups(tokio::time::Instant::now())
            .await
            .unwrap();

        assert!(matches!(
            store.add_child(&group, AgentId::new("b")).await,
            Err(TaskGroupError::GroupClosed { .. })
        ));
        assert!(matches!(
            store
                .record_result(&group, result("a", ChildTerminalStatus::Completed))
                .await,
            Err(TaskGroupError::GroupClosed { .. })
        ));
    }

    #[tokio::test]
    async fn cancellation_removes_group_and_rejects_late_result() {
        let store = TaskGroupStore::default();
        let group = create_root_group(
            &store,
            "turn-1",
            0,
            tokio::time::Instant::now() + Duration::from_secs(30),
        )
        .await;
        store.add_child(&group, AgentId::new("a")).await.unwrap();

        assert_eq!(store.cancel_generation(&SessionId::new("s"), 0).await, 1);
        assert_eq!(store.group_count().await, 0);
        assert!(matches!(
            store
                .record_result(&group, result("a", ChildTerminalStatus::Completed))
                .await,
            Err(TaskGroupError::GroupNotFound { .. })
        ));
    }

    #[tokio::test]
    async fn claiming_removes_group_from_retained_state() {
        let store = TaskGroupStore::default();
        let group = create_root_group(
            &store,
            "turn-1",
            0,
            tokio::time::Instant::now() + Duration::from_secs(30),
        )
        .await;
        store.add_child(&group, AgentId::new("a")).await.unwrap();
        store
            .record_result(&group, result("a", ChildTerminalStatus::Completed))
            .await
            .unwrap();

        assert!(store
            .claim_ready(&SessionId::new("s"), 0)
            .await
            .unwrap()
            .is_some());
        assert_eq!(store.group_count().await, 0);
        assert!(matches!(
            store.add_child(&group, AgentId::new("b")).await,
            Err(TaskGroupError::GroupNotFound { .. })
        ));
    }

    #[tokio::test]
    async fn concurrent_claimers_deliver_a_ready_group_exactly_once() {
        let store = Arc::new(TaskGroupStore::default());
        let group = create_root_group(
            &store,
            "turn-1",
            0,
            tokio::time::Instant::now() + Duration::from_secs(30),
        )
        .await;
        store.add_child(&group, AgentId::new("a")).await.unwrap();
        store
            .record_result(&group, result("a", ChildTerminalStatus::Completed))
            .await
            .unwrap();
        let barrier = Arc::new(Barrier::new(3));

        let mut claimers = Vec::new();
        for _ in 0..2 {
            let store = store.clone();
            let barrier = barrier.clone();
            claimers.push(tokio::spawn(async move {
                barrier.wait().await;
                store.claim_ready(&SessionId::new("s"), 0).await.unwrap()
            }));
        }
        barrier.wait().await;

        let first = claimers.remove(0).await.unwrap();
        let second = claimers.remove(0).await.unwrap();
        assert_eq!(
            usize::from(first.is_some()) + usize::from(second.is_some()),
            1
        );
    }

    #[tokio::test]
    async fn owner_claim_does_not_deliver_another_owners_ready_group() {
        let store = TaskGroupStore::default();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        let root_group = store
            .create_for_root_turn(
                SessionId::new("s"),
                AgentId::new("root"),
                "turn-1",
                0,
                deadline,
            )
            .await
            .unwrap();
        let parent_group = store
            .create_for_parent(SessionId::new("s"), AgentId::new("parent"), 0, deadline)
            .await
            .unwrap();
        store
            .add_child(&parent_group, AgentId::new("grandchild"))
            .await
            .unwrap();
        store
            .record_result(
                &parent_group,
                result("grandchild", ChildTerminalStatus::Completed),
            )
            .await
            .unwrap();

        assert!(store
            .claim_ready_for_owner(&SessionId::new("s"), &AgentId::new("root"), 0)
            .await
            .unwrap()
            .is_none());
        assert_eq!(store.group_count().await, 2);

        store
            .add_child(&root_group, AgentId::new("child"))
            .await
            .unwrap();
        store
            .record_result(&root_group, result("child", ChildTerminalStatus::Completed))
            .await
            .unwrap();
        let delivery = store
            .claim_ready_for_owner(&SessionId::new("s"), &AgentId::new("root"), 0)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(delivery.group_id, root_group);
    }

    #[tokio::test]
    async fn specific_claim_requires_matching_group_owner_and_generation() {
        let store = TaskGroupStore::default();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        let group = store
            .create_for_parent(SessionId::new("s"), AgentId::new("parent"), 0, deadline)
            .await
            .unwrap();
        store
            .add_child(&group, AgentId::new("child"))
            .await
            .unwrap();
        store
            .record_result(&group, result("child", ChildTerminalStatus::Completed))
            .await
            .unwrap();

        assert!(store
            .claim_specific(&group, &SessionId::new("s"), &AgentId::new("other"), 0,)
            .await
            .unwrap()
            .is_none());
        assert!(store
            .claim_specific(&group, &SessionId::new("s"), &AgentId::new("parent"), 1,)
            .await
            .unwrap()
            .is_none());

        let delivery = store
            .claim_specific(&group, &SessionId::new("s"), &AgentId::new("parent"), 0)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(delivery.group_id, group);
        assert_eq!(store.group_count().await, 0);
    }
}
