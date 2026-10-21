//! Regression: daemon streaming run loop must use a coordinator-registered
//! root context.
//!
//! Bug: `RootToolPort::new` built an ad-hoc `AgentExecutionContext::root(...)`
//! with a fresh agent_id that was never registered in the coordinator. When
//! the `task` tool spawned a subagent it looked up the caller's scope by
//! `(session_id, agent_id)`, found nothing, and failed with
//! `parent agent is not running`.
//!
//! Fix: `run_session_turn` now resolves the root context via
//! `DaemonState::root_context` (→ `coordinator.ensure_root`), which registers
//! the scope. This test asserts that the context the daemon produces is
//! visible to the coordinator — i.e. `reserve_child` does not return
//! `ParentNotRunning`.

use crate::daemon_harness::{create_session, spawn_daemon};
use wgenty_code::agent::{CoordinatorError, SpawnChildRequest};

#[tokio::test]
async fn daemon_root_context_is_coordinator_registered() {
    let d = spawn_daemon().await;
    let sid = create_session(&d, "subagent-regression").await;

    // This is exactly the context run_session_turn now feeds into RootToolPort.
    let root_ctx = d
        .state
        .root_context(&sid)
        .await
        .expect("root_context resolves a registered scope");

    // The task tool calls reserve_child with the caller's context. Before the
    // fix this returned ParentNotRunning because the scope was never registered.
    let reservation = d
        .state
        .coordinator
        .reserve_child(&root_ctx, SpawnChildRequest::new("regression probe"))
        .await;

    assert!(
        !matches!(reservation, Err(CoordinatorError::ParentNotRunning)),
        "root_context must be coordinator-registered so subagent dispatch finds the parent scope; got {reservation:?}"
    );
}
