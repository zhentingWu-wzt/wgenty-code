//! Git worktree isolation for subagents (s12).
//!
//! Creates a dedicated worktree + branch for a subagent so parallel work
//! doesn't collide on the main checkout. Provides a RAII guard that removes
//! the worktree on drop (best-effort; `remove --force`).
//!
//! NOTE: wiring the worktree path as a subagent's working directory requires
//! per-subagent cwd support in the tool layer (currently global cwd). Until
//! that lands, this module manages worktree *lifecycle* only; the path is
//! returned for callers that can scope their own commands.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A worktree created for an isolated subagent. Drop to remove.
#[derive(Debug)]
pub struct WorktreeIsolation {
    /// Absolute path to the worktree checkout.
    pub path: PathBuf,
    /// Branch created for this worktree.
    pub branch: String,
    /// Repo root the worktree was created from.
    repo_root: PathBuf,
}

impl WorktreeIsolation {
    /// Create a worktree at `.wgenty-worktrees/{id}` on branch
    /// `wgenty/{id}`, based off `base_ref` (default: HEAD).
    ///
    /// `repo_root` is the existing git repository to add into.
    pub fn create(repo_root: &Path, id: &str, base_ref: Option<&str>) -> Result<Self, String> {
        let branch = format!("wgenty/{id}");
        let worktree_dir = repo_root.join(".wgenty-worktrees").join(id);
        if worktree_dir.exists() {
            return Err(format!(
                "worktree target already exists: {}",
                worktree_dir.display()
            ));
        }

        let base = base_ref.unwrap_or("HEAD");
        // `git worktree add -b <branch> <path> <base>`
        let out = Command::new("git")
            .arg("worktree")
            .arg("add")
            .arg("-b")
            .arg(&branch)
            .arg(&worktree_dir)
            .arg(base)
            .current_dir(repo_root)
            .output()
            .map_err(|e| format!("failed to run git: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "git worktree add failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }

        Ok(Self {
            path: worktree_dir,
            branch,
            repo_root: repo_root.to_path_buf(),
        })
    }

    /// Force-remove the worktree and its branch. Safe to call multiple times.
    pub fn remove(&self) -> Result<(), String> {
        let out = Command::new("git")
            .arg("worktree")
            .arg("remove")
            .arg("--force")
            .arg(&self.path)
            .current_dir(&self.repo_root)
            .output()
            .map_err(|e| format!("failed to run git: {e}"))?;
        if !out.status.success() {
            tracing::warn!(
                stderr = %String::from_utf8_lossy(&out.stderr).trim(),
                "worktree remove failed; leaving {} for manual cleanup",
                self.path.display()
            );
        }
        // Best-effort branch delete.
        let _ = Command::new("git")
            .arg("branch")
            .arg("-D")
            .arg(&self.branch)
            .current_dir(&self.repo_root)
            .output();
        Ok(())
    }

    /// List leftover `.wgenty-worktrees/*` entries (for daemon-startup recovery).
    pub fn list_leftovers(repo_root: &Path) -> Vec<PathBuf> {
        let dir = repo_root.join(".wgenty-worktrees");
        match std::fs::read_dir(&dir) {
            Ok(entries) => entries
                .flatten()
                .filter(|e| e.path().is_dir())
                .map(|e| e.path())
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Reclaim all leftover worktrees under `.wgenty-worktrees/` (startup recovery).
    /// Returns the count removed.
    pub fn reclaim_all(repo_root: &Path) -> usize {
        let leftovers = Self::list_leftovers(repo_root);
        let mut removed = 0;
        for path in &leftovers {
            let out = Command::new("git")
                .arg("worktree")
                .arg("remove")
                .arg("--force")
                .arg(path)
                .current_dir(repo_root)
                .output();
            if let Ok(o) = out {
                if o.status.success() {
                    removed += 1;
                }
            }
        }
        removed
    }
}

impl Drop for WorktreeIsolation {
    fn drop(&mut self) {
        let _ = self.remove();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Init a bare-ish git repo in a temp dir with one commit on HEAD.
    fn init_repo() -> tempfile::TempDir {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        for (k, v) in [("user.name", "test"), ("user.email", "t@t.test")] {
            let _ = Command::new("git").args(["config", "--global", k, v]).status();
        }
        let _ = Command::new("git")
            .arg("init")
            .current_dir(root)
            .status();
        std::fs::write(root.join("README.md"), "init").unwrap();
        let _ = Command::new("git")
            .args(["add", "."])
            .current_dir(root)
            .status();
        let _ = Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(root)
            .status();
        tmp
    }

    #[test]
    fn create_and_drop_removes_worktree() {
        let tmp = init_repo();
        let root = tmp.path().to_path_buf();
        {
            let wt = WorktreeIsolation::create(&root, "agent-1", None).unwrap();
            assert!(wt.path.exists());
            assert!(wt.path.join(".git").exists());
            // scoped: drop removes it
        }
        assert!(!root.join(".wgenty-worktrees").join("agent-1").exists());
    }

    #[test]
    fn duplicate_create_fails() {
        let tmp = init_repo();
        let root = tmp.path().to_path_buf();
        let _wt = WorktreeIsolation::create(&root, "dup", None).unwrap();
        let err = WorktreeIsolation::create(&root, "dup", None).unwrap_err();
        assert!(err.contains("already exists"));
    }

    #[test]
    fn reclaim_all_clears_leftovers() {
        let tmp = init_repo();
        let root = tmp.path().to_path_buf();
        // Create a worktree then leak it (forget the guard).
        let wt = WorktreeIsolation::create(&root, "leak", None).unwrap();
        let path = wt.path.clone();
        std::mem::forget(wt);
        assert!(path.exists());

        let removed = WorktreeIsolation::reclaim_all(&root);
        assert!(removed >= 1);
        assert!(!path.exists());
    }
}
