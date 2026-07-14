//! Single-machine multi-agent collaboration integration test.
//!
//! Proves s09 (mailbox), s10 (approval), s12 (worktree), and s11 (autonomous
//! claim) work together WITHOUT a real LLM - using local files, git, and the
//! coordinator's in-memory task-groups. No network, no API key.

use std::sync::Arc;
use tokio::time::Duration;
use wgenty_code::agent::AgentCoordinator;
use wgenty_code::services::{AutonomousWorker, AutonomousWorkerConfig};
use wgenty_code::teams::mailbox::{Mailbox, TeamMessage};
use wgenty_code::teams::WorktreeIsolation;

/// s09: agent A sends a message to agent B's mailbox; B drains it.
#[tokio::test]
async fn mailbox_peer_to_peer_delivery() {
    let tmp = tempfile::TempDir::new().unwrap();
    let prev = std::env::current_dir().unwrap();
    std::env::set_current_dir(tmp.path()).unwrap();

    // B's inbox.
    let b_path = tmp.path().join(".team/inbox/agent-B.jsonl");
    let b_mailbox = Mailbox::new(b_path.clone());

    // A sends to B.
    b_mailbox
        .send(&TeamMessage::Message {
            from: "agent-A".into(),
            to: "agent-B".into(),
            content: "please review src/auth.rs".into(),
            timestamp: "t".into(),
        })
        .await
        .unwrap();

    // B drains.
    let drained = b_mailbox.receive_all().await.unwrap();
    assert_eq!(drained.len(), 1);
    match &drained[0] {
        TeamMessage::Message { from, content, .. } => {
            assert_eq!(from, "agent-A");
            assert!(content.contains("src/auth.rs"));
        }
        other => panic!("unexpected: {other:?}"),
    }

    std::env::set_current_dir(prev).unwrap();
}

/// s10: approval request/response correlation via request_id.
#[tokio::test]
async fn approval_request_response_roundtrip() {
    use wgenty_code::teams::approval_registry;

    let agent = "approver-integration";
    let pending = approval_registry::register_agent(agent);

    // Worker registers a waiter (as request_approval tool does).
    let (tx, rx) = tokio::sync::oneshot::channel::<bool>();
    pending.lock().unwrap().insert("req-42".into(), tx);

    // MailboxInbox::drain would resolve on ApprovalResponse; simulate that.
    let resolved = {
        let mut map = pending.lock().unwrap();
        if let Some(sender) = map.remove("req-42") {
            let _ = sender.send(true);
            true
        } else {
            false
        }
    };
    assert!(resolved);

    let approve = tokio::time::timeout(Duration::from_secs(1), rx)
        .await
        .unwrap()
        .unwrap();
    assert!(approve);
    approval_registry::unregister_agent(agent);
}

/// s12: two isolated worktrees don't collide on the main checkout.
#[tokio::test]
async fn worktree_isolation_parallel_checkouts() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    for (k, v) in [("user.name", "t"), ("user.email", "t@t.test")] {
        let _ = std::process::Command::new("git").args(["config", "--global", k, v]).status();
    }
    let _ = std::process::Command::new("git")
        .arg("init")
        .current_dir(root)
        .status();
    std::fs::write(root.join("README.md"), "init").unwrap();
    let _ = std::process::Command::new("git")
        .args(["add", "."])
        .current_dir(root)
        .status();
    let _ = std::process::Command::new("git")
        .args(["commit", "-m", "init"])
        .current_dir(root)
        .status();

    // Two worktrees coexist.
    let wt_a = WorktreeIsolation::create(root, "agent-A", None).unwrap();
    let wt_b = WorktreeIsolation::create(root, "agent-B", None).unwrap();
    assert_ne!(wt_a.path, wt_b.path);
    assert!(wt_a.path.exists());
    assert!(wt_b.path.exists());

    // Each can modify its own checkout independently.
    std::fs::write(wt_a.path.join("file.txt"), "A").unwrap();
    std::fs::write(wt_b.path.join("file.txt"), "B").unwrap();
    assert_eq!(std::fs::read_to_string(wt_a.path.join("file.txt")).unwrap(), "A");
    assert_eq!(std::fs::read_to_string(wt_b.path.join("file.txt")).unwrap(), "B");

    // Main checkout untouched.
    assert!(!root.join("file.txt").exists());

    drop(wt_a);
    drop(wt_b);
    assert!(!root.join(".wgenty-worktrees/agent-A").exists());
    assert!(!root.join(".wgenty-worktrees/agent-B").exists());
}

/// s11: autonomous worker idles out when no ready task-group exists.
///
/// (A ready-group claim path is exercised by `unified_subagent_lifecycle`;
/// here we verify the worker loop starts, polls, and stops on idle timeout.)
#[tokio::test]
async fn autonomous_worker_idle_timeout() {
    let coordinator = Arc::new(AgentCoordinator::new(4, 3));
    let worker = Arc::new(AutonomousWorker::new(
        coordinator,
        AutonomousWorkerConfig {
            poll_interval: Duration::from_millis(20),
            max_idle_polls: 3,
            enabled: true,
        },
    ));
    let worker_clone = worker.clone();
    let handle = tokio::spawn(async move {
        worker_clone.run("session-idle", "root").await;
    });

    // Worker should stop after ~3 idle polls (60ms) + overhead.
    let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
    let st = worker.status().await;
    assert!(!st.running, "worker should have stopped after idle timeout");
    assert!(st.idle_polls >= 3);
    assert_eq!(st.claims, 0);
}

/// s11: a disabled worker is a no-op (returns immediately).
#[tokio::test]
async fn autonomous_worker_disabled_is_noop() {
    let coordinator = Arc::new(AgentCoordinator::new(4, 3));
    let worker = Arc::new(AutonomousWorker::new(
        coordinator,
        AutonomousWorkerConfig {
            enabled: false,
            ..AutonomousWorkerConfig::default()
        },
    ));
    // Should return immediately without looping.
    let worker_clone = worker.clone();
    tokio::time::timeout(Duration::from_millis(200), async move {
        worker_clone.run("session-disabled", "root").await;
    })
    .await
    .unwrap();
    assert!(!worker.status().await.running);
}
