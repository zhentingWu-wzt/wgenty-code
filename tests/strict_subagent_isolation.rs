//! End-to-end contracts for strict subagent isolation at the daemon boundary.
//!
//! These tests exercise the scoped agent APIs (viewer tokens, capability-bound
//! navigation, transcript, cancel) against a seeded coordinator without
//! booting the full HTTP server, so they run without live settings or a
//! model endpoint.

use wgenty_code::agent::capability::{CapabilityGrant, CapabilityRequest, CapabilityService};
use wgenty_code::agent::{AgentCoordinator, AgentExecutionContext, SessionId, SpawnChildRequest};

/// Seeds a three-level tree: root -> child -> grandchild, plus a sibling of
/// `child` and a separate session root. Returns the contexts so tests can
/// assert visibility from each vantage point.
struct SeededTree {
    coordinator: AgentCoordinator,
    root: AgentExecutionContext,
    child: AgentExecutionContext,
    grandchild: AgentExecutionContext,
    sibling: AgentExecutionContext,
    other_root: AgentExecutionContext,
}

async fn seed() -> SeededTree {
    let coordinator = AgentCoordinator::new(8, 4);
    let root = coordinator.ensure_root(SessionId::new("s")).await.unwrap();
    let child = coordinator
        .reserve_child(&root, SpawnChildRequest::new("child"))
        .await
        .unwrap()
        .context;
    let grandchild = coordinator
        .reserve_child(&child, SpawnChildRequest::new("grandchild"))
        .await
        .unwrap()
        .context;
    let sibling = coordinator
        .reserve_child(&root, SpawnChildRequest::new("sibling"))
        .await
        .unwrap()
        .context;
    let other_root = coordinator
        .ensure_root(SessionId::new("other"))
        .await
        .unwrap();
    SeededTree {
        coordinator,
        root,
        child,
        grandchild,
        sibling,
        other_root,
    }
}

#[tokio::test]
async fn root_local_view_contains_self_and_direct_children_only() {
    let tree = seed().await;
    let view = tree.coordinator.list_local(&tree.root).await.unwrap();
    assert_eq!(view.self_view.agent_id, tree.root.agent_id);
    let child_ids: Vec<&str> = view.children.iter().map(|c| c.agent_id.as_str()).collect();
    assert!(child_ids.contains(&tree.child.agent_id.as_str()));
    assert!(child_ids.contains(&tree.sibling.agent_id.as_str()));
    // Grandchild must not appear in root's view.
    assert!(!child_ids.contains(&tree.grandchild.agent_id.as_str()));
}

#[tokio::test]
async fn navigation_capability_verifies_only_for_bound_target() {
    let tree = seed().await;
    let service = CapabilityService::new([7; 32]);
    let viewer = "viewer-a";

    // Issue a navigate capability for `child`.
    let grant = CapabilityGrant::navigate(viewer, "s", tree.child.agent_id.as_str(), 0);
    let token = service.issue(&grant).await;

    // Correct request verifies.
    assert!(service
        .verify(
            &token,
            &CapabilityRequest::navigate(viewer, "s", tree.child.agent_id.as_str(), 0)
        )
        .await
        .is_ok());

    // Sibling, grandchild, other-session root, and missing are all NotVisible.
    for target in [
        tree.sibling.agent_id.as_str(),
        tree.grandchild.agent_id.as_str(),
        tree.other_root.agent_id.as_str(),
        "missing",
    ] {
        assert!(service
            .verify(&token, &CapabilityRequest::navigate(viewer, "s", target, 0))
            .await
            .is_err());
    }
    // A random/unknown capability is also denied (indistinguishable).
    assert!(service
        .verify(
            "not-a-real-capability",
            &CapabilityRequest::navigate(viewer, "s", tree.child.agent_id.as_str(), 0)
        )
        .await
        .is_err());
}

#[tokio::test]
async fn wrong_viewer_cannot_use_capability() {
    let tree = seed().await;
    let service = CapabilityService::new([7; 32]);
    let token = service
        .issue(&CapabilityGrant::navigate(
            "viewer-a",
            "s",
            tree.child.agent_id.as_str(),
            0,
        ))
        .await;
    // A different viewer is denied.
    assert!(service
        .verify(
            &token,
            &CapabilityRequest::navigate("viewer-b", "s", tree.child.agent_id.as_str(), 0)
        )
        .await
        .is_err());
}

#[tokio::test]
async fn cancel_subtree_authorizes_direct_child_only() {
    use wgenty_code::agent::CoordinatorError;
    let tree = seed().await;
    // Root can cancel its direct child.
    assert!(tree
        .coordinator
        .cancel_subtree(&tree.root, tree.child.agent_id.clone())
        .await
        .is_ok());
    // Root cannot cancel the grandchild directly (not a direct child).
    assert!(matches!(
        tree.coordinator
            .cancel_subtree(&tree.root, tree.grandchild.agent_id.clone())
            .await,
        Err(CoordinatorError::NotVisible)
    ));
    // Root cannot cancel an agent in another session.
    assert!(matches!(
        tree.coordinator
            .cancel_subtree(&tree.root, tree.other_root.agent_id.clone())
            .await,
        Err(CoordinatorError::NotVisible)
    ));
}

#[tokio::test]
async fn transcript_and_result_reads_require_direct_visibility() {
    let tree = seed().await;
    // Root reads child transcript/result: ok (direct child).
    assert!(tree
        .coordinator
        .read_transcript(&tree.root, tree.child.agent_id.clone())
        .await
        .is_ok());
    assert!(tree
        .coordinator
        .read_status(&tree.root, tree.child.agent_id.clone())
        .await
        .is_ok());
    // Root cannot read grandchild (hidden).
    assert!(tree
        .coordinator
        .read_transcript(&tree.root, tree.grandchild.agent_id.clone())
        .await
        .is_err());
    // Child can read its own grandchild (direct child of child).
    assert!(tree
        .coordinator
        .read_status(&tree.child, tree.grandchild.agent_id.clone())
        .await
        .is_ok());
    // Child cannot read its sibling (hidden from child).
    assert!(tree
        .coordinator
        .read_status(&tree.child, tree.sibling.agent_id.clone())
        .await
        .is_err());
}

#[test]
fn compatibility_guard_no_reserved_identity_reads() {
    // Scan identity-sensitive source files for model-input reads of
    // `_session_id`, `_agent_id`, `_parent_id`, or `_subagent_depth`.
    // These fields must never influence identity, authorization, depth, or
    // cancellation.  Comment-only mentions are fine.
    use std::fs;
    let files = &[
        "src/tools/meta/task.rs",
        "src/tools/meta/rlm/mod.rs",
        "src/tools/meta/run_script.rs",
        "src/daemon/handlers.rs",
        "src/teams/subagent_loop.rs",
    ];
    for file in files {
        let content = match fs::read_to_string(file) {
            Ok(c) => c,
            Err(_) => continue,
        };
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("//") || trimmed.starts_with("*") {
                continue;
            }
            // Model input reads for these fields are illegal.
            assert!(
                !trimmed.contains("_session_id\"")
                    && !trimmed.contains("_agent_id\"")
                    && !trimmed.contains("_parent_id\"")
                    && !trimmed.contains("_subagent_depth\""),
                "{} has reserved-field read: {}",
                file,
                trimmed
            );
        }
    }
}
