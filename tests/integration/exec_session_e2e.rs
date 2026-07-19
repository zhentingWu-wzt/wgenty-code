//! ExecutionSession inner-layer end-to-end tests (Task 8).
//!
//! Drives the full closed loop that the unit tests only exercise in pieces:
//! turn chaining + git refs + checkpoint capture + verify-gate + rollback +
//! unverified fallback, all in one flow per test. Also enforces the two
//! cross-cutting invariants the spec calls out:
//!
//! - **Decoupling (8.8):** `src/exec_session/` source contains no lowercase
//!   `comet` token outside comments / string literals. The `Comet` enum
//!   variant (PascalCase) and its kebab-case serde form (`"comet"`) are the
//!   only allowed occurrences.
//! - **Crash consistency (8.7):** a stale `session.json.tmp` left by a crash
//!   never corrupts `SessionState::load` - the committed `session.json` is the
//!   sole source of truth.
//!
//! These complement the per-module unit tests in `src/exec_session/` (which
//! cover each stage in isolation) and the agent-loop integration tests in
//! `src/agent/runtime/loop_tests.rs` (Task 7, which cover the turn-boundary
//! hook). The value here is realistic multi-stage flows over a real git repo.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, RwLock};

use tempfile::TempDir;
use wgenty_code::exec_session::{
    NoHooks, ProcessCommandExecutor, SessionCoordinator, SessionSource, SessionStatus,
    UnverifiedOutcome, VerifyFailAction, VerifyFailure, VerifyGate, VerifyLog,
    VerifyLogFinalStatus, VerifyLogResult,
};
use wgenty_code::tools::checkpoint_store::CheckpointStore;

// ── helpers ──────────────────────────────────────────────────────────────

/// Initialize a git repo with one seed commit. `.gitignore` excludes
/// `.wgenty-code/` and `*.tmp` so checkpoint artifacts don't pollute the
/// untracked baseline (mirrors the coordinator unit-test helper).
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

/// Run `git` in `dir`, assert success, return trimmed stdout.
fn git_run(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("spawn git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn git_commit(dir: &Path, msg: &str) {
    git_run(dir, &["add", "."]);
    git_run(dir, &["commit", "-m", msg]);
}

/// Bundle of handles for driving a session end-to-end.
struct Session {
    coord: Arc<RwLock<SessionCoordinator>>,
    gate: Arc<VerifyGate>,
    _tmp: TempDir,
    root: PathBuf,
}

/// Build a session over a fresh non-git temp dir (caller runs `init_git_repo`
/// first if it wants git).
fn make_session(git: bool) -> Session {
    let tmp = tempfile::tempdir().expect("tempdir");
    if git {
        init_git_repo(tmp.path());
    }
    let store = Arc::new(CheckpointStore::with_keep_n(tmp.path(), 5));
    let coord = SessionCoordinator::new(
        format!("e2e-{}", uuid::Uuid::new_v4()),
        SessionSource::AgentSelf,
        tmp.path(),
        store,
    )
    .expect("coordinator new");
    let coord = Arc::new(RwLock::new(coord));
    let gate = Arc::new(VerifyGate::new_with_default_hooks(
        coord.clone(),
        Arc::new(ProcessCommandExecutor),
    ));
    Session {
        coord,
        gate,
        root: tmp.path().to_path_buf(),
        _tmp: tmp,
    }
}

/// begin_turn + end_turn, returning the turn id and its checkpoint_turn_id.
fn seal_turn(sess: &Session) -> (String, String) {
    let mut coord = sess.coord.write().unwrap();
    let turn = coord.begin_turn().expect("begin_turn");
    let turn_id = turn.turn_id.clone();
    let ct = turn.checkpoint_turn_id.clone();
    coord.end_turn().expect("end_turn");
    (turn_id, ct)
}

/// Capture a file's pre-edit state then write new content (mirrors what
/// `file_edit` does through the CheckpointStore).
fn capture_and_write(sess: &Session, checkpoint_turn_id: &str, rel: &str, content: &str) {
    sess.coord
        .read()
        .unwrap()
        .checkpoint_store()
        .try_capture_file(checkpoint_turn_id, rel)
        .expect("try_capture_file");
    std::fs::write(sess.root.join(rel), content).expect("write file");
}

/// Read `verify_log.json` from the session dir (best-effort: empty log if
/// missing - the gate writes it before any status transition).
fn read_verify_log(sess: &Session) -> VerifyLog {
    let dir = sess.coord.read().unwrap().session_dir().to_path_buf();
    match std::fs::read_to_string(dir.join("verify_log.json")) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => VerifyLog::default(),
    }
}

// ── 8.1: full closed loop, 3 turns + file edits + verify pass ────────────

#[tokio::test]
async fn full_closed_loop_three_turns_verify_pass() {
    let sess = make_session(true);

    // turn 0: create a.txt (tombstone at capture, then written).
    let (_, ct0) = seal_turn(&sess);
    capture_and_write(&sess, &ct0, "a.txt", "a-v1\n");
    // turn 1: create b.txt.
    let (_, ct1) = seal_turn(&sess);
    capture_and_write(&sess, &ct1, "b.txt", "b-v1\n");
    // turn 2: no file changes (planning / review turn).
    seal_turn(&sess);

    // Verify: `true` passes; actual changed = {a.txt, b.txt} (tombstones +
    // new untracked); expected covers both.
    let result = sess
        .gate
        .verify_and_complete(
            vec!["true".into()],
            vec![PathBuf::from("a.txt"), PathBuf::from("b.txt")],
        )
        .await
        .expect("verify_and_complete");
    assert!(result.success, "expected success: {:?}", result.fail_reason);
    assert!(result.out_of_scope.is_empty());
    assert_eq!(
        result.actual_changed_files,
        vec![PathBuf::from("a.txt"), PathBuf::from("b.txt")]
    );

    let coord = sess.coord.read().unwrap();
    assert_eq!(coord.session().status, SessionStatus::Completed);
    assert_eq!(coord.session().turns.len(), 3);
    // Parent chain intact.
    assert!(coord.session().turns[0].parent.is_none());
    assert_eq!(coord.session().turns[1].parent.as_deref(), Some("turn-0"));
    assert_eq!(coord.session().turns[2].parent.as_deref(), Some("turn-1"));
    drop(coord);

    let log = read_verify_log(&sess);
    assert_eq!(log.attempts.len(), 1);
    assert_eq!(log.attempts[0].result, VerifyLogResult::Completed);
    assert_eq!(log.final_status, Some(VerifyLogFinalStatus::Completed));
}

// ── 8.2: verify fails (command exit != 0) -> AutoRetry, then passes ──────

#[tokio::test]
async fn verify_fail_then_retry_passes() {
    let sess = make_session(true);
    let (_, ct0) = seal_turn(&sess);
    capture_and_write(&sess, &ct0, "a.txt", "a-v1\n");

    // First attempt: `false` exits 1 -> CommandFailed. Default hook returns
    // AutoRetry{remaining:2}; status stays InProgress.
    let r1 = sess
        .gate
        .verify_and_complete(vec!["false".into()], vec![PathBuf::from("a.txt")])
        .await
        .expect("verify call 1");
    assert!(!r1.success);
    assert!(matches!(
        r1.fail_reason,
        Some(VerifyFailure::CommandFailed { .. })
    ));
    assert_eq!(
        r1.action,
        Some(VerifyFailAction::AutoRetry { remaining: 2 })
    );
    assert_eq!(
        sess.coord.read().unwrap().session().status,
        SessionStatus::InProgress
    );

    // Second attempt: `true` exits 0 -> Completed.
    let r2 = sess
        .gate
        .verify_and_complete(vec!["true".into()], vec![PathBuf::from("a.txt")])
        .await
        .expect("verify call 2");
    assert!(r2.success);
    assert_eq!(
        sess.coord.read().unwrap().session().status,
        SessionStatus::Completed
    );

    // verify_log records both attempts; final_status = completed.
    let log = read_verify_log(&sess);
    assert_eq!(log.attempts.len(), 2);
    assert_eq!(log.attempts[0].result, VerifyLogResult::CommandFailed);
    assert_eq!(log.attempts[1].result, VerifyLogResult::Completed);
    assert_eq!(log.final_status, Some(VerifyLogFinalStatus::Completed));
}

// ── 8.3: verify out-of-scope -> fail, agent adjusts expected -> pass ─────

#[tokio::test]
async fn verify_out_of_scope_then_adjusted_passes() {
    let sess = make_session(true);
    let (_, ct0) = seal_turn(&sess);
    // a.txt declared; c.txt created but NOT declared (boundary violation).
    capture_and_write(&sess, &ct0, "a.txt", "a-v1\n");
    capture_and_write(&sess, &ct0, "c.txt", "c-v1\n");

    // First attempt: expected = [a.txt] but actual = {a.txt, c.txt}.
    let r1 = sess
        .gate
        .verify_and_complete(vec!["true".into()], vec![PathBuf::from("a.txt")])
        .await
        .expect("verify call 1");
    assert!(!r1.success);
    match &r1.fail_reason {
        Some(VerifyFailure::BoundaryViolation { unexpected_files }) => {
            assert!(unexpected_files.iter().any(|f| f == "c.txt"));
        }
        other => panic!("expected BoundaryViolation, got {other:?}"),
    }
    assert_eq!(r1.out_of_scope, vec![PathBuf::from("c.txt")]);

    // Agent adjusts: declare c.txt too -> pass.
    let r2 = sess
        .gate
        .verify_and_complete(
            vec!["true".into()],
            vec![PathBuf::from("a.txt"), PathBuf::from("c.txt")],
        )
        .await
        .expect("verify call 2");
    assert!(r2.success);
    assert_eq!(
        sess.coord.read().unwrap().session().status,
        SessionStatus::Completed
    );
}

// ── 8.4: rollback_to restores workspace (git reset + rewind + untracked) ─

#[test]
fn rollback_restores_workspace_full() {
    let sess = make_session(true);
    let sha0 = git_run(&sess.root, &["rev-parse", "HEAD"]);

    // turn 0: capture seed.txt (Saved), edit it (uncommitted).
    let (_, ct0) = seal_turn(&sess);
    capture_and_write(&sess, &ct0, "seed.txt", "modified\n");

    // turn 1: commit seed.txt change (HEAD -> sha1), create untracked.txt.
    seal_turn(&sess);
    git_commit(&sess.root, "modify seed");
    let sha1 = git_run(&sess.root, &["rev-parse", "HEAD"]);
    assert_ne!(sha0, sha1, "commit should advance HEAD");
    std::fs::write(sess.root.join("untracked.txt"), "created\n").unwrap();

    // Rollback to turn-0: git reset --hard sha0 + rewind seed.txt + delete
    // untracked.txt (new untracked, not in turn-0 baseline).
    let result = sess
        .coord
        .write()
        .unwrap()
        .rollback_to("turn-0", &NoHooks)
        .expect("rollback_to");
    assert!(result.git_reset, "git reset should have run");
    assert!(
        result
            .restored_files
            .iter()
            .any(|p| p.to_string_lossy() == "seed.txt"),
        "seed.txt should be in restored_files: {:?}",
        result.restored_files
    );
    assert!(
        result
            .deleted_untracked
            .iter()
            .any(|p| p.to_string_lossy() == "untracked.txt"),
        "untracked.txt should be deleted: {:?}",
        result.deleted_untracked
    );

    // Workspace state.
    assert_eq!(
        std::fs::read_to_string(sess.root.join("seed.txt")).unwrap(),
        "seed\n",
        "seed.txt restored to turn-0 pre-edit content"
    );
    assert!(
        !sess.root.join("untracked.txt").exists(),
        "untracked.txt should be deleted"
    );
    assert_eq!(
        git_run(&sess.root, &["rev-parse", "HEAD"]),
        sha0,
        "HEAD reset to turn-0 start"
    );
    // Cursor moved back.
    assert_eq!(
        sess.coord.read().unwrap().session().current_turn.as_deref(),
        Some("turn-0")
    );
}

// ── 8.5: agent skips verify -> fallback marks Unverified ─────────────────

#[tokio::test]
async fn agent_skips_verify_fallback_unverified() {
    let sess = make_session(true);
    let (_, ct0) = seal_turn(&sess);
    capture_and_write(&sess, &ct0, "a.txt", "a-v1\n");

    // Agent ends without calling verify_and_complete.
    assert_eq!(
        sess.coord.read().unwrap().session().status,
        SessionStatus::InProgress
    );

    // Session close: 兜底 marks Unverified.
    let outcome = sess.gate.mark_unverified_if_incomplete().expect("fallback");
    assert_eq!(outcome, UnverifiedOutcome::MarkedUnverified);
    assert_eq!(
        sess.coord.read().unwrap().session().status,
        SessionStatus::Unverified
    );

    // Idempotent: second call is a no-op (AlreadyTerminal).
    let outcome2 = sess
        .gate
        .mark_unverified_if_incomplete()
        .expect("fallback 2");
    assert_eq!(
        outcome2,
        UnverifiedOutcome::AlreadyTerminal(SessionStatus::Unverified)
    );

    // verify_log records the fallback.
    let log = read_verify_log(&sess);
    assert_eq!(log.final_status, Some(VerifyLogFinalStatus::Unverified));
}

// ── 8.6: non-git project degrades (no git refs, rollback via rewind) ─────

#[tokio::test]
async fn non_git_project_degraded_verify_and_rollback() {
    let sess = make_session(false);

    // turn 0: no git -> git_refs = None, untracked = [].
    let (turn0, ct0) = seal_turn(&sess);
    assert!(
        sess.coord.read().unwrap().session().turns[0]
            .git_refs
            .is_none(),
        "non-git project should have no git_refs"
    );
    capture_and_write(&sess, &ct0, "a.txt", "a-v1\n");

    // Verify: actual = {a.txt} (source 1 only; no git sources). `true` passes.
    let result = sess
        .gate
        .verify_and_complete(vec!["true".into()], vec![PathBuf::from("a.txt")])
        .await
        .expect("verify");
    assert!(
        result.success,
        "non-git verify should pass: {:?}",
        result.fail_reason
    );
    assert_eq!(
        sess.coord.read().unwrap().session().status,
        SessionStatus::Completed
    );

    // Rollback to turn-0: no git reset (git_reset=false), rewind restores
    // a.txt (tombstone -> deleted, since a.txt didn't exist at turn-0 start).
    let result = sess
        .coord
        .write()
        .unwrap()
        .rollback_to("turn-0", &NoHooks)
        .expect("rollback");
    assert!(!result.git_reset, "no git reset in non-git project");
    assert!(
        result
            .restored_files
            .iter()
            .any(|p| p.to_string_lossy() == "a.txt"),
        "a.txt in restored_files: {:?}",
        result.restored_files
    );
    assert!(
        !sess.root.join("a.txt").exists(),
        "a.txt (tombstone) should be deleted by rewind"
    );
    assert_eq!(turn0, "turn-0");
    assert_eq!(
        sess.coord.read().unwrap().session().current_turn.as_deref(),
        Some("turn-0")
    );
}

// ── 8.7: crash consistency - stale tmp never corrupts load ───────────────

#[test]
fn crash_consistency_stale_tmp_does_not_corrupt_load() {
    let sess = make_session(false);
    seal_turn(&sess);
    let session_dir = sess.coord.read().unwrap().session_dir().to_path_buf();

    // 1. session.json is committed with 1 turn. Simulate a crash mid-save:
    //    write garbage to session.json.tmp WITHOUT renaming.
    std::fs::write(
        session_dir.join("session.json.tmp"),
        "{ this is partial / corrupt",
    )
    .unwrap();
    // load reads the committed session.json, ignores the stale tmp.
    let loaded = wgenty_code::exec_session::SessionState::load(&session_dir).expect("load");
    assert_eq!(loaded.turns.len(), 1);
    assert_eq!(loaded.current_turn.as_deref(), Some("turn-0"));

    // 2. A fresh save overwrites the stale tmp and leaves no residue.
    sess.coord
        .write()
        .unwrap()
        .set_status(SessionStatus::Completed)
        .unwrap();
    assert!(
        !session_dir.join("session.json.tmp").exists(),
        "stale tmp should be gone after save"
    );
    let reloaded = wgenty_code::exec_session::SessionState::load(&session_dir).unwrap();
    assert_eq!(reloaded.status, SessionStatus::Completed);
}

#[test]
fn crash_consistency_missing_session_json_with_only_tmp_errors() {
    let sess = make_session(false);
    let session_dir = sess.coord.read().unwrap().session_dir().to_path_buf();
    // Remove session.json (simulate never-completed first save), leave only tmp.
    std::fs::remove_file(session_dir.join("session.json")).unwrap();
    std::fs::write(session_dir.join("session.json.tmp"), "garbage").unwrap();
    let err = wgenty_code::exec_session::SessionState::load(&session_dir)
        .expect_err("load should fail when session.json is missing");
    assert!(
        format!("{err}").contains("session.json"),
        "error should mention session.json: {err}"
    );
}

// ── 8.8: decoupling invariant - no comet dependency in exec_session ──────
//
// The invariant (spec §6, plan Task 8.8): `src/exec_session/` must not branch
// on or import comet-specific code. The only allowed occurrences of "comet"
// are (a) the `SessionSource::Comet` enum variant (PascalCase identifier) and
// (b) its kebab-case serde wire form (the string literal `"comet"`). Both are
// caller-declared labels, not core-runtime branching. This test enforces that
// by scanning the source, stripping comments and string literals, and
// asserting no lowercase "comet" survives in the code.

/// Strip `//` line comments (everything from `//` to EOL). Safe for these
/// files: no `//` appears inside a string literal in `src/exec_session/`.
fn strip_line_comment(line: &str) -> &str {
    match line.find("//") {
        Some(idx) => &line[..idx],
        None => line,
    }
}

/// Remove Rust string literals from `s`, honoring `\"` escapes. Returns the
/// code with string contents elided (quotes removed too).
fn strip_string_literals(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    let mut in_str = false;
    while let Some(c) = chars.next() {
        if in_str {
            if c == '\\' {
                chars.next(); // skip the escaped char
            } else if c == '"' {
                in_str = false;
            }
            // else: inside string literal, elide.
        } else if c == '"' {
            in_str = true;
        } else {
            out.push(c);
        }
    }
    out
}

#[test]
fn exec_session_source_has_no_comet_dependency() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/exec_session");
    let mut violations = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("read exec_session dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let src = std::fs::read_to_string(&path).expect("read source file");
        for (lineno, line) in src.lines().enumerate() {
            let code = strip_line_comment(line);
            let code_no_strings = strip_string_literals(code);
            // Lowercase "comet" in code (outside comments + strings) is a
            // violation. We do NOT lowercase the code: `Comet` (PascalCase,
            // the allowed enum variant) must not match. Only a literal
            // lowercase "comet" substring trips the check.
            if code_no_strings.contains("comet") {
                violations.push(format!(
                    "{}:{}: {}",
                    path.display(),
                    lineno + 1,
                    line.trim()
                ));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "decoupling invariant violated: lowercase 'comet' found in exec_session \
         source code (outside comments / string literals). Only the `Comet` enum \
         variant and its serde wire form are allowed.\nViolations:\n{}",
        violations.join("\n")
    );
}
