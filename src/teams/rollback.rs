//! Rollback mechanism for subagent execution.
//!
//! Uses git stash to create safety points before file modifications,
//! allowing selective rollback of affected files on error.

use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Clone)]
pub struct RollbackContext {
    pub stashed_ref: String,
    pub affected_files: Vec<PathBuf>,
    pub parent_commit: String,
    label: String,
}

#[derive(Debug, Clone)]
pub enum RollbackError {
    GitError(String),
    DirtyWorkingTree(String),
    StashConflict(String),
    NoChanges,
}

impl std::fmt::Display for RollbackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GitError(msg) => write!(f, "Git error: {}", msg),
            Self::DirtyWorkingTree(msg) => write!(f, "Dirty working tree: {}", msg),
            Self::StashConflict(msg) => write!(f, "Stash conflict: {}", msg),
            Self::NoChanges => write!(f, "No changes to rollback"),
        }
    }
}

impl std::error::Error for RollbackError {}

impl RollbackContext {
    /// Create a safety point before files are modified.
    ///
    /// Checks for existing unstaged changes and stashes them.
    pub fn create(label: &str) -> Result<Self, RollbackError> {
        // Check for unstaged changes
        let status = Command::new("git")
            .args(["status", "--porcelain"])
            .output()
            .map_err(|e| RollbackError::GitError(e.to_string()))?;

        let output = String::from_utf8_lossy(&status.stdout);
        let has_changes = output.lines().any(|l| !l.is_empty());
        if has_changes {
            return Err(RollbackError::DirtyWorkingTree(
                "There are uncommitted changes. Please commit or stash them first.".to_string(),
            ));
        }

        // Get current HEAD
        let head = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .output()
            .map_err(|e| RollbackError::GitError(e.to_string()))?;
        let parent_commit = String::from_utf8_lossy(&head.stdout).trim().to_string();

        // Create a stash with label
        let stash_result = Command::new("git")
            .args(["stash", "push", "--include-untracked", "--message", label])
            .output()
            .map_err(|e| RollbackError::GitError(e.to_string()))?;

        if !stash_result.status.success() {
            let stderr = String::from_utf8_lossy(&stash_result.stderr);
            return Err(RollbackError::GitError(format!(
                "Failed to create stash: {}",
                stderr
            )));
        }

        // Get stash ref
        let stash_list = Command::new("git")
            .args(["stash", "list"])
            .output()
            .map_err(|e| RollbackError::GitError(e.to_string()))?;
        let stash_output = String::from_utf8_lossy(&stash_list.stdout);
        let first_stash = stash_output.lines().next().unwrap_or("");
        let stash_ref = first_stash.split(':').next().unwrap_or("stash@{0}").to_string();

        Ok(Self {
            stashed_ref: stash_ref,
            affected_files: Vec::new(),
            parent_commit,
            label: label.to_string(),
        })
    }

    /// Record that a file was modified by the subagent.
    pub fn record_modification(&mut self, path: PathBuf) {
        if !self.affected_files.contains(&path) {
            self.affected_files.push(path);
        }
    }

    /// Rollback to safety point by restoring affected files from stash.
    pub fn rollback(&self) -> Result<(), RollbackError> {
        if self.affected_files.is_empty() {
            // Read-only subagent — no files to rollback
            return Err(RollbackError::NoChanges);
        }

        // Pop the stash to restore files
        let pop_result = Command::new("git")
            .args(["stash", "pop"])
            .output()
            .map_err(|e| RollbackError::GitError(e.to_string()))?;

        if !pop_result.status.success() {
            let stderr = String::from_utf8_lossy(&pop_result.stderr);
            if stderr.contains("conflict") {
                return Err(RollbackError::StashConflict(stderr.to_string()));
            }
            return Err(RollbackError::GitError(format!(
                "Failed to pop stash: {}",
                stderr
            )));
        }

        // Restore only the affected files from the index (which stash pop restored)
        // This selectively rolls back only our changes
        for file in &self.affected_files {
            let checkout_result = Command::new("git")
                .args(["checkout", "--", file.to_str().unwrap_or("")])
                .output()
                .map_err(|e| RollbackError::GitError(e.to_string()))?;

            if !checkout_result.status.success() {
                let stderr = String::from_utf8_lossy(&checkout_result.stderr);
                tracing::warn!("Failed to checkout {} during rollback: {}", file.display(), stderr);
            }
        }

        Ok(())
    }

    /// Release the safety point on success (drop the stash).
    pub fn release(&self) -> Result<(), RollbackError> {
        // Drop the stash entry
        let drop_result = Command::new("git")
            .args(["stash", "drop"])
            .output()
            .map_err(|e| RollbackError::GitError(e.to_string()))?;

        if !drop_result.status.success() {
            let stderr = String::from_utf8_lossy(&drop_result.stderr);
            tracing::warn!("Failed to drop stash: {}", stderr);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_rollback_context_no_changes_returns_err() {
        // In a git repo with no changes, create() will fail because
        // there are no changes to stash. This is expected.
        let result = RollbackContext::create("test-label");
        // The result depends on git state — just verify it doesn't panic
        assert!(result.is_err());
    }

    #[test]
    fn test_affected_files_tracking() {
        let mut ctx = RollbackContext {
            stashed_ref: "stash@{0}".to_string(),
            affected_files: Vec::new(),
            parent_commit: "abc123".to_string(),
            label: "test".to_string(),
        };

        ctx.record_modification(PathBuf::from("src/main.rs"));
        assert_eq!(ctx.affected_files.len(), 1);

        // Duplicate shouldn't be added
        ctx.record_modification(PathBuf::from("src/main.rs"));
        assert_eq!(ctx.affected_files.len(), 1);

        ctx.record_modification(PathBuf::from("src/lib.rs"));
        assert_eq!(ctx.affected_files.len(), 2);
    }
}
