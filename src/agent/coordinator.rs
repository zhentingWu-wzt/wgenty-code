//! Agent coordinator: exclusive owner of spawning, concurrency, and lifecycle.
//!
//! The coordinator is the only component permitted to create child agent
//! contexts, own semaphore permits, and transition lifecycle states. It
//! derives parentage, session, and depth entirely from trusted runtime
//! context--never from model-supplied JSON.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{OwnedSemaphorePermit, RwLock, Semaphore};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::agent::identity::{AgentExecutionContext, AgentId, AgentLifecycleStatus, SessionId};
use crate::agent::store::{InMemoryAgentStore, LocalAgentView, StoreError};

/// Request to spawn a child agent.
#[derive(Debug, Clone)]
pub struct SpawnChildRequest {
    /// Human-readable label for the child task.
    pub label: String,
}

impl SpawnChildRequest {
    /// Creates a new spawn request with the given label.
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
        }
    }
}

/// Terminal outcome reported by a child agent when it finishes.
#[derive(Debug, Clone)]
pub enum ChildTerminal {
    /// The child completed successfully.
    Completed {
        /// Human-readable summary of the result.
        summary: String,
    },
    /// The child failed.
    Failed {
        /// Machine-readable error code.
        code: String,
        /// Optional partial result produced before failure.
        partial_result: Option<String>,
    },
    /// The child was cancelled.
    Cancelled,
}

impl ChildTerminal {
    /// Creates a completed terminal with the given summary.
    pub fn completed(summary: impl Into<String>) -> Self {
        Self::Completed {
            summary: summary.into(),
        }
    }

    /// Maps this terminal to a lifecycle status.
    pub fn to_status(&self) -> AgentLifecycleStatus {
        match self {
            Self::Completed { .. } => AgentLifecycleStatus::Completed,
            Self::Failed { .. } => AgentLifecycleStatus::Failed,
            Self::Cancelled => AgentLifecycleStatus::Cancelled,
        }
    }
}

/// A reserved child slot carrying the trusted execution context.
///
/// The concurrency permit is retained internally by the coordinator's
/// `ScopeState` and released only after the child reaches a terminal state
/// through [`AgentCoordinator::finish_child`].
pub struct ChildReservation {
    /// Trusted execution context for the child agent.
    pub context: AgentExecutionContext,
}

/// Internal per-scope state owned by the coordinator.
struct ScopeState {
    status: AgentLifecycleStatus,
    #[expect(dead_code, reason = "used by recursive cancellation in Task 4")]
    cancellation: CancellationToken,
    #[allow(dead_code)]
    task: Option<JoinHandle<ChildTerminal>>,
    permit: Option<OwnedSemaphorePermit>,
    terminal: Option<ChildTerminal>,
}

/// Errors returned by the agent coordinator.
#[derive(Debug, thiserror::Error)]
pub enum CoordinatorError {
    /// The target is not visible from the current execution scope.
    #[error("agent is not visible from the current execution scope")]
    NotVisible,
    /// The configured maximum subagent depth was reached.
    #[error("maximum subagent depth {limit} reached")]
    DepthLimitReached {
        /// The depth limit that was exceeded.
        limit: usize,
    },
    /// The concurrency semaphore is closed.
    #[error("subagent concurrency is closed")]
    ConcurrencyClosed,
    /// The parent agent is not in a running state.
    #[error("parent agent is not running")]
    ParentNotRunning,
    /// A child join failed.
    #[error("child join failed: {0}")]
    JoinFailed(String),
    /// Underlying agent storage failed.
    #[error("agent storage failed: {0}")]
    Storage(String),
}

impl From<StoreError> for CoordinatorError {
    fn from(err: StoreError) -> Self {
        match err {
            StoreError::NotVisible => Self::NotVisible,
            other => Self::Storage(other.to_string()),
        }
    }
}

/// Exclusive owner of agent spawning, concurrency, and lifecycle transitions.
#[derive(Clone)]
pub struct AgentCoordinator {
    store: InMemoryAgentStore,
    permits: Arc<Semaphore>,
    max_depth: usize,
    scopes: Arc<RwLock<HashMap<(SessionId, AgentId), ScopeState>>>,
}

impl AgentCoordinator {
    /// Creates a coordinator with the given concurrency limit and max depth.
    pub fn new(max_concurrent: usize, max_depth: usize) -> Self {
        Self {
            store: InMemoryAgentStore::default(),
            permits: Arc::new(Semaphore::new(max_concurrent)),
            max_depth,
            scopes: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Returns the number of available concurrency permits.
    pub fn available_permits(&self) -> usize {
        self.permits.available_permits()
    }

    /// Returns the configured maximum subagent depth.
    pub fn max_depth(&self) -> usize {
        self.max_depth
    }

    /// Ensures a root execution context exists for the given session.
    ///
    /// If a root already exists for the session it is returned; otherwise a
    /// new root is created and registered. The root context is the trusted
    /// entry point--it is never derived from request JSON.
    pub async fn ensure_root(
        &self,
        session_id: SessionId,
    ) -> Result<AgentExecutionContext, CoordinatorError> {
        let context = AgentExecutionContext::root(session_id.clone());
        let key = (session_id.clone(), context.agent_id.clone());

        let mut scopes = self.scopes.write().await;
        if let Some(existing) = scopes.get(&key) {
            if existing.status.is_terminal() {
                return Err(CoordinatorError::ParentNotRunning);
            }
            return Ok(context);
        }

        let record =
            crate::agent::store::AgentRecord::new(session_id, context.agent_id.clone(), None, 0);
        self.store.insert(record).await?;

        scopes.insert(
            key,
            ScopeState {
                status: AgentLifecycleStatus::Running,
                cancellation: context.cancellation.clone(),
                task: None,
                permit: None,
                terminal: None,
            },
        );

        Ok(context)
    }

    /// Reserves a child agent slot under the given caller context.
    ///
    /// This acquires a concurrency permit, derives the child's trusted
    /// context (session, parent, depth) from the caller, inserts a pending
    /// record, and registers the scope. The permit is held until the child
    /// reaches a terminal state via [`finish_child`](Self::finish_child).
    pub async fn reserve_child(
        &self,
        caller: &AgentExecutionContext,
        request: SpawnChildRequest,
    ) -> Result<ChildReservation, CoordinatorError> {
        // Reject spawning if the caller is Cancelling or terminal.
        {
            let scopes = self.scopes.read().await;
            let key = (caller.session_id.clone(), caller.agent_id.clone());
            if let Some(state) = scopes.get(&key) {
                if state.status == AgentLifecycleStatus::Cancelling || state.status.is_terminal() {
                    return Err(CoordinatorError::ParentNotRunning);
                }
            } else {
                return Err(CoordinatorError::ParentNotRunning);
            }
        }

        // Enforce depth limit using trusted caller depth.
        if caller.depth >= self.max_depth {
            return Err(CoordinatorError::DepthLimitReached {
                limit: self.max_depth,
            });
        }

        // Acquire an owned semaphore permit before inserting the child.
        let permit = self
            .permits
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| CoordinatorError::ConcurrencyClosed)?;

        let child_id = AgentId::new(uuid::Uuid::new_v4().to_string());
        let context = caller.child(child_id.clone());
        let key = (context.session_id.clone(), context.agent_id.clone());

        let record = crate::agent::store::AgentRecord::new(
            context.session_id.clone(),
            context.agent_id.clone(),
            Some(caller.agent_id.clone()),
            context.depth,
        );
        self.store.insert(record).await?;

        {
            let mut scopes = self.scopes.write().await;
            scopes.insert(
                key,
                ScopeState {
                    status: AgentLifecycleStatus::Pending,
                    cancellation: context.cancellation.clone(),
                    task: None,
                    permit: Some(permit),
                    terminal: None,
                },
            );
        }

        // The permit stays in `ScopeState` so that `finish_child` can release
        // it after terminal persistence. Mark the child as running.
        {
            let mut scopes = self.scopes.write().await;
            let state = scopes
                .get_mut(&(context.session_id.clone(), context.agent_id.clone()))
                .expect("scope just inserted");
            state.status = AgentLifecycleStatus::Running;
        }

        // Drop request to avoid unused warnings in this phase.
        let _ = &request.label;

        Ok(ChildReservation { context })
    }

    /// Marks a child as terminal, persists the outcome, and releases its permit.
    pub async fn finish_child(
        &self,
        child: &AgentExecutionContext,
        terminal: ChildTerminal,
    ) -> Result<(), CoordinatorError> {
        let status = terminal.to_status();
        self.store
            .update_status(&child.session_id, &child.agent_id, status)
            .await?;

        let summary = match &terminal {
            ChildTerminal::Completed { summary } => Some(crate::agent::store::ChildSummary {
                text: summary.clone(),
                error_code: None,
            }),
            ChildTerminal::Failed { code, .. } => Some(crate::agent::store::ChildSummary {
                text: String::new(),
                error_code: Some(code.clone()),
            }),
            ChildTerminal::Cancelled => None,
        };

        if let Some(summary) = &summary {
            self.store
                .set_summary(&child.session_id, &child.agent_id, summary.clone())
                .await?;
        }

        let mut scopes = self.scopes.write().await;
        let key = (child.session_id.clone(), child.agent_id.clone());
        if let Some(state) = scopes.get_mut(&key) {
            state.status = status;
            state.terminal = Some(terminal);
            // Drop the permit to release the concurrency slot.
            state.permit.take();
        }

        Ok(())
    }

    /// Returns the local view (self plus direct children) for the caller.
    pub async fn list_local(
        &self,
        caller: &AgentExecutionContext,
    ) -> Result<LocalAgentView, CoordinatorError> {
        self.store
            .local_view(&caller.session_id, &caller.agent_id)
            .await
            .map_err(Into::into)
    }

    /// Returns the lifecycle status of an agent.
    pub async fn status(
        &self,
        context: &AgentExecutionContext,
    ) -> Result<AgentLifecycleStatus, CoordinatorError> {
        let scopes = self.scopes.read().await;
        let key = (context.session_id.clone(), context.agent_id.clone());
        if let Some(state) = scopes.get(&key) {
            return Ok(state.status);
        }
        // Fall back to the store record.
        let record = self
            .store
            .authorize_target(&context.session_id, &context.agent_id, &context.agent_id)
            .await?;
        Ok(record.status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_coordinator(max_concurrent: usize, max_depth: usize) -> AgentCoordinator {
        AgentCoordinator::new(max_concurrent, max_depth)
    }

    #[tokio::test]
    async fn spawn_derives_parent_depth_and_session() {
        let coordinator = test_coordinator(4, 3);
        let root = coordinator.ensure_root(SessionId::new("s")).await.unwrap();
        let child = coordinator
            .reserve_child(&root, SpawnChildRequest::new("work"))
            .await
            .unwrap();

        assert_eq!(child.context.session_id, root.session_id);
        assert_eq!(child.context.parent_id.as_ref(), Some(&root.agent_id));
        assert_eq!(child.context.depth, root.depth + 1);
    }

    #[tokio::test]
    async fn depth_limit_is_enforced_by_coordinator() {
        let coordinator = test_coordinator(4, 1);
        let root = coordinator.ensure_root(SessionId::new("s")).await.unwrap();
        let child = coordinator
            .reserve_child(&root, SpawnChildRequest::new("child"))
            .await
            .unwrap();
        assert!(matches!(
            coordinator
                .reserve_child(&child.context, SpawnChildRequest::new("too deep"))
                .await,
            Err(CoordinatorError::DepthLimitReached { limit: 1 })
        ));
    }

    #[tokio::test]
    async fn semaphore_permit_returns_after_terminal_cleanup() {
        let coordinator = test_coordinator(1, 3);
        let root = coordinator.ensure_root(SessionId::new("s")).await.unwrap();
        let child = coordinator
            .reserve_child(&root, SpawnChildRequest::new("first"))
            .await
            .unwrap();
        assert_eq!(coordinator.available_permits(), 0);
        coordinator
            .finish_child(&child.context, ChildTerminal::completed("done"))
            .await
            .unwrap();
        assert_eq!(coordinator.available_permits(), 1);
        assert!(coordinator
            .reserve_child(&root, SpawnChildRequest::new("second"))
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn ensure_root_is_idempotent_for_running_session() {
        let coordinator = test_coordinator(4, 3);
        let root = coordinator.ensure_root(SessionId::new("s")).await.unwrap();
        let root_again = coordinator.ensure_root(SessionId::new("s")).await.unwrap();
        // Same session; both are valid root contexts.
        assert_eq!(root.session_id, root_again.session_id);
    }

    #[tokio::test]
    async fn reserve_child_rejects_terminal_parent() {
        let coordinator = test_coordinator(4, 3);
        let root = coordinator.ensure_root(SessionId::new("s")).await.unwrap();
        coordinator
            .finish_child(&root, ChildTerminal::completed("root done"))
            .await
            .unwrap();
        assert!(matches!(
            coordinator
                .reserve_child(&root, SpawnChildRequest::new("after"))
                .await,
            Err(CoordinatorError::ParentNotRunning)
        ));
    }

    #[tokio::test]
    async fn list_local_returns_self_and_direct_children() {
        let coordinator = test_coordinator(4, 3);
        let root = coordinator.ensure_root(SessionId::new("s")).await.unwrap();
        let child = coordinator
            .reserve_child(&root, SpawnChildRequest::new("child-a"))
            .await
            .unwrap();

        let view = coordinator.list_local(&root).await.unwrap();
        assert_eq!(view.self_view.agent_id, root.agent_id);
        assert_eq!(view.children.len(), 1);
        assert_eq!(view.children[0].agent_id, child.context.agent_id);
    }
}
