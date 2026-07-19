//! Verify-gate: the agent must call `verify_and_complete` to mark a session
//! [`crate::exec_session::SessionStatus::Completed`]. The runtime re-runs the
//! declared commands (anti-fabrication) and checks `actual_changed_files`
//! against `expected_changed_files` (boundary detection). The agent cannot
//! paste a "claimed result" - the [`VerifyAndCompleteTool`] input schema only
//! accepts `commands` and `expected_changed_files`.
//!
//! Task 5 scope: core verify logic + `verify_log.json` persistence + status
//! transition on success. Hook invocation on failure (Task 6) and agent-loop
//! fallback marking `Unverified` (Task 6) layer on top.
//!
//! Spec reference: §3.3 (A 方案). Failure semantics: gate failure is a signal,
//! not a punishment - the runtime never auto-rolls-back. Status stays
//! `InProgress` on failure; Task 6's hook decides retry / escalate.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::exec_session::git::run_git;
use crate::exec_session::hooks::VerifyFailure;
use crate::exec_session::session::{SessionStatus, TurnRecord};
use crate::exec_session::SessionCoordinator;
use crate::tools::checkpoint_store::{FileState, Manifest};
use crate::tools::{Tool, ToolError, ToolOutput};

const VERIFY_LOG_FILE: &str = "verify_log.json";
const VERIFY_LOG_TMP: &str = "verify_log.json.tmp";

/// One command execution record produced by [`CommandExecutor::execute`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandRun {
    pub cmd: String,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Outcome of a `verify_and_complete` attempt.
#[derive(Debug, Clone)]
pub struct VerifyResult {
    pub success: bool,
    pub commands_run: Vec<CommandRun>,
    pub actual_changed_files: Vec<PathBuf>,
    pub expected_changed_files: Vec<PathBuf>,
    pub out_of_scope: Vec<PathBuf>,
    pub fail_reason: Option<VerifyFailure>,
}

/// Abstracts command execution so the gate can route commands through guardian
/// review + sandbox (matching `exec_command`'s treatment, spec §3.3). The
/// default [`ProcessCommandExecutor`] spawns via the OS shell; Task 7 wires a
/// guardian+sandbox-aware executor in the agent loop. Tests inject mocks (plan
/// step 5.5) to verify the executor is called (i.e. the runtime does not
/// accept agent-pasted results).
#[async_trait]
pub trait CommandExecutor: Send + Sync {
    /// Execute `command` in `project_root`. Returns the exit code, stdout,
    /// stderr. Non-zero exit is a valid [`CommandRun`] (not an error); errors
    /// only on spawn failure or IO issues.
    async fn execute(&self, command: &str, project_root: &Path) -> Result<CommandRun>;
}

/// Default executor: spawns `sh -c <command>` (unix) / `cmd /C <command>`
/// (windows) in `project_root`. Task 7 replaces this with a guardian+sandbox
/// aware executor in the agent loop.
pub struct ProcessCommandExecutor;

#[async_trait]
impl CommandExecutor for ProcessCommandExecutor {
    async fn execute(&self, command: &str, project_root: &Path) -> Result<CommandRun> {
        let mut cmd = if cfg!(target_os = "windows") {
            let mut c = tokio::process::Command::new("cmd");
            c.args(["/C", command]);
            c
        } else {
            let mut c = tokio::process::Command::new("sh");
            c.args(["-c", command]);
            c
        };
        cmd.current_dir(project_root);
        let output = cmd
            .output()
            .await
            .with_context(|| format!("spawn command: {command}"))?;
        Ok(CommandRun {
            cmd: command.to_string(),
            exit_code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        })
    }
}

/// Per-attempt result recorded in `verify_log.json`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VerifyLogResult {
    Completed,
    CommandFailed,
    OutOfScope,
}

/// One attempt's record in `verify_log.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyLogEntry {
    pub attempt: usize,
    pub timestamp: String,
    pub commands_run: Vec<CommandRun>,
    pub actual_changed_files: Vec<String>,
    pub expected_changed_files: Vec<String>,
    pub out_of_scope: Vec<String>,
    pub result: VerifyLogResult,
}

/// Final status recorded in `verify_log.json` when the session reaches a
/// terminal verify state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VerifyLogFinalStatus {
    Completed,
    Failed,
    Unverified,
}

/// `verify_log.json` structure: a list of attempts + final_status. Persisted
/// at `<session_dir>/verify_log.json` (atomic tmp+rename, same as session.json).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VerifyLog {
    pub attempts: Vec<VerifyLogEntry>,
    pub final_status: Option<VerifyLogFinalStatus>,
}

/// The verify-gate. Holds a shared coordinator (so the agent loop and the tool
/// can both access it) and a command executor. Task 6 adds a hooks field for
/// `verify_fail` invocation.
///
/// The coordinator is wrapped in `Arc<RwLock<...>>` because the agent loop
/// (Task 7) and this gate share it: the loop drives `begin_turn` / `end_turn`,
/// the gate reads session state and transitions status. Locks are never held
/// across `.await` points - project_root / turns are cloned out of the read
/// guard before any async work.
pub struct VerifyGate {
    coordinator: Arc<RwLock<SessionCoordinator>>,
    executor: Arc<dyn CommandExecutor>,
}

impl VerifyGate {
    pub fn new(
        coordinator: Arc<RwLock<SessionCoordinator>>,
        executor: Arc<dyn CommandExecutor>,
    ) -> Self {
        Self {
            coordinator,
            executor,
        }
    }

    /// Run `commands` (via the executor, which routes through guardian+sandbox
    /// in production), compute `actual_changed_files` (three sources, spec
    /// §3.3), check `actual ⊆ expected`, write `verify_log.json`, and on
    /// success transition the session to `Completed`. On failure, status stays
    /// `InProgress` (Task 6 invokes the hook).
    ///
    /// Anti-fabrication: the runtime runs the commands itself; the agent only
    /// supplies the command strings and expected file set. The agent cannot
    /// paste a "claimed result".
    pub async fn verify_and_complete(
        &self,
        commands: Vec<String>,
        expected_changed_files: Vec<PathBuf>,
    ) -> Result<VerifyResult> {
        let project_root = {
            let coord = self
                .coordinator
                .read()
                .map_err(|e| anyhow::anyhow!("coordinator read lock: {e}"))?;
            coord.project_root().to_path_buf()
        };

        // 1. Execute commands (anti-fabrication: runtime runs them itself).
        let mut commands_run = Vec::with_capacity(commands.len());
        let mut command_failure: Option<VerifyFailure> = None;
        for cmd in &commands {
            let run = self.executor.execute(cmd, &project_root).await?;
            if run.exit_code != Some(0) && command_failure.is_none() {
                command_failure = Some(VerifyFailure::CommandFailed {
                    command: cmd.clone(),
                    exit_code: run.exit_code,
                    stderr: run.stderr.clone(),
                });
            }
            commands_run.push(run);
        }

        // 2. Compute actual_changed_files (three sources).
        let actual = self.compute_actual_changed_files(&project_root)?;

        // 3. Boundary check: actual ⊆ expected.
        let expected_set: HashSet<PathBuf> = expected_changed_files.iter().cloned().collect();
        let out_of_scope: Vec<PathBuf> = actual
            .iter()
            .filter(|f| !expected_set.contains(*f))
            .cloned()
            .collect();

        // 4. Determine fail_reason. Command failure takes precedence; boundary
        //    is checked regardless so the agent sees both signals when both
        //    fail (the out_of_scope list is always populated for visibility).
        let fail_reason = command_failure.or_else(|| {
            if out_of_scope.is_empty() {
                None
            } else {
                Some(VerifyFailure::BoundaryViolation {
                    unexpected_files: out_of_scope
                        .iter()
                        .map(|p| p.to_string_lossy().into_owned())
                        .collect(),
                })
            }
        });
        let success = fail_reason.is_none();

        // 5. Write verify_log (append attempt). Best-effort read of the
        //    session dir - the lock is released before IO.
        let (session_dir, turns_count) = {
            let coord = self
                .coordinator
                .read()
                .map_err(|e| anyhow::anyhow!("coordinator read lock: {e}"))?;
            (
                coord.session_dir().to_path_buf(),
                coord.session().turns.len(),
            )
        };
        let attempt_num = append_verify_log(
            &session_dir,
            &commands_run,
            &actual,
            &expected_changed_files,
            &out_of_scope,
            &fail_reason,
        )?;

        // 6. Transition session status on success. On failure, status stays
        //    InProgress (Task 6 invokes the hook).
        if success {
            let mut coord = self
                .coordinator
                .write()
                .map_err(|e| anyhow::anyhow!("coordinator write lock: {e}"))?;
            coord.set_status(SessionStatus::Completed)?;
        }

        tracing::info!(
            session_turns = turns_count,
            attempt = attempt_num,
            success,
            "verify_and_complete"
        );

        Ok(VerifyResult {
            success,
            commands_run,
            actual_changed_files: actual,
            expected_changed_files,
            out_of_scope,
            fail_reason,
        })
    }

    /// Compute `actual_changed_files` (spec §3.3): union of three sources.
    ///
    /// 1. **CheckpointStore manifest union** (session scope): for each turn in
    ///    the session, read the manifest and collect `Saved` / `Tombstone`
    ///    file paths (file_edit / file_write / apply_patch mutations).
    /// 2. **`git diff --name-only <session_start_head>`** (tracked changes
    ///    since session start, committed or not). Only when the first turn
    ///    recorded a `HEAD`.
    /// 3. **New untracked files**: current `git ls-files --others` minus the
    ///    first turn's baseline untracked set. Only when the first turn
    ///    recorded git state.
    ///
    /// Non-git projects (first turn has no `git_refs`): only Source 1 applies.
    fn compute_actual_changed_files(&self, project_root: &Path) -> Result<Vec<PathBuf>> {
        let mut files: HashSet<PathBuf> = HashSet::new();

        let turns: Vec<TurnRecord> = {
            let coord = self
                .coordinator
                .read()
                .map_err(|e| anyhow::anyhow!("coordinator read lock: {e}"))?;
            coord.session().turns.clone()
        };

        // Source 1: CheckpointStore manifest union.
        for turn in &turns {
            for path in read_manifest_paths(project_root, &turn.checkpoint_turn_id) {
                files.insert(path);
            }
        }

        // Sources 2 + 3: only when the first turn recorded git state.
        if let Some(first) = turns.first() {
            if let Some(refs) = &first.git_refs {
                // Source 2: git diff --name-only <session_start_head>.
                if let Ok(out) = run_git(project_root, &["diff", "--name-only", &refs.head]) {
                    for line in out.lines() {
                        if !line.is_empty() {
                            files.insert(PathBuf::from(line));
                        }
                    }
                }
                // Source 3: new untracked = current untracked - first-turn baseline.
                let baseline: HashSet<&str> =
                    first.untracked_files.iter().map(|s| s.as_str()).collect();
                if let Ok(out) = run_git(
                    project_root,
                    &["ls-files", "--others", "--exclude-standard"],
                ) {
                    for line in out.lines() {
                        if !line.is_empty() && !baseline.contains(line) {
                            files.insert(PathBuf::from(line));
                        }
                    }
                }
            }
        }

        let mut result: Vec<PathBuf> = files.into_iter().collect();
        result.sort();
        Ok(result)
    }
}

/// Read the file paths recorded in a turn's checkpoint manifest (best-effort).
/// Returns relative paths for entries with state `Saved` or `Tombstone`;
/// `Skipped` entries are excluded. An unreadable / missing manifest yields an
/// empty list - this mirrors `SessionCoordinator::collect_rewind_files` but is
/// a free function so the verify-gate can call it without a coordinator
/// reference (it only needs `project_root` + `checkpoint_turn_id`).
fn read_manifest_paths(project_root: &Path, checkpoint_turn_id: &str) -> Vec<PathBuf> {
    let manifest_path = project_root
        .join(".wgenty-code")
        .join("checkpoints")
        .join(checkpoint_turn_id)
        .join("manifest.json");
    let data = match std::fs::read_to_string(&manifest_path) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };
    let manifest: Manifest = match serde_json::from_str(&data) {
        Ok(m) => m,
        Err(_) => return Vec::new(),
    };
    manifest
        .files
        .into_iter()
        .filter(|e| e.state != FileState::Skipped)
        .map(|e| PathBuf::from(e.path))
        .collect()
}

/// Read `verify_log.json` from `session_dir`. Returns an empty
/// [`VerifyLog`] if the file is missing or unparseable (best-effort: the gate
/// proceeds even if the log is unreadable; the attempts list is the source of
/// truth for the attempt counter).
fn read_verify_log(session_dir: &Path) -> VerifyLog {
    let path = session_dir.join(VERIFY_LOG_FILE);
    let data = match std::fs::read_to_string(&path) {
        Ok(d) => d,
        Err(_) => return VerifyLog::default(),
    };
    serde_json::from_str(&data).unwrap_or_default()
}

/// Atomically write `verify_log.json` (tmp + rename, same L1 pattern as
/// `session.json`).
fn write_verify_log(session_dir: &Path, log: &VerifyLog) -> Result<()> {
    let tmp = session_dir.join(VERIFY_LOG_TMP);
    let final_path = session_dir.join(VERIFY_LOG_FILE);
    let data = serde_json::to_string_pretty(log).context("serialize verify_log.json")?;
    std::fs::write(&tmp, &data)
        .with_context(|| format!("write {}: {}", VERIFY_LOG_TMP, tmp.display()))?;
    std::fs::rename(&tmp, &final_path)
        .with_context(|| format!("rename {} -> {}", VERIFY_LOG_TMP, VERIFY_LOG_FILE))?;
    Ok(())
}

/// Append an attempt to `verify_log.json` and return the 1-based attempt
/// number. Sets `final_status = Completed` when the attempt succeeds.
fn append_verify_log(
    session_dir: &Path,
    commands_run: &[CommandRun],
    actual: &[PathBuf],
    expected: &[PathBuf],
    out_of_scope: &[PathBuf],
    fail_reason: &Option<VerifyFailure>,
) -> Result<usize> {
    let mut log = read_verify_log(session_dir);
    let attempt_num = log.attempts.len() + 1;
    let result = match fail_reason {
        None => VerifyLogResult::Completed,
        Some(VerifyFailure::CommandFailed { .. }) => VerifyLogResult::CommandFailed,
        Some(VerifyFailure::BoundaryViolation { .. }) => VerifyLogResult::OutOfScope,
    };
    let entry = VerifyLogEntry {
        attempt: attempt_num,
        timestamp: chrono::Utc::now().to_rfc3339(),
        commands_run: commands_run.to_vec(),
        actual_changed_files: actual
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect(),
        expected_changed_files: expected
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect(),
        out_of_scope: out_of_scope
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect(),
        result,
    };
    log.attempts.push(entry);
    if result == VerifyLogResult::Completed {
        log.final_status = Some(VerifyLogFinalStatus::Completed);
    }
    write_verify_log(session_dir, &log)?;
    Ok(attempt_num)
}

/// Tool wrapper: exposes [`VerifyGate::verify_and_complete`] as a [`Tool`] the
/// agent can call. The input schema accepts only `commands` and
/// `expected_changed_files` - there is no `result` / `status` field, so the
/// agent cannot paste a claimed result (anti-fabrication, spec §3.3).
///
/// Constructed per-session by the agent loop (Task 7); not registered in
/// `ToolRegistry::with_project_root` because it needs the session's
/// coordinator.
pub struct VerifyAndCompleteTool {
    gate: Arc<VerifyGate>,
}

impl VerifyAndCompleteTool {
    pub fn new(gate: Arc<VerifyGate>) -> Self {
        Self { gate }
    }
}

#[async_trait]
impl Tool for VerifyAndCompleteTool {
    fn name(&self) -> &str {
        "verify_and_complete"
    }

    fn description(&self) -> &str {
        "Mark the current exec session as completed by running verification \
commands and checking changed files against the expected set. The runtime \
runs the commands itself (anti-fabrication) and rejects out-of-scope changes."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "commands": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Verification commands to run (e.g. cargo test). The runtime executes these itself."
                },
                "expected_changed_files": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Files the agent expects to have changed. The runtime rejects changes outside this set."
                }
            },
            "required": ["commands", "expected_changed_files"]
        })
    }

    // is_read_only defaults to false: the tool transitions session.status to
    // Completed and writes verify_log.json.

    async fn execute(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let commands = parse_string_array(&input, "commands")?;
        let expected = parse_string_array(&input, "expected_changed_files")?
            .into_iter()
            .map(PathBuf::from)
            .collect::<Vec<_>>();
        let result = self
            .gate
            .verify_and_complete(commands, expected)
            .await
            .map_err(|e| ToolError {
                message: format!("{e:#}"),
                code: Some("verify_failed".to_string()),
            })?;
        let content = format_verify_result(&result);
        let mut metadata = std::collections::HashMap::new();
        metadata.insert("success".to_string(), serde_json::json!(result.success));
        metadata.insert(
            "out_of_scope".to_string(),
            serde_json::json!(result
                .out_of_scope
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect::<Vec<_>>()),
        );
        Ok(ToolOutput {
            output_type: "text".to_string(),
            content,
            metadata,
        })
    }
}

/// Parse a required JSON string array field. Returns a [`ToolError`] if the
/// field is missing or not an array of strings.
fn parse_string_array(input: &serde_json::Value, field: &str) -> Result<Vec<String>, ToolError> {
    input
        .get(field)
        .and_then(|v| v.as_array())
        .ok_or_else(|| ToolError {
            message: format!("{field} is required"),
            code: Some("missing_parameter".to_string()),
        })?
        .iter()
        .map(|v| {
            v.as_str().map(String::from).ok_or_else(|| ToolError {
                message: format!("{field} must be an array of strings"),
                code: Some("invalid_parameter".to_string()),
            })
        })
        .collect()
}

/// Format a [`VerifyResult`] as human-readable text for the tool output.
fn format_verify_result(result: &VerifyResult) -> String {
    let mut lines = Vec::new();
    if result.success {
        lines.push("Verify: PASSED. Session marked Completed.".to_string());
    } else {
        lines.push("Verify: FAILED.".to_string());
        match &result.fail_reason {
            Some(VerifyFailure::CommandFailed {
                command,
                exit_code,
                stderr,
            }) => {
                lines.push(format!(
                    "  Command failed: `{command}` (exit {exit_code:?})"
                ));
                if !stderr.is_empty() {
                    lines.push(format!("  stderr: {}", stderr.trim()));
                }
            }
            Some(VerifyFailure::BoundaryViolation { unexpected_files }) => {
                lines.push(format!(
                    "  Out-of-scope files ({}):",
                    unexpected_files.len()
                ));
                for f in unexpected_files {
                    lines.push(format!("    - {f}"));
                }
            }
            None => {}
        }
    }
    lines.push(format!(
        "  actual_changed_files ({}): {:?}",
        result.actual_changed_files.len(),
        result
            .actual_changed_files
            .iter()
            .map(|p| p.to_string_lossy())
            .collect::<Vec<_>>()
    ));
    lines.push(format!(
        "  expected_changed_files ({}): {:?}",
        result.expected_changed_files.len(),
        result
            .expected_changed_files
            .iter()
            .map(|p| p.to_string_lossy())
            .collect::<Vec<_>>()
    ));
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec_session::session::SessionSource;
    use crate::tools::checkpoint_store::CheckpointStore;
    use std::process::Command;
    use std::sync::Mutex;
    use tempfile::{tempdir, TempDir};

    // ---- test helpers ----

    /// Mock executor: records every call and returns a fixed exit code. Stands
    /// in for the guardian+sandbox-aware executor (plan step 5.5) - the point
    /// is to verify the gate routes commands through the executor rather than
    /// accepting agent-pasted results.
    struct RecordingExecutor {
        calls: Mutex<Vec<String>>,
        exit_code: i32,
    }

    impl RecordingExecutor {
        fn new(exit_code: i32) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                exit_code,
            }
        }
        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl CommandExecutor for RecordingExecutor {
        async fn execute(&self, command: &str, _project_root: &Path) -> Result<CommandRun> {
            self.calls.lock().unwrap().push(command.to_string());
            Ok(CommandRun {
                cmd: command.to_string(),
                exit_code: Some(self.exit_code),
                stdout: String::new(),
                stderr: String::new(),
            })
        }
    }

    /// Bundle of handles a test needs to drive the gate + inspect state.
    struct TestSetup {
        gate: Arc<VerifyGate>,
        coord: Arc<RwLock<SessionCoordinator>>,
        store: Arc<CheckpointStore>,
        executor: Arc<RecordingExecutor>,
        _dir: TempDir,
        dir_path: std::path::PathBuf,
    }

    fn setup(exit_code: i32) -> TestSetup {
        let dir = tempdir().unwrap();
        let store = Arc::new(CheckpointStore::new(dir.path()));
        let coord = SessionCoordinator::new(
            "es-test".into(),
            SessionSource::AgentSelf,
            dir.path(),
            store.clone(),
        )
        .unwrap();
        let coord_arc = Arc::new(RwLock::new(coord));
        let executor = Arc::new(RecordingExecutor::new(exit_code));
        let gate = Arc::new(VerifyGate::new(coord_arc.clone(), executor.clone()));
        TestSetup {
            gate,
            coord: coord_arc,
            store,
            executor,
            dir_path: dir.path().to_path_buf(),
            _dir: dir,
        }
    }

    /// Initialize a git repo with one commit; `.gitignore` excludes
    /// `.wgenty-code/` and `*.tmp` so checkpoint artifacts don't pollute
    /// untracked listings.
    fn init_git_repo(dir: &Path) {
        let cmds: &[&[&str]] = &[
            &["init"],
            &["config", "user.email", "test@wgenty.local"],
            &["config", "user.name", "wgenty test"],
        ];
        for args in cmds {
            let status = Command::new("git")
                .args(*args)
                .current_dir(dir)
                .status()
                .expect("spawn git");
            assert!(status.success(), "git {:?} failed", args);
        }
        std::fs::write(dir.join(".gitignore"), ".wgenty-code/\n*.tmp\n").unwrap();
        std::fs::write(dir.join("seed.txt"), "seed\n").unwrap();
        for args in [&["add", "."][..], &["commit", "-m", "seed"][..]] {
            let status = Command::new("git")
                .args(args)
                .current_dir(dir)
                .status()
                .expect("git");
            assert!(status.success(), "git {:?} failed", args);
        }
    }

    fn begin_turn(setup: &TestSetup) -> String {
        let mut coord = setup.coord.write().unwrap();
        let turn = coord.begin_turn().unwrap();
        let turn_id = turn.turn_id.clone();
        coord.end_turn().unwrap();
        turn_id
    }

    /// Get the checkpoint_turn_id for a turn (to drive try_capture_file).
    fn checkpoint_turn_id(setup: &TestSetup, turn_idx: usize) -> String {
        let coord = setup.coord.read().unwrap();
        coord.session().turns[turn_idx].checkpoint_turn_id.clone()
    }

    // ---- 5.1: success path -> Completed + verify_log ----

    #[tokio::test]
    async fn verify_success_marks_completed_and_logs() {
        let setup = setup(0);
        begin_turn(&setup);
        // No files changed; commands pass; expected is empty -> actual ⊆ expected.
        let result = setup
            .gate
            .verify_and_complete(vec!["true".into()], vec![])
            .await
            .unwrap();
        assert!(result.success, "expected success");
        assert!(result.fail_reason.is_none());
        assert!(result.out_of_scope.is_empty());
        // Session status -> Completed.
        let coord = setup.coord.read().unwrap();
        assert_eq!(coord.session().status, SessionStatus::Completed);
        // verify_log: 1 attempt, result=completed, final_status=completed.
        let log = read_verify_log(coord.session_dir());
        assert_eq!(log.attempts.len(), 1);
        assert_eq!(log.attempts[0].result, VerifyLogResult::Completed);
        assert_eq!(log.final_status, Some(VerifyLogFinalStatus::Completed));
    }

    // ---- 5.2: command failure -> fail, status stays InProgress ----

    #[tokio::test]
    async fn verify_command_failure_keeps_in_progress() {
        let setup = setup(1); // executor returns exit 1
        begin_turn(&setup);
        let result = setup
            .gate
            .verify_and_complete(vec!["cargo test".into()], vec![])
            .await
            .unwrap();
        assert!(!result.success);
        match &result.fail_reason {
            Some(VerifyFailure::CommandFailed {
                command, exit_code, ..
            }) => {
                assert_eq!(command, "cargo test");
                assert_eq!(*exit_code, Some(1));
            }
            other => panic!("expected CommandFailed, got {other:?}"),
        }
        // Status stays InProgress (Task 6 adds hook-driven transitions).
        let coord = setup.coord.read().unwrap();
        assert_eq!(coord.session().status, SessionStatus::InProgress);
        // verify_log records the failure.
        let log = read_verify_log(coord.session_dir());
        assert_eq!(log.attempts.len(), 1);
        assert_eq!(log.attempts[0].result, VerifyLogResult::CommandFailed);
        assert_eq!(log.final_status, None); // not completed
    }

    // ---- 5.3: boundary violation -> OutOfScope ----

    #[tokio::test]
    async fn verify_boundary_violation_lists_out_of_scope() {
        let setup = setup(0);
        begin_turn(&setup);
        let ct_id = checkpoint_turn_id(&setup, 0);
        // Capture a file in the manifest so it appears in actual_changed_files.
        std::fs::create_dir_all(setup.dir_path.join("src")).unwrap();
        std::fs::write(setup.dir_path.join("src/edited.rs"), "new\n").unwrap();
        setup
            .store
            .try_capture_file(&ct_id, "src/edited.rs")
            .unwrap();
        // expected does NOT include src/edited.rs -> out of scope.
        let result = setup
            .gate
            .verify_and_complete(vec!["true".into()], vec![])
            .await
            .unwrap();
        assert!(!result.success);
        match &result.fail_reason {
            Some(VerifyFailure::BoundaryViolation { unexpected_files }) => {
                assert!(
                    unexpected_files.iter().any(|f| f == "src/edited.rs"),
                    "expected src/edited.rs in out_of_scope: {unexpected_files:?}"
                );
            }
            other => panic!("expected BoundaryViolation, got {other:?}"),
        }
        assert!(
            result
                .out_of_scope
                .iter()
                .any(|p| p == std::path::Path::new("src/edited.rs")),
            "out_of_scope should contain src/edited.rs: {:?}",
            result.out_of_scope
        );
    }

    // ---- 5.4: actual = three sources (manifest + git diff + new untracked) ----

    #[tokio::test]
    async fn verify_actual_changed_files_three_sources() {
        let dir = tempdir().unwrap();
        init_git_repo(dir.path());
        let store = Arc::new(CheckpointStore::new(dir.path()));
        let coord = SessionCoordinator::new(
            "es-git".into(),
            SessionSource::AgentSelf,
            dir.path(),
            store.clone(),
        )
        .unwrap();
        let coord_arc = Arc::new(RwLock::new(coord));
        let executor = Arc::new(RecordingExecutor::new(0));
        let gate = Arc::new(VerifyGate::new(coord_arc.clone(), executor.clone()));

        // Begin turn-0: records HEAD + untracked baseline.
        {
            let mut c = coord_arc.write().unwrap();
            c.begin_turn().unwrap();
            c.end_turn().unwrap();
        }
        let ct_id = coord_arc.read().unwrap().session().turns[0]
            .checkpoint_turn_id
            .clone();

        // Source 1: manifest entry (file tool mutation).
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/edited.rs"), "new\n").unwrap();
        store.try_capture_file(&ct_id, "src/edited.rs").unwrap();

        // Source 2: tracked file modified (git diff).
        std::fs::write(dir.path().join("seed.txt"), "modified\n").unwrap();

        // Source 3: new untracked file (not in turn-0 baseline).
        std::fs::write(dir.path().join("scratch.log"), "x\n").unwrap();

        // expected covers all three -> success.
        let expected = vec![
            PathBuf::from("src/edited.rs"),
            PathBuf::from("seed.txt"),
            PathBuf::from("scratch.log"),
        ];
        let result = gate
            .verify_and_complete(vec!["true".into()], expected.clone())
            .await
            .unwrap();

        // actual must contain all three files (three sources covered).
        for f in &["src/edited.rs", "seed.txt", "scratch.log"] {
            assert!(
                result
                    .actual_changed_files
                    .iter()
                    .any(|p| p == std::path::Path::new(f)),
                "actual should contain {f} (three-source check): {:?}",
                result.actual_changed_files
            );
        }
        assert!(result.success, "expected success with full coverage");
        assert!(result.out_of_scope.is_empty());
    }

    #[tokio::test]
    async fn verify_actual_degrades_without_git() {
        // Non-git project: only Source 1 (manifest) applies.
        let setup = setup(0);
        begin_turn(&setup);
        let ct_id = checkpoint_turn_id(&setup, 0);
        std::fs::create_dir_all(setup.dir_path.join("src")).unwrap();
        std::fs::write(setup.dir_path.join("src/edited.rs"), "new\n").unwrap();
        setup
            .store
            .try_capture_file(&ct_id, "src/edited.rs")
            .unwrap();
        let result = setup
            .gate
            .verify_and_complete(vec!["true".into()], vec![PathBuf::from("src/edited.rs")])
            .await
            .unwrap();
        assert!(result.success);
        assert!(
            result
                .actual_changed_files
                .iter()
                .any(|p| p == std::path::Path::new("src/edited.rs")),
            "manifest source should still work without git"
        );
    }

    // ---- 5.5: commands routed through executor (anti-fabrication) ----

    #[tokio::test]
    async fn verify_routes_commands_through_executor() {
        let setup = setup(0);
        begin_turn(&setup);
        setup
            .gate
            .verify_and_complete(vec!["cargo test".into(), "cargo clippy".into()], vec![])
            .await
            .unwrap();
        let calls = setup.executor.calls();
        assert_eq!(calls, vec!["cargo test", "cargo clippy"]);
    }

    // ---- 5.6: verify_log appends attempts across calls ----

    #[tokio::test]
    async fn verify_log_appends_across_attempts() {
        let dir = tempdir().unwrap();
        init_git_repo(dir.path());
        let store = Arc::new(CheckpointStore::new(dir.path()));
        let coord =
            SessionCoordinator::new("es-log".into(), SessionSource::AgentSelf, dir.path(), store)
                .unwrap();
        let coord_arc = Arc::new(RwLock::new(coord));
        // First attempt: fail (exit 1).
        let fail_executor = Arc::new(RecordingExecutor::new(1));
        let gate_fail = Arc::new(VerifyGate::new(coord_arc.clone(), fail_executor.clone()));
        {
            let mut c = coord_arc.write().unwrap();
            c.begin_turn().unwrap();
            c.end_turn().unwrap();
        }
        let r1 = gate_fail
            .verify_and_complete(vec!["cargo test".into()], vec![])
            .await
            .unwrap();
        assert!(!r1.success);

        // Second attempt: succeed (exit 0) with a fresh executor.
        let ok_executor = Arc::new(RecordingExecutor::new(0));
        let gate_ok = Arc::new(VerifyGate::new(coord_arc.clone(), ok_executor));
        let r2 = gate_ok
            .verify_and_complete(vec!["cargo test".into()], vec![])
            .await
            .unwrap();
        assert!(r2.success);

        // verify_log: 2 attempts, final_status=completed.
        let session_dir = coord_arc.read().unwrap().session_dir().to_path_buf();
        let log = read_verify_log(&session_dir);
        assert_eq!(log.attempts.len(), 2, "two attempts should be logged");
        assert_eq!(log.attempts[0].attempt, 1);
        assert_eq!(log.attempts[0].result, VerifyLogResult::CommandFailed);
        assert_eq!(log.attempts[1].attempt, 2);
        assert_eq!(log.attempts[1].result, VerifyLogResult::Completed);
        assert_eq!(log.final_status, Some(VerifyLogFinalStatus::Completed));
        // No stale .tmp after atomic write.
        assert!(!session_dir.join(VERIFY_LOG_TMP).exists());
    }

    // ---- 5.7: input_schema has no result/status field (anti-fabrication) ----

    #[test]
    fn tool_input_schema_rejects_claimed_results() {
        let setup = setup(0);
        let tool = VerifyAndCompleteTool::new(setup.gate.clone());
        let schema = tool.input_schema();
        let props = schema["properties"].as_object().expect("properties");
        // Must accept commands + expected_changed_files.
        assert!(props.contains_key("commands"));
        assert!(props.contains_key("expected_changed_files"));
        // Must NOT accept agent-pasted result/status/exit_code/output fields.
        assert!(
            !props.contains_key("result"),
            "anti-fabrication: schema must not have 'result'"
        );
        assert!(
            !props.contains_key("status"),
            "anti-fabrication: schema must not have 'status'"
        );
        assert!(
            !props.contains_key("exit_code"),
            "anti-fabrication: schema must not have 'exit_code'"
        );
        assert!(
            !props.contains_key("output"),
            "anti-fabrication: schema must not have 'output'"
        );
        // required has exactly the two fields.
        let required: Vec<&str> = schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(required.contains(&"commands"));
        assert!(required.contains(&"expected_changed_files"));
    }

    #[test]
    fn tool_is_not_read_only() {
        let setup = setup(0);
        let tool = VerifyAndCompleteTool::new(setup.gate.clone());
        // verify_and_complete transitions session.status + writes verify_log.
        assert!(!tool.is_read_only());
    }

    #[tokio::test]
    async fn tool_execute_round_trip_success() {
        let setup = setup(0);
        begin_turn(&setup);
        let tool = VerifyAndCompleteTool::new(setup.gate.clone());
        let input = serde_json::json!({
            "commands": ["true"],
            "expected_changed_files": []
        });
        let out = tool.execute(input).await.unwrap();
        assert!(out.metadata["success"].as_bool().unwrap());
        assert!(out.content.contains("PASSED"));
        let coord = setup.coord.read().unwrap();
        assert_eq!(coord.session().status, SessionStatus::Completed);
    }

    #[tokio::test]
    async fn tool_execute_errors_on_missing_commands() {
        let setup = setup(0);
        let tool = VerifyAndCompleteTool::new(setup.gate.clone());
        let input = serde_json::json!({ "expected_changed_files": [] });
        let err = tool.execute(input).await.unwrap_err();
        assert!(err.message.contains("commands"));
        assert_eq!(err.code.as_deref(), Some("missing_parameter"));
    }

    // ---- process executor smoke test (uses real `sh`) ----

    #[tokio::test]
    async fn process_executor_runs_real_command() {
        let dir = tempdir().unwrap();
        let executor = ProcessCommandExecutor;
        // `true` exits 0 on unix; `cmd /C exit 0` on windows. Use a portable
        // command: echo (exits 0 on both).
        let run = executor.execute("echo hello", dir.path()).await.unwrap();
        assert_eq!(run.exit_code, Some(0));
        assert!(run.stdout.contains("hello") || run.stdout.contains("hello\r"));
    }
}
