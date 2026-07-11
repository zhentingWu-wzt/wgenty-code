//! Agent coordinator: exclusive owner of spawning, concurrency, and lifecycle.
//!
//! The coordinator is the only component permitted to create child agent
//! contexts, own semaphore permits, and transition lifecycle states. It
//! derives parentage, session, and depth entirely from trusted runtime
//! context--never from model-supplied JSON.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::{Notify, OwnedSemaphorePermit, RwLock, Semaphore};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::agent::identity::{AgentExecutionContext, AgentId, AgentLifecycleStatus, SessionId};
use crate::agent::store::{InMemoryAgentStore, LocalAgentView, StoreError};

/// Default bounded shutdown timeout applied while awaiting cancelling children.
///
/// A child that does not observe cooperative cancellation within this window is
/// forcibly aborted so terminal cleanup can still complete and its permit is
/// released. The timeout is configurable via
/// [`AgentCoordinator::with_shutdown_timeout`].
const DEFAULT_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(30);

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

    /// Maps this terminal to a serializable child terminal status.
    pub fn to_child_status(&self) -> ChildTerminalStatus {
        match self {
            Self::Completed { .. } => ChildTerminalStatus::Completed,
            Self::Failed { .. } => ChildTerminalStatus::Failed,
            Self::Cancelled => ChildTerminalStatus::Cancelled,
        }
    }
}

/// Policy governing how a parent finalization or join waits for direct children.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinPolicy {
    /// Wait for every direct child and surface each terminal result.
    ///
    /// If any child fails or is cancelled, the parent still receives that
    /// terminal result and decides whether its own work can succeed.
    AllRequired,
    /// Wait for all direct children regardless of outcome; aggregate successes
    /// and failures together.
    BestEffort,
    /// On the first required-child failure, cancel remaining live direct
    /// children, wait for their cancellation, then return failure.
    FailFast,
}

/// Terminal outcome requested for a parent scope during finalization.
#[derive(Debug, Clone)]
pub enum ParentOutcome {
    /// The parent completed successfully.
    Completed(String),
    /// The parent failed.
    Failed {
        /// Machine-readable error code.
        code: String,
        /// Optional partial result produced before failure.
        partial_result: Option<String>,
    },
    /// The parent was cancelled.
    Cancelled,
}

impl ParentOutcome {
    /// Maps this outcome to the lifecycle status the parent should reach.
    pub fn to_status(&self) -> AgentLifecycleStatus {
        match self {
            Self::Completed { .. } => AgentLifecycleStatus::Completed,
            Self::Failed { .. } => AgentLifecycleStatus::Failed,
            Self::Cancelled => AgentLifecycleStatus::Cancelled,
        }
    }

    /// Maps this outcome to the terminal a parent would record if it were a
    /// child of an outer scope. Used to derive a [`ChildResult`] for the parent.
    fn to_child_terminal(&self) -> ChildTerminal {
        match self {
            Self::Completed(summary) => ChildTerminal::Completed {
                summary: summary.clone(),
            },
            Self::Failed {
                code,
                partial_result,
            } => ChildTerminal::Failed {
                code: code.clone(),
                partial_result: partial_result.clone(),
            },
            Self::Cancelled => ChildTerminal::Cancelled,
        }
    }
}

/// Serializable terminal status of a direct child, returned to its parent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChildTerminalStatus {
    /// The child completed successfully.
    Completed,
    /// The child failed.
    Failed,
    /// The child was cancelled.
    Cancelled,
}

/// Bounded result a direct child returns to its parent.
///
/// `summary` and `partial_result` must not include descendant identifiers,
/// raw descendant transcripts, or serialized tree structures. The child is
/// responsible for synthesizing descendant work into its own result before
/// returning it upward.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChildResult {
    /// Identity of the child agent that produced this result.
    pub child_id: AgentId,
    /// Terminal status of the child.
    pub status: ChildTerminalStatus,
    /// Human-readable summary of the result.
    pub summary: String,
    /// Optional machine-readable error code.
    pub error_code: Option<String>,
    /// Optional partial result produced before the terminal state.
    pub partial_result: Option<String>,
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
    cancellation: CancellationToken,
    /// Notified when this scope reaches a terminal state, so a joining parent
    /// can resolve even when the child's terminal was set via `finish_child`
    /// rather than its registered task completing.
    terminal_notify: Arc<Notify>,
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
    shutdown_timeout: Duration,
    scopes: Arc<RwLock<HashMap<(SessionId, AgentId), ScopeState>>>,
}

impl AgentCoordinator {
    /// Creates a coordinator with the given concurrency limit and max depth.
    ///
    /// The bounded shutdown timeout defaults to [`DEFAULT_SHUTDOWN_TIMEOUT`].
    pub fn new(max_concurrent: usize, max_depth: usize) -> Self {
        Self {
            store: InMemoryAgentStore::default(),
            permits: Arc::new(Semaphore::new(max_concurrent)),
            max_depth,
            shutdown_timeout: DEFAULT_SHUTDOWN_TIMEOUT,
            scopes: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Sets the bounded shutdown timeout used while awaiting cancelling children.
    ///
    /// A child that does not observe cooperative cancellation within this
    /// window is forcibly aborted so terminal cleanup can still complete.
    #[must_use]
    pub fn with_shutdown_timeout(mut self, timeout: Duration) -> Self {
        self.shutdown_timeout = timeout;
        self
    }

    /// Returns the configured shutdown timeout.
    pub fn shutdown_timeout(&self) -> Duration {
        self.shutdown_timeout
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
                terminal_notify: Arc::new(Notify::new()),
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
                    terminal_notify: Arc::new(Notify::new()),
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
            // Wake any parent blocked in `await_child_terminal` so the join
            // resolves when the terminal is set here rather than only when the
            // child's registered task completes.
            state.terminal_notify.notify_waiters();
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

    /// Registers the task handle backing a reserved child scope.
    ///
    /// The coordinator owns the handle so that finalization, joining, and
    /// cancellation can await or abort the child future. The handle's output is
    /// the [`ChildTerminal`] the child produces on natural completion. This is a
    /// trusted registration: only the component that created the child future
    /// (via [`reserve_child`](Self::reserve_child)) should register it.
    pub async fn register_task(
        &self,
        child: &AgentExecutionContext,
        task: JoinHandle<ChildTerminal>,
    ) -> Result<(), CoordinatorError> {
        let mut scopes = self.scopes.write().await;
        let key = (child.session_id.clone(), child.agent_id.clone());
        let state = scopes
            .get_mut(&key)
            .ok_or(CoordinatorError::ParentNotRunning)?;
        state.task = Some(task);
        Ok(())
    }

    /// Returns whether a scope is live (non-terminal and not yet `Cancelling`).
    ///
    /// A scope is live in `Pending`, `Running`, `WaitingForChildren`, and
    /// `Finalizing`. It stops being live once it enters `Cancelling` or a
    /// terminal state.
    fn is_live(status: AgentLifecycleStatus) -> bool {
        !status.is_terminal() && status != AgentLifecycleStatus::Cancelling
    }

    /// Collects the live direct-child scopes of `caller`.
    ///
    /// Returns `(child_id, child_context)` pairs for every direct child whose
    /// scope is still live, so finalization and cancellation can target them.
    async fn live_direct_children(
        &self,
        caller: &AgentExecutionContext,
    ) -> Result<Vec<(AgentId, AgentExecutionContext)>, CoordinatorError> {
        let children = self
            .store
            .direct_children(&caller.session_id, &caller.agent_id)
            .await?;

        let scopes = self.scopes.read().await;
        let mut live = Vec::new();
        for child in children {
            let key = (caller.session_id.clone(), child.agent_id.clone());
            let status = scopes.get(&key).map(|s| s.status).unwrap_or(child.status);
            if !Self::is_live(status) {
                continue;
            }
            // Reconstruct the child context for cancellation signalling. The
            // cancellation token is shared from the scope state so cancelling
            // the derived token propagates to the child future.
            let cancellation = scopes
                .get(&key)
                .map(|s| s.cancellation.clone())
                .unwrap_or_else(CancellationToken::new);
            let child_context = AgentExecutionContext {
                session_id: child.session_id.clone(),
                agent_id: child.agent_id.clone(),
                parent_id: Some(caller.agent_id.clone()),
                depth: child.depth,
                cancellation,
            };
            live.push((child.agent_id.clone(), child_context));
        }
        Ok(live)
    }

    /// Converts a stored terminal into a bounded [`ChildResult`] for the parent.
    fn child_result_from_terminal(child_id: &AgentId, terminal: &ChildTerminal) -> ChildResult {
        let (summary, error_code, partial_result) = match terminal {
            ChildTerminal::Completed { summary } => (summary.clone(), None, None),
            ChildTerminal::Failed {
                code,
                partial_result,
            } => (String::new(), Some(code.clone()), partial_result.clone()),
            ChildTerminal::Cancelled => (String::new(), None, None),
        };
        ChildResult {
            child_id: child_id.clone(),
            status: terminal.to_child_status(),
            summary,
            error_code,
            partial_result,
        }
    }

    /// Joins all direct children of `caller` according to `policy`.
    ///
    /// Awaits each live direct child's task handle. If a child has no
    /// registered task (for example, a recovery scenario) and has already
    /// reached a terminal state, its stored terminal is surfaced directly;
    /// otherwise it is awaited until natural completion.
    ///
    /// `AllRequired` and `BestEffort` wait for every direct child. `FailFast`
    /// is handled by [`join_children_failfast`](Self::join_children_failfast),
    /// which finalization routes to separately; passing it here is treated the
    /// same as `AllRequired` for the join itself.
    pub async fn join_children(
        &self,
        caller: &AgentExecutionContext,
        _policy: JoinPolicy,
    ) -> Result<Vec<ChildResult>, CoordinatorError> {
        let children = self
            .store
            .direct_children(&caller.session_id, &caller.agent_id)
            .await?;

        let mut results = Vec::new();
        for child in children {
            let child_context = AgentExecutionContext {
                session_id: child.session_id.clone(),
                agent_id: child.agent_id.clone(),
                parent_id: Some(caller.agent_id.clone()),
                depth: child.depth,
                cancellation: self.scope_token(&child.session_id, &child.agent_id).await,
            };

            let terminal = self.await_child_terminal(&child_context).await?;
            results.push(Self::child_result_from_terminal(&child.agent_id, &terminal));
        }

        Ok(results)
    }

    /// Returns the stored cancellation token for a scope, or a fresh token if
    /// the scope is not registered (recovery/foreign).
    async fn scope_token(&self, session: &SessionId, agent: &AgentId) -> CancellationToken {
        let scopes = self.scopes.read().await;
        scopes
            .get(&(session.clone(), agent.clone()))
            .map(|s| s.cancellation.clone())
            .unwrap_or_else(CancellationToken::new)
    }

    /// Awaits a child's terminal, resolving when the child reaches a terminal
    /// state by any path.
    ///
    /// The child may reach its terminal either by its registered task producing
    /// a [`ChildTerminal`] on natural completion, or by an external caller
    /// invoking [`finish_child`](Self::finish_child) (for example, the task tool
    /// finishing a direct child). This method races both signals so a join can
    /// never hang on a future that completes via `finish_child`.
    async fn await_child_terminal(
        &self,
        child: &AgentExecutionContext,
    ) -> Result<ChildTerminal, CoordinatorError> {
        // Fast path: already terminal.
        if let Some(terminal) = self.stored_terminal(child).await {
            return Ok(terminal);
        }

        // Take the registered task handle (if any) and the terminal notify.
        let (task, notify) = {
            let mut scopes = self.scopes.write().await;
            let key = (child.session_id.clone(), child.agent_id.clone());
            match scopes.get_mut(&key) {
                Some(s) => (s.task.take(), Some(s.terminal_notify.clone())),
                None => (None, None),
            }
        };
        let notify = notify.unwrap_or_else(|| Arc::new(Notify::new()));

        // Register the notification future BEFORE checking state so a terminal
        // set concurrently with `finish_child` cannot slip between the state
        // check and the registration (Notify only wakes already-registered
        // waiters).
        let mut task = task;
        loop {
            let notified = notify.notified();
            // Pin the registration, then re-check state.
            if let Some(terminal) = self.stored_terminal(child).await {
                return Ok(terminal);
            }

            if let Some(handle) = task.as_mut() {
                tokio::select! {
                    biased;
                    join_result = handle => {
                        match join_result {
                            Ok(terminal) => {
                                self.finish_child(child, terminal.clone()).await?;
                                return Ok(terminal);
                            }
                            Err(join_err) => {
                                return Err(CoordinatorError::JoinFailed(join_err.to_string()));
                            }
                        }
                    }
                    _ = notified => {
                        // Terminal may have been set externally; re-check at top.
                    }
                }
            } else {
                // No task: wait for an external terminal signal, then re-check.
                notified.await;
            }
        }
    }

    /// Returns the stored terminal for a scope if it has reached one.
    async fn stored_terminal(&self, child: &AgentExecutionContext) -> Option<ChildTerminal> {
        let scopes = self.scopes.read().await;
        let key = (child.session_id.clone(), child.agent_id.clone());
        scopes.get(&key).and_then(|s| {
            if s.status.is_terminal() {
                s.terminal.clone()
            } else {
                None
            }
        })
    }

    /// Recursively cancels the subtree rooted at `scope`.
    ///
    /// Cancellation proceeds top-down and completion is observed bottom-up:
    /// set `Cancelling`, cancel the scope token, recursively signal live direct
    /// children, await handles with the shutdown timeout, abort uncooperative
    /// tasks, persist child terminal states, and release child permits.
    async fn cancel_descendants(
        &self,
        scope: &AgentExecutionContext,
    ) -> Result<(), CoordinatorError> {
        // Mark this scope Cancelling and signal its token so the scope's future
        // (and all derived child tokens) are cooperatively cancelled.
        let scope_token = {
            let mut scopes = self.scopes.write().await;
            let key = (scope.session_id.clone(), scope.agent_id.clone());
            match scopes.get_mut(&key) {
                Some(state) if state.status.is_terminal() => return Ok(()),
                Some(state) => {
                    state.status = AgentLifecycleStatus::Cancelling;
                    self.store
                        .update_status(
                            &scope.session_id,
                            &scope.agent_id,
                            AgentLifecycleStatus::Cancelling,
                        )
                        .await?;
                    state.cancellation.clone()
                }
                None => return Ok(()),
            }
        };
        // Cancel the scope's token: this cascades to every derived child token.
        scope_token.cancel();

        // Collect live direct children before awaiting (their tokens are now
        // signalled). Recurse top-down into each live child.
        let live_children = self.live_direct_children(scope).await?;
        for (_child_id, child_context) in &live_children {
            Box::pin(self.cancel_descendants(child_context)).await?;
        }

        // Await each child's task handle with the shutdown timeout, aborting
        // uncooperative tasks, then persist terminal state bottom-up.
        for (child_id, child_context) in &live_children {
            let _ = child_id;
            self.await_cancelled_child(child_context).await?;
        }

        Ok(())
    }

    /// Awaits a cancelling child with the bounded shutdown timeout.
    ///
    /// If the child's future does not complete within the shutdown timeout, it
    /// is forcibly aborted and the child is persisted as `Cancelled`.
    async fn await_cancelled_child(
        &self,
        child: &AgentExecutionContext,
    ) -> Result<(), CoordinatorError> {
        // If already terminal, nothing to do.
        {
            let scopes = self.scopes.read().await;
            let key = (child.session_id.clone(), child.agent_id.clone());
            if let Some(state) = scopes.get(&key) {
                if state.status.is_terminal() {
                    return Ok(());
                }
            }
        }

        let task = {
            let mut scopes = self.scopes.write().await;
            let key = (child.session_id.clone(), child.agent_id.clone());
            scopes.get_mut(&key).and_then(|s| s.task.take())
        };

        if let Some(mut task) = task {
            // Race the child future against the shutdown timeout so the
            // JoinHandle is retained for forced abort on timeout.
            tokio::select! {
                biased;
                join_result = &mut task => match join_result {
                    Ok(terminal) => {
                        // The child produced a terminal; honor it (typically Cancelled).
                        self.finish_child(child, terminal).await?;
                    }
                    Err(join_err) => {
                        return Err(CoordinatorError::JoinFailed(join_err.to_string()));
                    }
                },
                _ = tokio::time::sleep(self.shutdown_timeout) => {
                    // Timed out: forcibly abort and record cancellation.
                    task.abort();
                    self.finish_child(child, ChildTerminal::Cancelled).await?;
                }
            }
        } else {
            // No registered task: persist as cancelled.
            self.finish_child(child, ChildTerminal::Cancelled).await?;
        }

        Ok(())
    }

    /// Finalizes the `caller` scope: waits for or cancels direct children,
    /// then persists the requested parent terminal state.
    ///
    /// The parent cannot become terminal while a scoped child is live. The
    /// transition to the requested terminal state is rejected unless every
    /// direct child reaches a terminal state first.
    pub async fn finalize_scope(
        &self,
        caller: &AgentExecutionContext,
        outcome: ParentOutcome,
        policy: JoinPolicy,
    ) -> Result<Vec<ChildResult>, CoordinatorError> {
        // Move into WaitingForChildren while any direct child is still live.
        let has_live = !self.live_direct_children(caller).await?.is_empty();
        if has_live {
            {
                let mut scopes = self.scopes.write().await;
                let key = (caller.session_id.clone(), caller.agent_id.clone());
                if let Some(state) = scopes.get_mut(&key) {
                    if Self::is_live(state.status) {
                        state.status = AgentLifecycleStatus::WaitingForChildren;
                        self.store
                            .update_status(
                                &caller.session_id,
                                &caller.agent_id,
                                AgentLifecycleStatus::WaitingForChildren,
                            )
                            .await?;
                    }
                }
            }
        }

        let results = match policy {
            JoinPolicy::AllRequired | JoinPolicy::BestEffort => {
                self.join_children(caller, policy).await?
            }
            JoinPolicy::FailFast => {
                // Join children one at a time; on the first failure/cancel,
                // cancel the remaining live children, then collect their results.
                self.join_children_failfast(caller).await?
            }
        };

        // Transition to Finalizing while aggregating.
        {
            let mut scopes = self.scopes.write().await;
            let key = (caller.session_id.clone(), caller.agent_id.clone());
            if let Some(state) = scopes.get_mut(&key) {
                state.status = AgentLifecycleStatus::Finalizing;
                self.store
                    .update_status(
                        &caller.session_id,
                        &caller.agent_id,
                        AgentLifecycleStatus::Finalizing,
                    )
                    .await?;
            }
        }

        // Persist the parent's terminal state. `finish_child` works for any
        // scope (root or child): it updates the store, stores the terminal,
        // releases the permit, and notifies any outer joiner.
        self.finish_child(caller, outcome.to_child_terminal())
            .await?;

        Ok(results)
    }

    /// FailFast join: on the first required-child failure, cancel remaining
    /// live children, await their cancellation, then return all results.
    async fn join_children_failfast(
        &self,
        caller: &AgentExecutionContext,
    ) -> Result<Vec<ChildResult>, CoordinatorError> {
        let children = self
            .store
            .direct_children(&caller.session_id, &caller.agent_id)
            .await?;

        let mut results = Vec::new();
        let mut failed = false;
        for child in children {
            let child_context = AgentExecutionContext {
                session_id: child.session_id.clone(),
                agent_id: child.agent_id.clone(),
                parent_id: Some(caller.agent_id.clone()),
                depth: child.depth,
                cancellation: self.scope_token(&child.session_id, &child.agent_id).await,
            };

            if failed {
                // Cancel remaining live children.
                self.cancel_descendants(&child_context).await?;
            }

            let terminal = self.await_child_terminal(&child_context).await?;
            let result = Self::child_result_from_terminal(&child.agent_id, &terminal);
            if matches!(
                terminal,
                ChildTerminal::Failed { .. } | ChildTerminal::Cancelled
            ) {
                failed = true;
            }
            results.push(result);
        }

        Ok(results)
    }

    /// Cancels the `caller` scope and its entire subtree.
    ///
    /// The caller is marked `Cancelling`, its cancellation token is signalled,
    /// live descendants are cancelled bottom-up, and the caller is persisted as
    /// `Cancelled` after all descendants terminate.
    pub async fn cancel_scope(
        &self,
        caller: &AgentExecutionContext,
    ) -> Result<(), CoordinatorError> {
        self.cancel_descendants(caller).await?;

        // Persist the caller as Cancelled and release its permit.
        self.finish_child(caller, ChildTerminal::Cancelled).await?;
        Ok(())
    }

    /// Cancels a target subtree that is the caller itself or one of its direct
    /// children.
    ///
    /// Authorization uses the same store predicate as all targeted operations:
    /// the target must be the caller or a direct child. All hidden, absent, and
    /// out-of-scope targets map to [`CoordinatorError::NotVisible`].
    pub async fn cancel_subtree(
        &self,
        caller: &AgentExecutionContext,
        target: AgentId,
    ) -> Result<(), CoordinatorError> {
        // Authorize: target must be self or a direct child.
        let is_self = target == caller.agent_id;
        let is_direct_child = if is_self {
            false
        } else {
            self.store
                .direct_children(&caller.session_id, &caller.agent_id)
                .await?
                .iter()
                .any(|c| c.agent_id == target)
        };

        if !is_self && !is_direct_child {
            return Err(CoordinatorError::NotVisible);
        }

        let target_context = if is_self {
            caller.clone()
        } else {
            // Reconstruct a trusted child context for the direct child.
            let child_record = self
                .store
                .direct_children(&caller.session_id, &caller.agent_id)
                .await?
                .into_iter()
                .find(|c| c.agent_id == target)
                .ok_or(CoordinatorError::NotVisible)?;
            AgentExecutionContext {
                session_id: child_record.session_id.clone(),
                agent_id: child_record.agent_id.clone(),
                parent_id: Some(caller.agent_id.clone()),
                depth: child_record.depth,
                cancellation: self
                    .scope_token(&child_record.session_id, &child_record.agent_id)
                    .await,
            }
        };

        self.cancel_descendants(&target_context).await?;

        if is_self {
            // Self-cancellation persists the caller as Cancelled.
            self.finish_child(caller, ChildTerminal::Cancelled).await?;
        }
        // For a direct child, cancel_descendants already persisted it Cancelled.

        Ok(())
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

    // ---- Task 4: structured finalization, joining, cancellation ----

    /// Spawns a child future controlled by a oneshot and registers its handle.
    ///
    /// The child awaits `release` before producing `terminal`, simulating a
    /// live child that cannot finish until the test signals it.
    async fn register_controlled_child(
        coordinator: &AgentCoordinator,
        caller: &AgentExecutionContext,
        terminal: ChildTerminal,
    ) -> (AgentExecutionContext, tokio::sync::oneshot::Sender<()>) {
        let reservation = coordinator
            .reserve_child(caller, SpawnChildRequest::new("controlled"))
            .await
            .unwrap();
        let context = reservation.context.clone();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
        let terminal_clone = terminal.clone();
        let token = context.cancellation.clone();
        // The future is cooperative: it resolves when released (producing the
        // configured terminal) or when its cancellation token fires (Cancelled).
        let task = tokio::spawn(async move {
            tokio::select! {
                biased;
                _ = token.cancelled() => ChildTerminal::Cancelled,
                _ = release_rx => terminal_clone,
            }
        });
        coordinator.register_task(&context, task).await.unwrap();
        (context, release_tx)
    }

    async fn running_parent_and_child() -> (
        AgentCoordinator,
        AgentExecutionContext,
        AgentExecutionContext,
    ) {
        let coordinator = test_coordinator(4, 3);
        let root = coordinator.ensure_root(SessionId::new("s")).await.unwrap();
        let (child, release) =
            register_controlled_child(&coordinator, &root, ChildTerminal::completed("child done"))
                .await;
        // Keep the release sender alive for the lifetime of the test so the
        // child's future stays blocked until `finish_child` externally sets its
        // terminal. Leaking is acceptable in a test fixture.
        Box::leak(Box::new(release));
        (coordinator, root, child)
    }

    #[tokio::test]
    async fn parent_waits_for_live_direct_children_before_completion() {
        let (coordinator, root, child) = running_parent_and_child().await;
        let coordinator_for_finalize = coordinator.clone();
        let root_for_finalize = root.clone();
        let finalize = tokio::spawn(async move {
            coordinator_for_finalize
                .finalize_scope(
                    &root_for_finalize,
                    ParentOutcome::Completed("root done".into()),
                    JoinPolicy::AllRequired,
                )
                .await
        });
        tokio::task::yield_now().await;
        assert_eq!(
            coordinator.status(&root).await.unwrap(),
            AgentLifecycleStatus::WaitingForChildren
        );

        let coordinator_for_finish = coordinator.clone();
        let child_for_finish = child.clone();
        let finish = tokio::spawn(async move {
            coordinator_for_finish
                .finish_child(&child_for_finish, ChildTerminal::completed("child done"))
                .await
                .unwrap()
        });
        finish.await.unwrap();
        let results = finalize.await.unwrap().unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(
            coordinator.status(&root).await.unwrap(),
            AgentLifecycleStatus::Completed
        );
    }

    /// A three-level running tree: root -> child -> grandchild, each with a
    /// controlled live future.
    struct ThreeLevelTree {
        coordinator: AgentCoordinator,
        root: AgentExecutionContext,
        child: AgentExecutionContext,
        grandchild: AgentExecutionContext,
        max_concurrent: usize,
    }

    async fn three_level_running_tree() -> ThreeLevelTree {
        let max_concurrent = 8;
        let coordinator = test_coordinator(max_concurrent, 3);
        let root = coordinator.ensure_root(SessionId::new("s")).await.unwrap();
        let (child, release) =
            register_controlled_child(&coordinator, &root, ChildTerminal::completed("child")).await;
        // Keep release alive so the child stays live until cancellation drives
        // its terminal. Leaking is acceptable in a test fixture.
        Box::leak(Box::new(release));
        let (grandchild, release) =
            register_controlled_child(&coordinator, &child, ChildTerminal::completed("grandchild"))
                .await;
        Box::leak(Box::new(release));
        ThreeLevelTree {
            coordinator,
            root,
            child,
            grandchild,
            max_concurrent,
        }
    }

    #[tokio::test]
    async fn cancelling_parent_terminates_descendants_bottom_up() {
        let fixture = three_level_running_tree().await;
        fixture
            .coordinator
            .cancel_scope(&fixture.root)
            .await
            .unwrap();
        assert_eq!(
            fixture
                .coordinator
                .status(&fixture.grandchild)
                .await
                .unwrap(),
            AgentLifecycleStatus::Cancelled
        );
        assert_eq!(
            fixture.coordinator.status(&fixture.child).await.unwrap(),
            AgentLifecycleStatus::Cancelled
        );
        assert_eq!(
            fixture.coordinator.status(&fixture.root).await.unwrap(),
            AgentLifecycleStatus::Cancelled
        );
        assert_eq!(
            fixture.coordinator.available_permits(),
            fixture.max_concurrent
        );
    }

    #[tokio::test]
    async fn best_effort_waits_for_all_children() {
        let coordinator = test_coordinator(8, 3);
        let root = coordinator.ensure_root(SessionId::new("s")).await.unwrap();
        // Two children: one completes, one fails. BestEffort waits for both.
        let (child_a, release_a) =
            register_controlled_child(&coordinator, &root, ChildTerminal::completed("a done"))
                .await;
        let (child_b, release_b) = register_controlled_child(
            &coordinator,
            &root,
            ChildTerminal::Failed {
                code: "boom".into(),
                partial_result: None,
            },
        )
        .await;
        // Release both so their futures settle naturally.
        drop(release_a);
        drop(release_b);

        let results = coordinator
            .finalize_scope(
                &root,
                ParentOutcome::Completed("root".into()),
                JoinPolicy::BestEffort,
            )
            .await
            .unwrap();

        // Both children must be represented in the result set.
        let ids: Vec<&str> = results.iter().map(|r| r.child_id.as_str()).collect();
        assert!(ids.contains(&child_a.agent_id.as_str()));
        assert!(ids.contains(&child_b.agent_id.as_str()));
        assert_eq!(
            coordinator.status(&root).await.unwrap(),
            AgentLifecycleStatus::Completed
        );
    }

    #[tokio::test]
    async fn fail_fast_cancels_remaining_children_after_first_failure() {
        let coordinator = test_coordinator(8, 3);
        let root = coordinator.ensure_root(SessionId::new("s")).await.unwrap();
        // First child fails when released.
        let (child_a, release_a) = register_controlled_child(
            &coordinator,
            &root,
            ChildTerminal::Failed {
                code: "boom".into(),
                partial_result: None,
            },
        )
        .await;
        // Second child is live and must be cancelled by FailFast. Keep its
        // release alive so cancellation--not natural completion--drives its
        // terminal, proving FailFast cancels the remaining child.
        let (child_b, release_b) =
            register_controlled_child(&coordinator, &root, ChildTerminal::completed("b done"))
                .await;
        Box::leak(Box::new(release_b));

        // Release the first child so it settles to Failed, triggering FailFast.
        drop(release_a);

        let results = coordinator
            .finalize_scope(
                &root,
                ParentOutcome::Failed {
                    code: "child_failed".into(),
                    partial_result: None,
                },
                JoinPolicy::FailFast,
            )
            .await
            .unwrap();

        // Both children reach a terminal state.
        assert_eq!(
            coordinator.status(&child_a).await.unwrap(),
            AgentLifecycleStatus::Failed
        );
        assert_eq!(
            coordinator.status(&child_b).await.unwrap(),
            AgentLifecycleStatus::Cancelled
        );
        assert_eq!(
            coordinator.status(&root).await.unwrap(),
            AgentLifecycleStatus::Failed
        );
        // All permits released after finalization.
        assert_eq!(coordinator.available_permits(), 8);
        // Both results surfaced.
        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn cancel_subtree_authorizes_self_and_direct_child_only() {
        let coordinator = test_coordinator(8, 3);
        let root = coordinator.ensure_root(SessionId::new("s")).await.unwrap();
        let (child, release) =
            register_controlled_child(&coordinator, &root, ChildTerminal::completed("child")).await;
        // Keep the child live until cancellation drives its terminal.
        Box::leak(Box::new(release));

        // Self-cancellation is allowed.
        assert!(coordinator
            .cancel_subtree(&root, root.agent_id.clone())
            .await
            .is_ok());

        // Direct child cancellation is allowed (child already terminal via root).
        assert!(coordinator
            .cancel_subtree(&root, child.agent_id.clone())
            .await
            .is_ok());

        // A non-existent / hidden target is NotVisible.
        assert!(matches!(
            coordinator
                .cancel_subtree(&root, AgentId::new("grandchild-or-stranger"))
                .await,
            Err(CoordinatorError::NotVisible)
        ));
    }

    #[tokio::test]
    async fn forced_abort_releases_permit_after_shutdown_timeout() {
        // A child that ignores cooperative cancellation must be aborted and its
        // permit released after the shutdown timeout.
        let coordinator =
            AgentCoordinator::new(1, 3).with_shutdown_timeout(Duration::from_millis(50));
        let root = coordinator.ensure_root(SessionId::new("s")).await.unwrap();
        let reservation = coordinator
            .reserve_child(&root, SpawnChildRequest::new("uncooperative"))
            .await
            .unwrap();
        let context = reservation.context.clone();
        // A future that never completes and ignores cancellation.
        let task = tokio::spawn(async move {
            std::future::pending::<()>().await;
            ChildTerminal::completed("never")
        });
        coordinator.register_task(&context, task).await.unwrap();

        assert_eq!(coordinator.available_permits(), 0);
        coordinator.cancel_scope(&root).await.unwrap();
        // Child aborted and permit released.
        assert_eq!(coordinator.available_permits(), 1);
        assert_eq!(
            coordinator.status(&context).await.unwrap(),
            AgentLifecycleStatus::Cancelled
        );
    }
}
