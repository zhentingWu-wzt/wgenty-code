//! Git worktree endpoints for the web command center's WorktreePanel.
//! Thin wrappers around `git worktree` run in the daemon's working_dir.

use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct WorktreeInfo {
    pub path: String,
    pub head: String,
    pub branch: Option<String>,
    /// The first entry in `git worktree list` is always the main worktree.
    pub is_main: bool,
}

/// Parse `git worktree list --porcelain` output. Blocks are separated by blank
/// lines; `branch` is absent for detached HEAD entries.
pub(crate) fn parse_worktree_list(input: &str) -> Vec<WorktreeInfo> {
    let mut out = Vec::new();
    let mut path: Option<String> = None;
    let mut head = String::new();
    let mut branch: Option<String> = None;

    let mut flush = |path: &mut Option<String>, head: &mut String, branch: &mut Option<String>| {
        if let Some(p) = path.take() {
            out.push(WorktreeInfo {
                path: p,
                head: std::mem::take(head),
                branch: branch.take(),
                is_main: out.is_empty(), // first block = main worktree
            });
        }
    };

    for line in input.lines() {
        if line.is_empty() {
            flush(&mut path, &mut head, &mut branch);
        } else if let Some(p) = line.strip_prefix("worktree ") {
            path = Some(p.to_string());
        } else if let Some(h) = line.strip_prefix("HEAD ") {
            head = h.to_string();
        } else if let Some(b) = line.strip_prefix("branch refs/heads/") {
            branch = Some(b.to_string());
        }
    }
    flush(&mut path, &mut head, &mut branch);
    out
}

use crate::daemon::state::DaemonState;
use axum::{extract::State, http::StatusCode, Json};
use std::sync::Arc;

/// Run `git` in the daemon's working_dir; on non-zero exit return the stderr
/// text so the web panel can show why (e.g. "already exists").
pub(crate) async fn git(
    args: &[&str],
    state: &DaemonState,
) -> Result<String, (StatusCode, String)> {
    let out = tokio::process::Command::new("git")
        .args(args)
        .current_dir(&state.app_state.settings.storage.working_dir)
        .output()
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to spawn git: {e}"),
            )
        })?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err((StatusCode::BAD_REQUEST, stderr));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// GET /api/v1/worktrees — list git worktrees (main first).
pub async fn list_worktrees(
    State(state): State<Arc<DaemonState>>,
) -> Result<Json<Vec<WorktreeInfo>>, (StatusCode, String)> {
    let stdout = git(&["worktree", "list", "--porcelain"], &state).await?;
    Ok(Json(parse_worktree_list(&stdout)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_main_and_linked_worktrees() {
        let input = "worktree /repo\nHEAD 111aaa\nbranch refs/heads/main\n\nworktree /repo/.worktrees/feat\nHEAD 222bbb\nbranch refs/heads/feature/x\n\n";
        let wt = parse_worktree_list(input);
        assert_eq!(wt.len(), 2);
        assert!(wt[0].is_main);
        assert_eq!(wt[0].branch.as_deref(), Some("main"));
        assert!(!wt[1].is_main);
        assert_eq!(wt[1].path, "/repo/.worktrees/feat");
    }

    #[test]
    fn detached_head_has_no_branch() {
        let input = "worktree /repo\nHEAD 111aaa\ndetached\n\n";
        let wt = parse_worktree_list(input);
        assert_eq!(wt.len(), 1);
        assert_eq!(wt[0].branch, None);
        assert_eq!(wt[0].head, "111aaa");
    }

    #[test]
    fn empty_input_yields_empty_list() {
        assert!(parse_worktree_list("").is_empty());
    }
}
