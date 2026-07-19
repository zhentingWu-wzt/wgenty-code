//! Git state recording for turn boundaries.
//!
//! At [`super::coordinator::SessionCoordinator::begin_turn`], the coordinator
//! captures `HEAD` and the untracked-file list so that rollback (Task 4) can
//! `git reset --hard` to the turn-start SHA and delete turn-new untracked
//! files. Non-git projects degrade gracefully: [`record_git_state`] returns
//! `(None, Vec::new())` and file rollback via
//! [`crate::tools::checkpoint_store::CheckpointStore`] still works.
//!
//! Spec §3.2: the recording happens at the turn boundary (before any tool in
//! the turn runs), not before each tool call.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};

use super::session::GitRefs;

/// Run `git` with `args` in `project_root` and return stdout (trimmed). Errors
/// if git is missing, the dir is not inside a repo, or git exits non-zero.
pub(crate) fn run_git(project_root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(project_root)
        .output()
        .with_context(|| format!("spawn git {:?}", args))?;
    if !output.status.success() {
        anyhow::bail!(
            "git {:?} failed (exit {:?}): {}",
            args,
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Capture `(HEAD sha, untracked files)` at a turn boundary.
///
/// Degrades to `(None, [])` when `project_root` is not inside a git repo, the
/// repo has no commits yet, or git is unavailable. The caller proceeds without
/// git-ref protection; file rollback via `CheckpointStore` still applies.
///
/// `untracked_files` are relative paths (as `git ls-files` prints them), one
/// per entry, excluding gitignored files (`--exclude-standard`).
pub fn record_git_state(project_root: &Path) -> (Option<GitRefs>, Vec<String>) {
    let head = run_git(project_root, &["rev-parse", "HEAD"])
        .ok()
        .filter(|s| !s.is_empty())
        .map(|sha| GitRefs { head: sha });

    let untracked = run_git(
        project_root,
        &["ls-files", "--others", "--exclude-standard"],
    )
    .map(|out| {
        out.lines()
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect()
    })
    .unwrap_or_default();

    (head, untracked)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// Initialize a git repo with one commit so `HEAD` exists. Writes a
    /// `.gitignore` so test scaffolding does not pollute the untracked list.
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
        std::fs::write(dir.join(".gitignore"), "*.tmp\n").unwrap();
        std::fs::write(dir.join("seed.txt"), "seed\n").unwrap();
        let status = Command::new("git")
            .args(["add", "."])
            .current_dir(dir)
            .status()
            .expect("git add");
        assert!(status.success(), "git add failed");
        let status = Command::new("git")
            .args(["commit", "-m", "seed"])
            .current_dir(dir)
            .status()
            .expect("git commit");
        assert!(status.success(), "git commit failed");
    }

    #[test]
    fn non_git_dir_returns_none_and_empty() {
        let dir = tempdir().unwrap();
        let (refs, untracked) = record_git_state(dir.path());
        assert!(refs.is_none(), "non-git dir should yield no HEAD");
        assert!(
            untracked.is_empty(),
            "non-git dir should yield no untracked"
        );
    }

    #[test]
    fn git_repo_records_head_sha() {
        let dir = tempdir().unwrap();
        init_git_repo(dir.path());
        let (refs, _untracked) = record_git_state(dir.path());
        let refs = refs.expect("git repo should yield HEAD");
        assert!(!refs.head.is_empty(), "HEAD sha should be non-empty");
        assert!(
            refs.head.chars().all(|c| c.is_ascii_hexdigit()),
            "HEAD should be hex: {}",
            refs.head
        );
    }

    #[test]
    fn git_repo_lists_untracked_files() {
        let dir = tempdir().unwrap();
        init_git_repo(dir.path());
        // `*.tmp` is gitignored, so use a different extension for the
        // untracked file we expect to see.
        std::fs::write(dir.path().join("scratch.log"), "x\n").unwrap();
        std::fs::create_dir_all(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub").join("new.log"), "y\n").unwrap();
        // This one is gitignored - must NOT appear.
        std::fs::write(dir.path().join("ignored.tmp"), "z\n").unwrap();
        let (_refs, untracked) = record_git_state(dir.path());
        assert!(
            untracked.iter().any(|f| f == "scratch.log"),
            "scratch.log should be listed: {:?}",
            untracked
        );
        assert!(
            untracked.iter().any(|f| f == "sub/new.log"),
            "sub/new.log should be listed: {:?}",
            untracked
        );
        assert!(
            !untracked.iter().any(|f| f == "seed.txt"),
            "seed.txt is tracked, must not be untracked: {:?}",
            untracked
        );
        assert!(
            !untracked.iter().any(|f| f == "ignored.tmp"),
            "ignored.tmp is gitignored, must not appear: {:?}",
            untracked
        );
    }

    #[test]
    fn empty_repo_no_commits_yields_none_head() {
        let dir = tempdir().unwrap();
        let status = Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .status()
            .expect("git init");
        assert!(status.success(), "git init failed");
        let (refs, _untracked) = record_git_state(dir.path());
        assert!(
            refs.is_none(),
            "repo with no commits should degrade to None"
        );
    }
}
