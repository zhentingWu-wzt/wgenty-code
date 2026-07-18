//! Checkpoint/Undo tools - git stash-based rollback.
//!
//! Creates a git stash checkpoint before risky operations; undo restores.
//!
//! ## Design note: non-destructive snapshot
//!
//! A naive `git stash push` saves the working tree into a stash **and clears
//! the working tree in the process** - which is itself destructive. An earlier
//! implementation did exactly that, stranding in-progress work in the stash if
//! the session ended before `undo` was called.
//!
//! `create` therefore uses the "push then re-apply" pattern: `git stash push
//! --include-untracked` captures the full state (tracked + staged + untracked)
//! into a stash, and we immediately `git stash apply --index stash@{0}` to
//! restore the working tree to its exact prior state while keeping the stash.
//! The net effect is a non-destructive snapshot: the caller's working tree is
//! untouched, and the returned SHA identifies the checkpoint for a precise
//! `undo` later.

use std::path::PathBuf;

use crate::tools::{Tool, ToolError, ToolOutput};
use async_trait::async_trait;

pub struct CheckpointManager {
    count: std::sync::atomic::AtomicU32,
    /// Working directory for git operations. `None` inherits the process CWD
    /// (the normal case - the agent runs in the project root). `Some(dir)` is
    /// used by tests (and potentially the daemon) to scope operations.
    workdir: Option<PathBuf>,
}

impl Default for CheckpointManager {
    fn default() -> Self {
        Self::new()
    }
}

impl CheckpointManager {
    pub fn new() -> Self {
        Self {
            count: std::sync::atomic::AtomicU32::new(0),
            workdir: None,
        }
    }

    /// Construct with an explicit working directory (used by tests).
    pub fn new_with_workdir(dir: impl Into<PathBuf>) -> Self {
        Self {
            count: std::sync::atomic::AtomicU32::new(0),
            workdir: Some(dir.into()),
        }
    }

    /// Build a git command scoped to `workdir` when set.
    fn git(&self) -> tokio::process::Command {
        let mut cmd = tokio::process::Command::new("git");
        if let Some(dir) = &self.workdir {
            cmd.current_dir(dir);
        }
        cmd
    }

    /// Create a non-destructive git stash checkpoint.
    ///
    /// Captures tracked + staged + untracked changes into a stash **without
    /// clearing the working tree**, and returns the stash commit SHA as the
    /// checkpoint ID. The SHA can be passed to [`undo`](Self::undo) for a
    /// precise restore; if omitted, `undo` targets the most recent
    /// `wgenty-checkpoint` stash.
    pub async fn create(&self, description: &str) -> Result<String, String> {
        let n = self.count.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        let msg = format!("wgenty-checkpoint-{}: {}", n, description);

        // Step 1: push captures the full state (incl. untracked) into a stash.
        // This clears the working tree, so we must restore it below.
        let push = self
            .git()
            .args(["stash", "push", "--include-untracked", "-m", &msg])
            .output()
            .await
            .map_err(|e| format!("git stash push failed: {}", e))?;
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&push.stdout),
            String::from_utf8_lossy(&push.stderr)
        );
        // `git stash push` prints "No local changes to save" and may exit 0 or
        // non-zero depending on the git version. Detect by message first so we
        // don't try to resolve a (non-existent) stash ref below.
        if combined.contains("No local changes to save") {
            return Err("No local changes to checkpoint".to_string());
        }
        if !push.status.success() {
            return Err(format!("git stash push failed: {}", combined.trim()));
        }

        // Step 2: resolve the SHA of the stash we just created. This is the
        // stable checkpoint ID returned to the caller (stash@{n} indices shift
        // as stashes are added/dropped; a SHA does not).
        let sha = self.resolve_head_stash_sha().await?;
        if sha.is_empty() {
            // Should be unreachable after a successful push, but fail safely
            // rather than returning an empty checkpoint ID.
            return Err("git stash push succeeded but no stash SHA was found".to_string());
        }

        // Step 3: restore the working tree to its pre-checkpoint state. The
        // tree is currently clean (push reset it), so the apply always lands
        // cleanly. `--index` also restores staged state. We keep the stash
        // (apply, not pop) so the checkpoint survives for a later `undo`.
        match self.try_restore(&sha).await {
            Ok(()) => Ok(sha),
            Err(e) => {
                // The stash is safe - work is recoverable. Tell the caller how.
                Err(format!(
                    "Checkpoint saved as {sha} but auto-restore of the working tree failed: {e}. \
                     Recover with: git stash apply --index {sha}"
                ))
            }
        }
    }

    /// Resolve the SHA of the current top-of-stack stash (`refs/stash`).
    async fn resolve_head_stash_sha(&self) -> Result<String, String> {
        let out = self
            .git()
            .args(["rev-parse", "refs/stash"])
            .output()
            .await
            .map_err(|e| format!("git rev-parse failed: {}", e))?;
        if !out.status.success() {
            return Err(format!(
                "git rev-parse refs/stash failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    /// Restore the working tree from a stash SHA without dropping it.
    ///
    /// Tries `--index` first (restores staged state); on failure falls back to
    /// a plain apply (working tree only) so we still recover the bulk of the
    /// work if index restoration happens to conflict.
    async fn try_restore(&self, _sha: &str) -> Result<(), String> {
        // The stash we just pushed is at stash@{0}; restore from there.
        let with_index = self
            .git()
            .args(["stash", "apply", "--index", "stash@{0}"])
            .output()
            .await
            .map_err(|e| format!("git stash apply --index failed: {}", e))?;
        if with_index.status.success() {
            return Ok(());
        }
        // Fallback: restore working tree only (drop index restoration).
        let plain = self
            .git()
            .args(["stash", "apply", "stash@{0}"])
            .output()
            .await
            .map_err(|e| format!("git stash apply failed: {}", e))?;
        if plain.status.success() {
            // Index state was not restored; report as a soft failure so the
            // caller knows staged changes are now unstaged.
            return Err(
                "working tree restored, but staged state could not be re-applied (--index conflict)"
                    .to_string(),
            );
        }
        Err(format!(
            "git stash apply failed: {}",
            String::from_utf8_lossy(&plain.stderr).trim()
        ))
    }

    /// Undo a checkpoint: apply it back onto the working tree and drop it.
    ///
    /// If `checkpoint_id` (a SHA returned by [`create`](Self::create)) is
    /// supplied, that specific stash is targeted. Otherwise the most recent
    /// `wgenty-checkpoint` stash is used. Targeting by SHA avoids accidentally
    /// popping an unrelated stash that happens to sit on top of the stack.
    ///
    /// On apply conflict, `git stash pop` preserves the stash, so the
    /// checkpoint is not lost - the error tells the caller to resolve and
    /// retry.
    pub async fn undo(&self, checkpoint_id: Option<&str>) -> Result<String, String> {
        let target = match checkpoint_id {
            Some(sha) => self
                .resolve_stash_ref(sha)
                .await?
                .ok_or_else(|| format!("No stash found matching checkpoint SHA {sha}"))?,
            None => self
                .latest_wgenty_stash()
                .await?
                .ok_or_else(|| "No wgenty checkpoint to undo".to_string())?,
        };

        let output = self
            .git()
            .args(["stash", "pop", &target])
            .output()
            .await
            .map_err(|e| format!("git stash pop failed: {}", e))?;
        if !output.status.success() {
            let combined = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            if combined.contains("No stash entries found") || combined.contains("not a stash") {
                return Err("No matching checkpoint to undo".to_string());
            }
            // Conflict: pop leaves the stash in place, so the checkpoint is
            // preserved. Instruct the caller to resolve and retry.
            return Err(format!(
                "Undo failed - stash preserved at {target}. Resolve conflicts and retry, \
                 or inspect with `git stash show -p {target}`. Details: {}",
                combined.trim()
            ));
        }
        Ok(format!(
            "Checkpoint restored and dropped ({target}):\n{}",
            String::from_utf8_lossy(&output.stdout)
        ))
    }

    /// Map a stash commit SHA to its `stash@{n}` ref, if present.
    async fn resolve_stash_ref(&self, sha: &str) -> Result<Option<String>, String> {
        let list = self.stash_list_raw().await?;
        for line in list.lines() {
            // Format: "stash@{0} <full-sha> <subject>"
            let mut parts = line.splitn(3, ' ');
            if let (Some(gd), Some(hash), _) = (parts.next(), parts.next(), parts.next()) {
                if hash == sha {
                    return Ok(Some(gd.to_string()));
                }
            }
        }
        Ok(None)
    }

    /// Return the `stash@{n}` ref of the most recent `wgenty-checkpoint` stash.
    async fn latest_wgenty_stash(&self) -> Result<Option<String>, String> {
        let list = self.stash_list_raw().await?;
        for line in list.lines() {
            // The subject (3rd field) starts with "wgenty-checkpoint-N: ...".
            if line.contains("wgenty-checkpoint-") {
                if let Some(gd) = line.split(' ').next() {
                    return Ok(Some(gd.to_string()));
                }
            }
        }
        Ok(None)
    }

    /// Raw `git stash list` output as `stash@{n} <sha> <subject>` lines.
    async fn stash_list_raw(&self) -> Result<String, String> {
        let out = self
            .git()
            .args(["stash", "list", "--format=%gd %H %gs"])
            .output()
            .await
            .map_err(|e| format!("git stash list failed: {}", e))?;
        if !out.status.success() {
            return Err(format!(
                "git stash list failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }

    /// List all wgenty checkpoints (`stash@{n} <sha> <subject>`).
    pub async fn list(&self) -> Result<String, String> {
        let out = self
            .git()
            .args([
                "stash",
                "list",
                "--grep=wgenty-checkpoint",
                "--format=%gd %H %gs",
            ])
            .output()
            .await
            .map_err(|e| format!("git stash list failed: {}", e))?;
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }
}

pub struct CheckpointTool {
    manager: std::sync::Arc<CheckpointManager>,
}

impl CheckpointTool {
    pub fn new(manager: std::sync::Arc<CheckpointManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for CheckpointTool {
    fn name(&self) -> &str {
        "checkpoint"
    }
    fn description(&self) -> &str {
        "Create a git stash checkpoint before potentially destructive operations. Returns the stash commit SHA as the checkpoint ID. Non-destructive: the working tree is left untouched."
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "description": { "type": "string", "description": "Why this checkpoint is being created" }
            },
            "required": ["description"]
        })
    }
    async fn execute(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let desc = input["description"].as_str().unwrap_or("checkpoint");
        match self.manager.create(desc).await {
            Ok(sha) => Ok(ToolOutput {
                output_type: "text".to_string(),
                content: format!("Checkpoint created: {sha}"),
                metadata: std::collections::HashMap::new(),
            }),
            Err(e) => Err(ToolError {
                message: e,
                code: None,
            }),
        }
    }
}

pub struct UndoTool {
    manager: std::sync::Arc<CheckpointManager>,
}

impl UndoTool {
    pub fn new(manager: std::sync::Arc<CheckpointManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for UndoTool {
    fn name(&self) -> &str {
        "undo"
    }
    fn description(&self) -> &str {
        "Undo the most recent checkpoint, restoring files to their previous state via git stash pop. Pass `checkpoint_id` (the SHA returned by `checkpoint`) to target a specific checkpoint; omit it to restore the latest wgenty checkpoint."
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "checkpoint_id": {
                    "type": "string",
                    "description": "Optional stash SHA returned by the `checkpoint` tool. Targets that specific checkpoint; if omitted, the most recent wgenty checkpoint is used."
                }
            }
        })
    }
    async fn execute(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let id = input["checkpoint_id"].as_str();
        match self.manager.undo(id).await {
            Ok(output) => Ok(ToolOutput {
                output_type: "text".to_string(),
                content: format!("Checkpoint restored:\n{output}"),
                metadata: std::collections::HashMap::new(),
            }),
            Err(e) => Err(ToolError {
                message: e,
                code: None,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// Initialize a temp git repo with an initial commit; return the temp dir.
    fn init_test_repo() -> tempfile::TempDir {
        let tmp = tempfile::TempDir::new().expect("create temp dir");
        let run = |args: &[&str]| {
            let status = std::process::Command::new("git")
                .args(args)
                .current_dir(tmp.path())
                .output()
                .expect("git command runs");
            assert!(
                status.status.success(),
                "git {:?} failed: {}",
                args,
                String::from_utf8_lossy(&status.stderr)
            );
        };
        run(&["init", "-q"]);
        run(&["config", "user.name", "test"]);
        run(&["config", "user.email", "test@test.com"]);
        // Seed an initial commit so there is a HEAD to stash against.
        std::fs::write(tmp.path().join("seed.txt"), "init\n").expect("write seed");
        run(&["add", "seed.txt"]);
        run(&["commit", "-q", "-m", "init"]);
        tmp
    }

    /// `git status --short` in the repo (for assertions).
    fn status_short(dir: &Path) -> String {
        let out = std::process::Command::new("git")
            .args(["status", "--short"])
            .current_dir(dir)
            .output()
            .expect("git status");
        String::from_utf8_lossy(&out.stdout).to_string()
    }

    #[tokio::test]
    async fn create_leaves_working_tree_untouched() {
        let tmp = init_test_repo();
        let dir = tmp.path();
        // Tracked modification + staged new file + untracked file.
        std::fs::write(dir.join("seed.txt"), "init\nmodified\n").expect("modify tracked");
        std::fs::write(dir.join("staged.txt"), "staged\n").expect("write staged");
        std::fs::write(dir.join("untracked.txt"), "untracked\n").expect("write untracked");
        let run = |args: &[&str]| {
            let s = std::process::Command::new("git")
                .args(args)
                .current_dir(dir)
                .output()
                .expect("git");
            assert!(s.status.success(), "{:?}", args);
        };
        run(&["add", "staged.txt"]);
        let before = status_short(dir);

        let mgr = CheckpointManager::new_with_workdir(dir);
        let sha = mgr
            .create("before risky op")
            .await
            .expect("create succeeds");

        // SHA is a non-empty hex string.
        assert!(!sha.is_empty(), "checkpoint ID should be a SHA, got empty");
        assert!(
            sha.chars().all(|c| c.is_ascii_hexdigit()),
            "not a hex SHA: {sha}"
        );

        // The whole point: working tree is byte-for-byte unchanged.
        assert_eq!(
            status_short(dir),
            before,
            "create must not alter the working tree"
        );

        // And a stash was actually created.
        let list = mgr.list().await.expect("list");
        assert!(
            list.contains("wgenty-checkpoint-1"),
            "stash list should contain the checkpoint: {list}"
        );
    }

    #[tokio::test]
    async fn create_on_clean_tree_reports_no_changes() {
        let tmp = init_test_repo();
        let mgr = CheckpointManager::new_with_workdir(tmp.path());
        let err = mgr
            .create("nothing to checkpoint")
            .await
            .expect_err("should fail");
        assert!(err.contains("No local changes"), "unexpected error: {err}");
        // No stray stash should be left behind.
        assert!(mgr.list().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn undo_restores_after_destructive_reset() {
        let tmp = init_test_repo();
        let dir = tmp.path();
        std::fs::write(dir.join("seed.txt"), "init\nwork-in-progress\n").expect("modify");
        std::fs::write(dir.join("new.txt"), "new\n").expect("untracked");

        let mgr = CheckpointManager::new_with_workdir(dir);
        let sha = mgr.create("before reset").await.expect("create");

        // Simulate a destructive operation: discard everything back to HEAD.
        let reset = std::process::Command::new("git")
            .args(["checkout", "--", "."])
            .current_dir(dir)
            .output()
            .expect("git checkout");
        assert!(reset.status.success());
        let _ = std::fs::remove_file(dir.join("new.txt"));
        assert!(
            status_short(dir).is_empty(),
            "tree should be clean after destructive op"
        );

        // undo should bring the work back.
        let restored = mgr.undo(Some(&sha)).await.expect("undo restores");
        assert!(restored.contains("restored"), "{restored}");
        assert!(
            status_short(dir).contains("new.txt"),
            "untracked file should be restored"
        );
        assert!(
            std::fs::read_to_string(dir.join("seed.txt"))
                .unwrap()
                .contains("work-in-progress"),
            "tracked modification should be restored"
        );

        // Stash consumed by the pop.
        assert!(
            mgr.list().await.unwrap().is_empty(),
            "stash should be dropped after undo"
        );
    }

    #[tokio::test]
    async fn undo_without_id_targets_latest_wgenty_checkpoint() {
        let tmp = init_test_repo();
        let dir = tmp.path();
        std::fs::write(dir.join("seed.txt"), "init\nv1\n").expect("modify");
        let mgr = CheckpointManager::new_with_workdir(dir);
        let sha = mgr.create("latest").await.expect("create");

        // Discard, then undo without an explicit id.
        let _ = std::process::Command::new("git")
            .args(["checkout", "--", "."])
            .current_dir(dir)
            .output()
            .expect("git checkout");
        mgr.undo(None).await.expect("undo by latest");
        assert!(std::fs::read_to_string(dir.join("seed.txt"))
            .unwrap()
            .contains("v1"));
        let _ = sha; // also created via SHA; both paths exercised
    }

    #[tokio::test]
    async fn undo_unknown_sha_errors_without_touching_tree() {
        let tmp = init_test_repo();
        let dir = tmp.path();
        std::fs::write(dir.join("seed.txt"), "init\nkeep\n").expect("modify");
        let before = status_short(dir);
        let mgr = CheckpointManager::new_with_workdir(dir);
        let err = mgr
            .undo(Some("0000000000000000000000000000000000000000"))
            .await
            .expect_err("unknown sha should fail");
        assert!(err.contains("No stash"), "unexpected error: {err}");
        // Tree untouched on failure.
        assert_eq!(status_short(dir), before);
    }
}
