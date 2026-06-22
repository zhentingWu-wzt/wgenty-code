# worktree-isolation-tool Specification

## Purpose
TBD - created by archiving change comet-workflow-compat. Update Purpose after archive.
## Requirements
### Requirement: Git operations supports worktree_add
The `git_operations` tool SHALL support `operation: "worktree_add"` to create a new git worktree. Required parameters: `path` (relative or absolute path under `.claude/worktrees/` or `.wgenty-code/worktrees/`), `branch` (new branch name). Optional: `base_ref` (default: `origin/main`).

#### Scenario: Create worktree with new branch from origin/main
- **WHEN** agent calls `git_operations` with `operation: "worktree_add"`, `path: ".claude/worktrees/feature-x"`, `branch: "feature/20260622/feature-x"`, `base_ref: "origin/main"`
- **THEN** `git worktree add -b feature/20260622/feature-x .claude/worktrees/feature-x origin/main` SHALL be executed
- **AND** the result SHALL include the new worktree path
- **AND** the output SHALL be the git command stdout

#### Scenario: Create worktree fails when branch already exists
- **WHEN** `base_ref` branch already has a worktree at the given path or the branch name is taken
- **THEN** the tool SHALL return a `non_zero_exit` error with the git error message

### Requirement: Git operations supports worktree_remove
The `git_operations` tool SHALL support `operation: "worktree_remove"` to remove a git worktree. Required parameter: `path`. Optional: `force` (boolean, default: `false`).

#### Scenario: Remove worktree with force when clean
- **WHEN** agent calls `git_operations` with `operation: "worktree_remove"`, `path: ".claude/worktrees/feature-x"`, `force: true`
- **THEN** `git worktree remove --force .claude/worktrees/feature-x` SHALL be executed
- **AND** the worktree directory and its branch SHALL be removed

#### Scenario: Remove with uncommitted changes without force fails
- **WHEN** the worktree has uncommitted changes or commits not on the original branch
- **AND** `force` is `false`
- **THEN** the tool SHALL return a failure indicating uncommitted changes exist
- **AND** the error SHALL suggest using `force: true` to discard changes

### Requirement: Git operations supports worktree_list
The `git_operations` tool SHALL support `operation: "worktree_list"` to list all git worktrees for the repository.

#### Scenario: List all worktrees
- **WHEN** agent calls `git_operations` with `operation: "worktree_list"`
- **THEN** `git worktree list` SHALL be executed
- **AND** the output SHALL contain one line per worktree with path, HEAD, and branch info

### Requirement: Worktree operations run in the repository root
All worktree operations SHALL execute with `current_dir` set to the repository root (where `.git` lives), regardless of the `path` argument in the tool call.

#### Scenario: Worktree command runs from repo root
- **WHEN** agent's working directory is a subdirectory
- **AND** agent calls `git_operations` with `operation: "worktree_add"`
- **THEN** the git command SHALL execute with `current_dir` set to the repository root

