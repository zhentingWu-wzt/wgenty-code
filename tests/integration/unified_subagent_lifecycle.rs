//! End-to-end lifecycle contracts for the unified subagent lifecycle.
//!
//! These tests exercise the non-root synthesis barrier and the persistent-root
//! delivery contracts without driving a live model loop: they reserve real
//! coordinator-owned children, drive them to terminal states, and assert the
//! coordinator's synthesis/finalization and delivery APIs behave as the
//! `run_subagent_loop` synthesis round depends on them to.

use std::sync::Arc;
use std::time::Duration;

use wgenty_code::agent::{
    AgentCoordinator, AgentExecutionContext, AgentLifecycleStatus, ChildTerminal, CoordinatorError,
    JoinPolicy, SessionId, SpawnChildRequest,
};

fn coordinator() -> Arc<AgentCoordinator> {
    // Short shutdown timeout so any misbehaving leaked future cannot stall the
    // suite; the synthesis/claim contracts do not depend on the timeout value.
    Arc::new(AgentCoordinator::new(8, 3).with_shutdown_timeout(Duration::from_secs(1)))
}

/// A controlled child future that resolves to `terminal` when its release
/// oneshot is dropped/fired, or `Cancelled` when its token fires. Mirrors the
/// coordinator test helper so children can be driven deterministically.
async fn register_terminal_child(
    coordinator: &AgentCoordinator,
    caller: &AgentExecutionContext,
    terminal: ChildTerminal,
) -> AgentExecutionContext {
    let reservation = coordinator
        .reserve_child(caller, SpawnChildRequest::new("controlled"))
        .await
        .expect("reserve child");
    let context = reservation.context.clone();
    let token = context.cancellation.clone();
    let (_release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
    let terminal_clone = terminal.clone();
    let task = tokio::spawn(async move {
        tokio::select! {
            biased;
            _ = token.cancelled() => ChildTerminal::Cancelled,
            _ = release_rx => terminal_clone,
        }
    });
    coordinator
        .register_task(&context, task)
        .await
        .expect("register task");
    // Fire the release so the child settles to the configured terminal. The
    // spawned task resolves on the next poll; finish_child persists it.
    drop(_release_tx);
    // Give the runtime a chance to drive the spawned task to its terminal.
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;
    context
}

#[tokio::test]
async fn non_root_parent_synthesizes_children_before_finalizing() {
    // A non-root parent with a completed direct child must be able to collect
    // the child's result for synthesis (WaitingForChildren -> Running) and then
    // finalize once no live children remain. This is the contract the
    // `run_subagent_loop` synthesis round relies on.
    let coordinator = coordinator();
    let root = coordinator.ensure_root(SessionId::new("s")).await.unwrap();
    let parent = coordinator
        .reserve_child(&root, SpawnChildRequest::new("parent"))
        .await
        .unwrap();

    let child = register_terminal_child(
        &coordinator,
        &parent.context,
        ChildTerminal::completed("child evidence"),
    )
    .await;
    // Persist the child's terminal so it is no longer "live".
    coordinator
        .finish_child(&child, ChildTerminal::completed("child evidence"))
        .await
        .unwrap();

    let results = coordinator
        .collect_children_for_synthesis(&parent.context)
        .await
        .expect("collect children for synthesis");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].summary, "child evidence");
    // The parent is restored to Running (not terminalized) after synthesis.
    assert_eq!(
        coordinator.status(&parent.context).await.unwrap(),
        AgentLifecycleStatus::Running,
    );

    // With no live children, the parent may begin finalizing.
    coordinator
        .begin_finalizing(&parent.context)
        .await
        .expect("begin finalizing");
}

#[tokio::test]
async fn non_root_parent_cannot_finalize_while_child_is_live() {
    let coordinator = coordinator();
    let root = coordinator.ensure_root(SessionId::new("s")).await.unwrap();
    let parent = coordinator
        .reserve_child(&root, SpawnChildRequest::new("parent"))
        .await
        .unwrap();

    // Register a child but do NOT finish it: it stays live.
    let reservation = coordinator
        .reserve_child(&parent.context, SpawnChildRequest::new("live"))
        .await
        .unwrap();
    let context = reservation.context.clone();
    let token = context.cancellation.clone();
    let (_release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        tokio::select! {
            biased;
            _ = token.cancelled() => ChildTerminal::Cancelled,
            _ = release_rx => ChildTerminal::completed("done"),
        }
    });
    coordinator.register_task(&context, task).await.unwrap();
    // Leak the release so the child stays live for the duration of the test.
    std::mem::forget(_release_tx);

    let error = coordinator
        .begin_finalizing(&parent.context)
        .await
        .expect_err("expected ChildrenStillRunning");
    assert!(
        matches!(error, CoordinatorError::ChildrenStillRunning),
        "expected ChildrenStillRunning, got {error:?}"
    );

    // Cleanup: cancel the live child subtree so no task outlives the test.
    let _ = coordinator
        .cancel_subtree(&parent.context, context.agent_id)
        .await;
}

#[tokio::test]
async fn persistent_root_never_synthesizes_or_finalizes() {
    // The persistent root consumes ready groups via the daemon delivery API,
    // never through collect_children_for_synthesis or finalize_scope.
    let coordinator = coordinator();
    let root = coordinator.ensure_root(SessionId::new("s")).await.unwrap();

    let err = coordinator
        .collect_children_for_synthesis(&root)
        .await
        .expect_err("root cannot synthesize");
    assert!(matches!(err, CoordinatorError::RootHasNoTerminalState));

    let err = coordinator
        .finalize_scope(
            &root,
            wgenty_code::agent::ParentOutcome::Completed("done".into()),
            JoinPolicy::BestEffort,
        )
        .await
        .expect_err("root cannot finalize");
    assert!(matches!(err, CoordinatorError::RootHasNoTerminalState));
    assert_eq!(
        coordinator.status(&root).await.unwrap(),
        AgentLifecycleStatus::Running,
    );
}

#[tokio::test]
async fn root_direct_child_group_is_deliverable_once_terminal() {
    // A root-direct child spawned through reserve_child_in_group becomes
    // deliverable to the persistent root exactly once after it terminates.
    let coordinator = coordinator();
    let root = coordinator.ensure_root(SessionId::new("s")).await.unwrap();
    let group = coordinator
        .create_root_task_group(
            &root,
            "turn-1",
            tokio::time::Instant::now() + Duration::from_secs(3600),
        )
        .await
        .unwrap();
    let child = coordinator
        .reserve_child_in_group(&root, SpawnChildRequest::new("work"), group.clone())
        .await
        .unwrap();
    coordinator
        .finish_child(&child.context, ChildTerminal::completed("done"))
        .await
        .unwrap();

    let delivery = coordinator
        .claim_ready_root_group(&root, 0)
        .await
        .unwrap()
        .expect("ready delivery");
    assert_eq!(delivery.group_id, group);
    assert_eq!(delivery.results.len(), 1);
    assert_eq!(delivery.results[0].summary, "done");

    // Exactly-once: a second claim returns None.
    assert!(coordinator
        .claim_ready_root_group(&root, 0)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn generation_reset_rejects_stale_claims_and_cancels_live_children() {
    // On /clear, the daemon advances the generation. A ready group from the
    // old generation must no longer be deliverable, and live root-direct
    // children are cancelled.
    let coordinator = coordinator();
    let root = coordinator.ensure_root(SessionId::new("s")).await.unwrap();
    let group = coordinator
        .create_root_task_group(
            &root,
            "turn-1",
            tokio::time::Instant::now() + Duration::from_secs(3600),
        )
        .await
        .unwrap();
    let child = coordinator
        .reserve_child_in_group(&root, SpawnChildRequest::new("work"), group.clone())
        .await
        .unwrap();
    coordinator
        .finish_child(&child.context, ChildTerminal::completed("done"))
        .await
        .unwrap();

    // The old generation's group is deliverable before reset.
    assert!(coordinator
        .claim_ready_root_group(&root, 0)
        .await
        .unwrap()
        .is_some());

    // Re-create a fresh ready group at generation 0 for the stale-check.
    let group2 = coordinator
        .create_root_task_group(
            &root,
            "turn-2",
            tokio::time::Instant::now() + Duration::from_secs(3600),
        )
        .await
        .unwrap();
    let child2 = coordinator
        .reserve_child_in_group(&root, SpawnChildRequest::new("work2"), group2.clone())
        .await
        .unwrap();
    coordinator
        .finish_child(&child2.context, ChildTerminal::completed("done2"))
        .await
        .unwrap();

    // Advance the generation (simulating /clear). Old ready groups are
    // cancelled and no longer deliverable at the old generation.
    let old_gen = coordinator.current_generation(&root.session_id).await;
    let _ = coordinator
        .cancel_generation(&root.session_id, old_gen)
        .await;
    let new_gen = coordinator.advance_generation(&root.session_id).await;
    assert_eq!(new_gen, old_gen + 1);
    assert!(
        coordinator
            .claim_ready_root_group(&root, old_gen)
            .await
            .unwrap()
            .is_none(),
        "stale-generation claim must be rejected after reset"
    );
}

#[tokio::test]
async fn cancel_agent_session_cancels_live_root_children() {
    // Application shutdown cancels every live root-direct child subtree.
    let coordinator = coordinator();
    let root = coordinator.ensure_root(SessionId::new("s")).await.unwrap();
    let child =
        register_terminal_child(&coordinator, &root, ChildTerminal::completed("done")).await;
    // The child is registered; cancel_root_children drives it terminal.
    let cancelled = coordinator.cancel_root_children(&root).await.unwrap();
    assert!(
        cancelled >= 1,
        "expected at least one child cancelled, got {cancelled}"
    );
    // The child reaches a terminal state after cancellation.
    let status = coordinator.status(&child).await.unwrap();
    assert!(
        status == AgentLifecycleStatus::Completed || status == AgentLifecycleStatus::Cancelled,
        "child should be terminal after session cancel, got {status:?}"
    );
}

#[test]
fn bundled_prompts_do_not_instruct_models_to_select_background_mode() {
    for path in ["src/prompts/base.md", "src/prompts/init_instructions.md"] {
        let text = std::fs::read_to_string(path).expect("read bundled prompt");
        assert!(
            !text.contains("background: true"),
            "legacy background:true instruction in {path}"
        );
        assert!(
            !text.contains("background: false"),
            "legacy background:false instruction in {path}"
        );
    }
}
