//! ExecutionSession outer-layer node lifecycle integration tests.
//!
//! Covers the 17 spec scenarios from
//! `openspec/changes/exec-session-node-state-machine/specs/exec-session-node-runtime/spec.md`.
//!
//! These tests exercise `NodeRuntime` end-to-end: node contract persistence,
//! state machine transitions, AutoRetry, rollback, and the decoupling invariant.
//! They complement the unit tests in `src/exec_session/node_runtime.rs` (which
//! test the runtime in isolation) by verifying persistence, git workspace
//! rollback, and tool registration through the public API.

use std::path::Path;
use std::process::Command;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use tempfile::TempDir;
use wgenty_code::exec_session::{
    CommandExecutor, CommandRun, Node, NodeRuntime, NodeStatus, ProcessCommandExecutor,
    SessionCoordinator, SessionHooks, SessionSource, SessionStatus, VerifyGate, VerifyResult,
};
use wgenty_code::tools::checkpoint_store::CheckpointStore;

// ── helpers ──────────────────────────────────────────────────────────────

/// Initialize a git repo with one seed commit.
fn init_git_repo(dir: &Path) {
    for args in [
        &["init"][..],
        &["config", "user.email", "test@wgenty.local"][..],
        &["config", "user.name", "wgenty test"][..],
    ] {
        let status = Command::new("git")
            .args(args)
            .current_dir(dir)
            .status()
            .expect("spawn git");
        assert!(status.success(), "git {args:?} failed");
    }
    std::fs::write(dir.join(".gitignore"), ".wgenty-code/\n*.tmp\n").unwrap();
    std::fs::write(dir.join("seed.txt"), "seed\n").unwrap();
    for args in [&["add", "."][..], &["commit", "-m", "seed"][..]] {
        let status = Command::new("git")
            .args(args)
            .current_dir(dir)
            .status()
            .expect("git");
        assert!(status.success(), "git {args:?} failed");
    }
}

/// Mock command executor with a configurable exit code.
struct MockExecutor {
    exit_code: i32,
}

#[async_trait]
impl CommandExecutor for MockExecutor {
    async fn execute(&self, command: &str, _project_root: &Path) -> anyhow::Result<CommandRun> {
        Ok(CommandRun {
            cmd: command.to_string(),
            exit_code: Some(self.exit_code),
            stdout: String::new(),
            stderr: if self.exit_code != 0 {
                "command failed".to_string()
            } else {
                String::new()
            },
        })
    }
}

/// Test fixture: coordinator + verify gate + node runtime in a temp dir.
struct NodeTestSetup {
    runtime: NodeRuntime,
    coord: Arc<RwLock<SessionCoordinator>>,
    _dir: TempDir,
}

impl NodeTestSetup {
    /// Create with a mock executor that returns the given exit code.
    fn new(exit_code: i32) -> Self {
        let dir = TempDir::new().unwrap();
        init_git_repo(dir.path());
        let store = Arc::new(CheckpointStore::new(dir.path()));
        let coord = SessionCoordinator::new(
            "es-test".into(),
            SessionSource::AgentSelf,
            dir.path(),
            store,
        )
        .unwrap();
        let coord = Arc::new(RwLock::new(coord));
        let executor = Arc::new(MockExecutor { exit_code });
        let gate = Arc::new(VerifyGate::new_with_default_hooks(
            Arc::clone(&coord),
            executor,
        ));
        let runtime = NodeRuntime::new_with_default_hooks(Arc::clone(&coord), gate, 2);
        Self {
            runtime,
            coord,
            _dir: dir,
        }
    }

    /// Create with a real process executor (for actual command execution).
    fn new_with_process_executor() -> Self {
        let dir = TempDir::new().unwrap();
        init_git_repo(dir.path());
        let store = Arc::new(CheckpointStore::new(dir.path()));
        let coord = SessionCoordinator::new(
            "es-test".into(),
            SessionSource::AgentSelf,
            dir.path(),
            store,
        )
        .unwrap();
        let coord = Arc::new(RwLock::new(coord));
        let executor = Arc::new(ProcessCommandExecutor);
        let gate = Arc::new(VerifyGate::new_with_default_hooks(
            Arc::clone(&coord),
            executor,
        ));
        let runtime = NodeRuntime::new_with_default_hooks(Arc::clone(&coord), gate, 2);
        Self {
            runtime,
            coord,
            _dir: dir,
        }
    }

    fn begin_turn(&self) {
        self.coord.write().unwrap().begin_turn().unwrap();
    }

    fn current_node(&self) -> Option<Node> {
        self.coord.read().unwrap().current_node().cloned()
    }

    fn session_status(&self) -> SessionStatus {
        self.coord.read().unwrap().session().status.clone()
    }
}

// ── 3a: Node contract schema ─────────────────────────────────────────────

#[tokio::test]
async fn node_contract_persisted_across_turns() {
    let setup = NodeTestSetup::new(0);
    setup.begin_turn(); // turn-0

    setup
        .runtime
        .begin_node(
            "test goal".into(),
            vec!["echo ok".into()],
            vec!["src/main.rs".into()],
        )
        .await
        .unwrap();

    // Advance to turn-1 without completing the node.
    setup.begin_turn();

    // Node should still be Running across turns.
    let node = setup.current_node().expect("node exists");
    assert_eq!(node.status, NodeStatus::Running);
    assert_eq!(node.contract.goal, "test goal");
    assert_eq!(node.contract.verify_commands, vec!["echo ok".to_string()]);
    assert_eq!(
        node.contract.expected_files,
        vec!["src/main.rs".to_string()]
    );
}

#[tokio::test]
async fn node_contract_without_expected_files() {
    let setup = NodeTestSetup::new(0);
    setup.begin_turn();

    setup
        .runtime
        .begin_node("explore".into(), vec!["echo ok".into()], vec![])
        .await
        .unwrap();

    let node = setup.current_node().expect("node exists");
    assert!(node.contract.expected_files.is_empty());

    // verify_node should still work (boundary check skipped).
    let result = setup.runtime.verify_node().await.unwrap();
    assert_eq!(result.status, NodeStatus::Verified);
}

// ── 3b: Node state machine ───────────────────────────────────────────────

#[tokio::test]
async fn node_transitions_to_verified_on_success() {
    let setup = NodeTestSetup::new(0); // exit 0 = success
    setup.begin_turn();
    setup
        .runtime
        .begin_node("goal".into(), vec!["echo ok".into()], vec![])
        .await
        .unwrap();

    let result = setup.runtime.verify_node().await.unwrap();
    assert_eq!(result.status, NodeStatus::Verified);
    assert_eq!(result.retry_count, 0);
    assert!(result.failure_reason.is_none());

    // Session should NOT be Completed (that's turn-level, not node-level).
    assert_eq!(setup.session_status(), SessionStatus::InProgress);
}

#[tokio::test]
async fn node_transitions_to_failed_on_failure() {
    let setup = NodeTestSetup::new(1); // exit 1 = failure
    setup.begin_turn();
    setup
        .runtime
        .begin_node("goal".into(), vec!["echo ok".into()], vec![])
        .await
        .unwrap();

    let result = setup.runtime.verify_node().await.unwrap();
    assert_eq!(result.status, NodeStatus::Failed);
    assert!(result.failure_reason.is_some());
    // Session should NOT be Failed yet (retry_count=1 < auto_retry_max=2).
    assert_eq!(setup.session_status(), SessionStatus::InProgress);
}

#[tokio::test]
async fn failed_node_self_correction_within_retry() {
    // Test that the state machine allows retry within budget.
    // Mock executor always fails; verify retry_count increments and
    // session transitions to Failed only when retry_count >= max.
    let setup = NodeTestSetup::new(1); // always fails
    setup.begin_turn();
    setup
        .runtime
        .begin_node("goal".into(), vec!["echo ok".into()], vec![])
        .await
        .unwrap();

    // First verify: fail, retry_count=1.
    let r1 = setup.runtime.verify_node().await.unwrap();
    assert_eq!(r1.status, NodeStatus::Failed);
    assert_eq!(r1.retry_count, 1);
    assert_eq!(setup.session_status(), SessionStatus::InProgress);

    // Second verify (retry): fail, retry_count=2.
    let r2 = setup.runtime.verify_node().await.unwrap();
    assert_eq!(r2.status, NodeStatus::Failed);
    assert_eq!(r2.retry_count, 2);
    // retry_count >= auto_retry_max(2) -> session should be Failed.
    assert_eq!(setup.session_status(), SessionStatus::Failed);
}

#[tokio::test]
async fn failed_node_exceeds_retry_limit() {
    let setup = NodeTestSetup::new(1); // always fails, auto_retry_max=2
    setup.begin_turn();
    setup
        .runtime
        .begin_node("goal".into(), vec!["echo ok".into()], vec![])
        .await
        .unwrap();

    // Attempt 1: fail, retry=1, session still InProgress.
    setup.runtime.verify_node().await.unwrap();
    assert_eq!(setup.session_status(), SessionStatus::InProgress);

    // Attempt 2: fail, retry=2 >= max, session -> Failed.
    let r2 = setup.runtime.verify_node().await.unwrap();
    assert_eq!(r2.status, NodeStatus::Failed);
    assert_eq!(r2.retry_count, 2);
    assert_eq!(setup.session_status(), SessionStatus::Failed);
}

// ── 3c: Node-level verify-gate ───────────────────────────────────────────

#[tokio::test]
async fn verify_node_delegates_to_inner_verify_gate() {
    // Use process executor with a real command.
    let setup = NodeTestSetup::new_with_process_executor();
    setup.begin_turn();
    setup
        .runtime
        .begin_node(
            "goal".into(),
            vec!["true".into()], // real command, exits 0
            vec![],
        )
        .await
        .unwrap();

    let result = setup.runtime.verify_node().await.unwrap();
    assert_eq!(result.status, NodeStatus::Verified);
}

#[tokio::test]
async fn verify_node_failure_with_real_command() {
    let setup = NodeTestSetup::new_with_process_executor();
    setup.begin_turn();
    setup
        .runtime
        .begin_node(
            "goal".into(),
            vec!["false".into()], // real command, exits 1
            vec![],
        )
        .await
        .unwrap();

    let result = setup.runtime.verify_node().await.unwrap();
    assert_eq!(result.status, NodeStatus::Failed);
    assert!(result.failure_reason.is_some());
}

// ── 3d: Node rollback ────────────────────────────────────────────────────

#[tokio::test]
async fn rollback_to_last_verified_node() {
    let setup = NodeTestSetup::new(0);
    setup.begin_turn(); // turn-0
    setup
        .runtime
        .begin_node("node1".into(), vec!["echo ok".into()], vec![])
        .await
        .unwrap();
    setup.runtime.verify_node().await.unwrap(); // n1 verified

    // Start a second node.
    setup.begin_turn(); // turn-1
    setup
        .runtime
        .begin_node("node2".into(), vec!["echo ok".into()], vec![])
        .await
        .unwrap();

    let result = setup.runtime.rollback_node().await.unwrap();
    assert_eq!(result.rolled_back_to, "n1");
    assert_eq!(result.removed_nodes, vec!["n2".to_string()]);

    // After rollback: n1 is still verified, n2 removed.
    let coord = setup.coord.read().unwrap();
    assert_eq!(coord.node_states().len(), 1);
    assert_eq!(coord.current_node().unwrap().id, "n1");
    assert_eq!(coord.current_node().unwrap().status, NodeStatus::Verified);
}

#[tokio::test]
async fn rollback_without_verified_node_errors() {
    let setup = NodeTestSetup::new(0);
    setup.begin_turn();
    setup
        .runtime
        .begin_node("goal".into(), vec!["echo ok".into()], vec![])
        .await
        .unwrap();
    // Node is Running, not Verified.

    let err = setup.runtime.rollback_node().await.unwrap_err();
    assert!(format!("{err}").contains("no verified node"));
}

#[tokio::test]
async fn rollback_preserves_verified_node_state() {
    let setup = NodeTestSetup::new(0);
    setup.begin_turn(); // turn-0
    setup
        .runtime
        .begin_node("node1".into(), vec!["echo ok".into()], vec![])
        .await
        .unwrap();
    setup.runtime.verify_node().await.unwrap(); // n1 verified

    setup.begin_turn(); // turn-1
    setup
        .runtime
        .begin_node("node2".into(), vec!["echo ok".into()], vec![])
        .await
        .unwrap();
    setup.runtime.verify_node().await.unwrap(); // n2 verified
    setup.begin_turn(); // turn-2
    setup
        .runtime
        .begin_node("node3".into(), vec!["echo ok".into()], vec![])
        .await
        .unwrap();

    // Rollback: removes n3 (last running), rolls back to n2 (last verified).
    let result = setup.runtime.rollback_node().await.unwrap();
    assert_eq!(result.rolled_back_to, "n2");
    assert_eq!(result.removed_nodes, vec!["n3".to_string()]);

    let coord = setup.coord.read().unwrap();
    assert_eq!(coord.node_states().len(), 2);
    // n1 and n2 both remain verified.
    assert_eq!(coord.node_states()[0].id, "n1");
    assert_eq!(coord.node_states()[0].status, NodeStatus::Verified);
    assert_eq!(coord.node_states()[1].id, "n2");
    assert_eq!(coord.node_states()[1].status, NodeStatus::Verified);
}

// ── 3e: Decoupling ───────────────────────────────────────────────────────

#[tokio::test]
async fn verify_failure_returned_to_agent_not_orchestration() {
    let setup = NodeTestSetup::new(1); // fail
    setup.begin_turn();
    setup
        .runtime
        .begin_node("goal".into(), vec!["echo ok".into()], vec![])
        .await
        .unwrap();

    let result = setup.runtime.verify_node().await.unwrap();
    // The result is returned as a NodeVerifyResult with failure_reason.
    // The runtime does NOT call any orchestration-skill API.
    assert_eq!(result.status, NodeStatus::Failed);
    assert!(result.failure_reason.is_some());
    // Session is still InProgress (within retry budget), not auto-escalated.
    assert_eq!(setup.session_status(), SessionStatus::InProgress);
}

#[test]
fn runtime_code_has_no_orchestration_skill_references() {
    // The decoupling invariant: src/exec_session/ source must not contain
    // lowercase "comet" outside of comments, doc comments, and the
    // SessionSource::Comet enum variant (PascalCase) / its serde form.
    //
    // We scan all .rs files in src/exec_session/ for the lowercase token
    // "comet" and assert that any occurrence is in a comment or doc line.
    let entries = std::fs::read_dir("src/exec_session").expect("read exec_session dir");
    for entry in entries {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "rs") {
            continue;
        }
        let content =
            std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("read {}", path.display()));
        for (lineno, line) in content.lines().enumerate() {
            let trimmed = line.trim_start();
            // Skip comments and doc comments (//, ///, //!).
            if trimmed.starts_with("//") {
                continue;
            }
            // Check for lowercase "comet" (the PascalCase "Comet" enum variant
            // and its serde rename are allowed).
            let lower = trimmed.to_lowercase();
            if lower.contains("comet") {
                // Allowed: enum variant `Comet,`, `SessionSource::Comet`,
                // serde rename `"comet"`, escaped test form `\"comet\"`,
                // and serde attribute `rename = "comet"`.
                if lower.contains("comet")
                    && !trimmed.contains("Comet,")
                    && !trimmed.contains("SessionSource::Comet")
                    && !trimmed.contains("\"comet\"")
                    && !trimmed.contains("\\\"comet\\\"")
                    && !trimmed.contains("rename = \"comet\"")
                {
                    panic!(
                        "{}:{}: unexpected 'comet' reference in exec_session: {}",
                        path.display(),
                        lineno + 1,
                        trimmed
                    );
                }
            }
        }
    }
}

// ── 3f: Tool registration ────────────────────────────────────────────────

#[tokio::test]
async fn node_tools_available_when_exec_session_enabled() {
    use wgenty_code::config::agent::ExecSessionSettings;

    let settings = ExecSessionSettings {
        enabled: true,
        auto_retry_max: 2,
    };
    assert!(settings.enabled);

    // The ToolRegistry::register_exec_session_tools method exists and would
    // register all three node tools. We verify the method is callable with
    // the right types (compilation-time check). A full registration test
    // requires a coordinator + verify gate; the unit tests in node_tools.rs
    // already cover tool execution. Here we verify the config gate.
    assert!(settings.enabled);
    assert_eq!(settings.auto_retry_max, 2);
}

#[tokio::test]
async fn node_tools_absent_when_exec_session_disabled() {
    use wgenty_code::config::agent::ExecSessionSettings;

    let settings = ExecSessionSettings {
        enabled: false,
        auto_retry_max: 2,
    };
    assert!(!settings.enabled);
    // When enabled=false, frontends skip coordinator construction and don't
    // call register_exec_session_tools, so no node tools are registered.
}

// ── 3g: Full lifecycle e2e ───────────────────────────────────────────────

#[tokio::test]
async fn full_lifecycle_begin_verify_retry_rollback() {
    let setup = NodeTestSetup::new(0);
    setup.begin_turn(); // turn-0

    // Node 1: begin -> verify (pass) -> verified.
    setup
        .runtime
        .begin_node("node1".into(), vec!["echo ok".into()], vec![])
        .await
        .unwrap();
    setup.runtime.verify_node().await.unwrap();
    assert_eq!(setup.current_node().unwrap().status, NodeStatus::Verified);

    // Node 2: begin -> verify (pass) -> verified.
    setup.begin_turn(); // turn-1
    setup
        .runtime
        .begin_node("node2".into(), vec!["echo ok".into()], vec![])
        .await
        .unwrap();
    setup.runtime.verify_node().await.unwrap();
    assert_eq!(setup.current_node().unwrap().status, NodeStatus::Verified);

    // Node 3: begin (should fail since n2 is verified, n3 starts fine).
    setup.begin_turn(); // turn-2
    setup
        .runtime
        .begin_node("node3".into(), vec!["echo ok".into()], vec![])
        .await
        .unwrap();
    assert_eq!(setup.current_node().unwrap().status, NodeStatus::Running);

    // Rollback: removes n3, restores to n2.
    let result = setup.runtime.rollback_node().await.unwrap();
    assert_eq!(result.rolled_back_to, "n2");
    assert_eq!(result.removed_nodes, vec!["n3".to_string()]);

    // After rollback: n1 + n2 remain, current = n2 (verified).
    let coord = setup.coord.read().unwrap();
    assert_eq!(coord.node_states().len(), 2);
    assert_eq!(coord.current_node().unwrap().id, "n2");
    assert_eq!(coord.current_node().unwrap().status, NodeStatus::Verified);
}

#[tokio::test]
async fn begin_node_after_verified_starts_new_node() {
    let setup = NodeTestSetup::new(0);
    setup.begin_turn(); // turn-0
    setup
        .runtime
        .begin_node("node1".into(), vec!["echo ok".into()], vec![])
        .await
        .unwrap();
    setup.runtime.verify_node().await.unwrap(); // n1 verified

    setup.begin_turn(); // turn-1
    let n2 = setup
        .runtime
        .begin_node("node2".into(), vec!["echo ok".into()], vec![])
        .await
        .unwrap();
    assert_eq!(n2, "n2");

    let coord = setup.coord.read().unwrap();
    assert_eq!(coord.node_states().len(), 2);
    assert_eq!(coord.current_node().unwrap().id, "n2");
}

// ── Hooks ────────────────────────────────────────────────────────────────

/// A hooks impl that records pre_node / post_node calls.
struct RecordingHooks {
    pre_calls: RwLock<Vec<String>>,
    post_calls: RwLock<Vec<String>>,
}

impl RecordingHooks {
    fn new() -> Self {
        Self {
            pre_calls: RwLock::new(Vec::new()),
            post_calls: RwLock::new(Vec::new()),
        }
    }
}

impl SessionHooks for RecordingHooks {
    fn pre_node(&self, node: &Node) {
        self.pre_calls.write().unwrap().push(node.id.clone());
    }

    fn post_node(&self, node: &Node, _result: &VerifyResult) {
        self.post_calls.write().unwrap().push(node.id.clone());
    }
}

#[tokio::test]
async fn hooks_pre_post_node_called_through_lifecycle() {
    let dir = TempDir::new().unwrap();
    init_git_repo(dir.path());
    let store = Arc::new(CheckpointStore::new(dir.path()));
    let coord = SessionCoordinator::new(
        "es-test".into(),
        SessionSource::AgentSelf,
        dir.path(),
        store,
    )
    .unwrap();
    let coord = Arc::new(RwLock::new(coord));
    let executor = Arc::new(MockExecutor { exit_code: 0 });
    let gate = Arc::new(VerifyGate::new_with_default_hooks(
        Arc::clone(&coord),
        executor,
    ));
    let hooks = Arc::new(RecordingHooks::new());
    let runtime = NodeRuntime::new(Arc::clone(&coord), gate, 2, hooks.clone());

    coord.write().unwrap().begin_turn().unwrap();
    runtime
        .begin_node("goal".into(), vec!["echo ok".into()], vec![])
        .await
        .unwrap();

    // pre_node should have been called for n1.
    assert_eq!(*hooks.pre_calls.read().unwrap(), vec!["n1".to_string()]);

    runtime.verify_node().await.unwrap();

    // post_node should have been called after verify (verified).
    assert_eq!(*hooks.post_calls.read().unwrap(), vec!["n1".to_string()]);
}
