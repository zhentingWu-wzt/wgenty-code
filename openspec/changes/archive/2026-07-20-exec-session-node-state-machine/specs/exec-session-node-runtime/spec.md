## ADDED Requirements

### Requirement: Node contract schema

The exec-session outer layer SHALL define a node contract that an agent declares when starting a verifiable work unit. A node contract consists of a human-readable goal, a list of verify commands (executed by the runtime, never trusted from agent-asserted results), and a list of expected changed files (for out-of-bounds detection). The runtime SHALL store the contract in `session.json` under a `node_states` field so node state survives across turns within a session.

#### Scenario: Agent declares a node with full contract

- **WHEN** the agent invokes `begin_node` with `{goal: "add memory clear command", verify_commands: ["cargo test --test integration memory", "cargo clippy -- -D warnings"], expected_files: ["src/cli.rs", "src/memory/list.rs"]}`
- **THEN** the runtime SHALL create a node record with status `running`
- **AND** the node contract (goal, verify_commands, expected_files) SHALL be persisted to `session.json` under `node_states`
- **AND** the node SHALL be linked to the current turn chain in the session

#### Scenario: Node contract without expected_files

- **WHEN** the agent invokes `begin_node` with `{goal: "...", verify_commands: ["cargo test"], expected_files: []}`
- **THEN** the runtime SHALL create the node with an empty expected_files list
- **AND** out-of-bounds detection SHALL be skipped (empty expected means no boundary constraint)

#### Scenario: Node contract persisted across turns

- **WHEN** a node is created in turn N and the agent continues to turn N+1 without completing the node
- **THEN** the node SHALL remain in `running` status across turns
- **AND** `session.json` SHALL retain the node record so it is available for verify or rollback in any subsequent turn

### Requirement: Node state machine

The runtime SHALL manage a node state machine with states `pending`, `running`, `verifying`, `verified`, and `failed`. A node transitions `pending -> running` on creation, `running -> verifying` when `verify_node` is invoked, `verifying -> verified` on verify success, and `verifying -> failed` on verify failure. A `failed` node MAY transition back to `running` for self-correction (AutoRetry) up to a configured maximum. Transitions SHALL be persisted atomically to `session.json`.

#### Scenario: Node transitions to verified on successful verify

- **WHEN** the agent invokes `verify_node` on a `running` node and all verify commands exit 0 and no out-of-bounds files are detected
- **THEN** the node status SHALL transition `running -> verifying -> verified`
- **AND** `session.json` SHALL be atomically updated with the new status
- **AND** the verify_log SHALL record the successful attempt

#### Scenario: Node transitions to failed on verify failure

- **WHEN** the agent invokes `verify_node` on a `running` node and a verify command exits non-zero OR out-of-bounds files are detected
- **THEN** the node status SHALL transition `running -> verifying -> failed`
- **AND** the workspace changes SHALL be preserved (no automatic rollback)
- **AND** the failure reason (which command failed, or the out-of-bounds file list) SHALL be returned to the agent

#### Scenario: Failed node self-correction within AutoRetry limit

- **WHEN** a node is `failed` and the retry count is below `auto_retry_max` (default 2)
- **THEN** the agent MAY invoke `begin_node` again or continue working and re-invoke `verify_node`
- **AND** the node SHALL transition `failed -> running` (self-correction path)
- **AND** the workspace changes from the failed attempt SHALL be preserved so the agent can inspect and fix them

#### Scenario: Failed node exceeds AutoRetry limit

- **WHEN** a node has failed more than `auto_retry_max` times
- **THEN** the session status SHALL become `failed`
- **AND** the runtime SHALL return the failure to the agent (not to any specific orchestration skill)
- **AND** the agent SHALL decide the escalation action based on its current flow (comet verify-failure handling, or user report in agent-self mode)

### Requirement: Node-level verify-gate reuses inner VerifyGate

The `verify_node` tool SHALL delegate command execution and out-of-bounds detection to the existing inner-layer `VerifyGate`. The runtime SHALL NOT re-implement command execution, guardian review, or sandbox execution. Out-of-bounds detection SHALL combine the inner layer's CheckpointStore manifest + git diff + untracked sources, scoped to the node's turn span.

#### Scenario: verify_node delegates to inner VerifyGate

- **WHEN** the agent invokes `verify_node`
- **THEN** the runtime SHALL call the inner `VerifyGate` with the node's `verify_commands` and `expected_files`
- **AND** each command SHALL pass through guardian review and sandbox execution (same as `exec_command`)
- **AND** the verify result (pass/fail, failure reason, verify_log) SHALL be produced by the inner `VerifyGate`

#### Scenario: Out-of-bounds detection scoped to node turn span

- **WHEN** `verify_node` checks for out-of-bounds changes
- **THEN** `actual_changed_files` SHALL be computed from turns belonging to the current node (node start turn to current turn)
- **AND** the check SHALL be `actual_changed_files ⊆ expected_files`
- **AND** out-of-bounds files (actual not in expected) SHALL cause verify failure with the out-of-bounds list returned to the agent

### Requirement: Node rollback to last verified node

The `rollback_node` tool SHALL roll back to the most recent `verified` node by delegating to the inner `SessionCoordinator::rollback_to` with the verified node's starting turn. The rollback algorithm (git reset --hard if head changed + CheckpointStore::rewind + delete new untracked) SHALL be reused from the inner layer without modification. Rollback SHALL only be triggered by explicit agent invocation, never automatically on verify failure.

#### Scenario: Rollback to last verified node

- **WHEN** the agent invokes `rollback_node` and there exists at least one `verified` node
- **THEN** the runtime SHALL identify the most recent `verified` node's starting turn
- **AND** SHALL delegate to `SessionCoordinator::rollback_to(that_turn)`
- **AND** the workspace SHALL be restored to the state at that verified node's start
- **AND** all nodes after the rollback target SHALL be removed from `node_states`

#### Scenario: Rollback with no verified node

- **WHEN** the agent invokes `rollback_node` and no `verified` node exists in the session
- **THEN** the runtime SHALL return an error indicating no verified node to roll back to
- **AND** no workspace changes SHALL occur

#### Scenario: Rollback preserves verified node state

- **WHEN** rollback to a verified node completes
- **THEN** the target verified node SHALL remain in `verified` status
- **AND** `session.json` SHALL reflect the removed nodes and the current node cursor at the verified node

### Requirement: Node runtime decoupled from orchestration skills

The exec-session outer layer SHALL NOT contain references to any specific orchestration skill (including comet) beyond the `SessionSource` enum variant. The runtime SHALL NOT branch behavior based on `SessionSource`. Node verify failure SHALL be returned to the agent as a tool result, and the agent (not the runtime) SHALL decide escalation based on its current flow. The `SessionHooks` trait SHALL provide `pre_node` and `post_node` hooks with default no-op implementations so callers (e.g., a comet plugin) can observe node transitions without the runtime depending on them.

#### Scenario: Runtime code has no orchestration-skill references

- **WHEN** the `src/exec_session/` source is inspected
- **THEN** no string matching an orchestration skill name (e.g., "comet") SHALL appear except in the `SessionSource::Comet` enum variant and comments/doc examples
- **AND** the runtime SHALL NOT call any orchestration-skill-specific API

#### Scenario: Verify failure returned to agent regardless of caller

- **WHEN** a node verify fails and exceeds AutoRetry limit
- **THEN** the runtime SHALL return the failure as a tool result to the agent
- **AND** the runtime SHALL NOT invoke any orchestration-skill-specific escalation API
- **AND** the agent SHALL receive the failure and decide the next action based on its active flow (skill instructions or user interaction)

#### Scenario: SessionHooks pre_node and post_node are optional

- **WHEN** a session is created with `NoHooks` (default)
- **THEN** `pre_node` and `post_node` SHALL be no-ops
- **AND** node state machine transitions SHALL proceed normally without hooks
- **WHEN** a session is created with a custom `SessionHooks` implementation
- **THEN** `pre_node` SHALL be called before a node transitions to `running`
- **AND** `post_node` SHALL be called after a node reaches `verified` or `failed`

### Requirement: Node tools registered in agent tool registry

The three node tools (`begin_node`, `verify_node`, `rollback_node`) SHALL be registered in the agent `ToolRegistry` and available to the agent when exec-session is enabled (`ExecSessionSettings.enabled = true`). The tools SHALL declare `is_read_only() = false` since they manage session state and may trigger workspace mutations (rollback). When exec-session is disabled, the tools SHALL NOT be registered.

#### Scenario: Node tools available when exec-session enabled

- **WHEN** `ExecSessionSettings.enabled = true`
- **THEN** `begin_node`, `verify_node`, and `rollback_node` SHALL be registered in the `ToolRegistry`
- **AND** each tool SHALL declare `is_read_only() = false`

#### Scenario: Node tools absent when exec-session disabled

- **WHEN** `ExecSessionSettings.enabled = false`
- **THEN** none of the node tools SHALL be registered in the `ToolRegistry`
- **AND** the agent SHALL not be able to invoke them
