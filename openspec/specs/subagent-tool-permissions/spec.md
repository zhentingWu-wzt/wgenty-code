# subagent-tool-permissions Specification

## Purpose
TBD - created by archiving change subagent-permission-hardening. Update Purpose after archive.
## Requirements
### Requirement: Unified tool execution path for all agents

All agents (root and subagents) SHALL execute tools through a shared permission pipeline that includes (1) tool visibility filtering, (2) `ToolPermissionPolicy` validation, (3) guardian checks for shell/exec tools where applicable, and (4) execution with hooks/sandbox. Subagents SHALL NOT bypass policy validation by calling the tool registry directly for guarded operations.

#### Scenario: Subagent write outside workspace is not silently allowed

- **WHEN** a subagent invokes `file_write` with a path outside the workspace root
- **THEN** the call SHALL be subject to the same policy decision family as the root agent (`Ask` or deny/escalate per configuration)
- **AND** the system SHALL NOT execute the write solely because the caller is a subagent

#### Scenario: Subagent dangerous command passes through guardian

- **WHEN** a subagent invokes `execute_command` / `exec_command` with a command classified as high/critical risk
- **THEN** guardian evaluation SHALL run before execution
- **AND** critical auto-deny configuration SHALL apply consistently with the main agent path

### Requirement: Ask decisions have a resolved recipient

When policy returns `Ask` for a subagent tool call, the system SHALL resolve the decision via one of: matching session approval rules, configured escalation (`escalate_to_user` or `escalate_to_parent`), or explicit deny. The system SHALL NOT leave `Ask` hanging without a decision path. On approval timeout, the system SHALL fail closed (deny) by default.

#### Scenario: Session rule allows without re-prompt

- **WHEN** policy returns `Ask` with a `session_rule` already approved for the session
- **THEN** the tool call SHALL proceed without a new user/parent prompt

#### Scenario: Escalation timeout denies

- **WHEN** policy returns `Ask` and escalation is waiting for an approval response
- **AND** no response arrives before `approval_timeout_secs`
- **THEN** the tool call SHALL fail with a permission denial
- **AND** the side effect of the tool SHALL NOT execute

#### Scenario: Headless without escalation target denies

- **WHEN** policy returns `Ask` and no interactive user/parent approval path is available
- **THEN** the tool call SHALL fail closed with an explicit permission-related error code

### Requirement: Structured approval requests

Escalated approval requests originating from policy `Ask` SHALL include structured fields sufficient for a human or parent agent to decide, including at least: requesting agent id, request id, tool name, policy reason, and session_rule key. Free-text-only requests remain parseable for compatibility but new escalations SHALL emit structured data.

#### Scenario: Policy ask emits structured approval

- **WHEN** a subagent tool call is escalated due to policy `Ask`
- **THEN** the approval request SHALL include tool name, policy reason, and session_rule
- **AND** SHALL include request_id for correlation

### Requirement: Root consumes approval requests for user decisions

When `ask_strategy` is `escalate_to_user`, the root/session UI (or equivalent daemon bridge) SHALL be able to present pending structured approval requests to the user and write an `ApprovalResponse` (or equivalent) that unblocks the waiting subagent. The system SHALL NOT rely solely on the main LLM reading free-text team-inbox content to complete the approval loop.

#### Scenario: User approves escalated request

- **WHEN** a subagent is blocked on an escalated policy Ask
- **AND** the user chooses allow (once or always) in the root permission UI
- **THEN** the subagent tool call SHALL resume and execute if still valid
- **AND** always-allow SHALL record the session_rule for future matching

#### Scenario: User denies escalated request

- **WHEN** a subagent is blocked on an escalated policy Ask
- **AND** the user chooses deny
- **THEN** the subagent SHALL receive a failed tool result indicating permission denial
- **AND** the tool side effect SHALL NOT execute

### Requirement: Role-enforced tool visibility for explore and plan

When `explore_readonly` is enabled (default true), `explore` and `plan` subagents SHALL NOT have mutating filesystem tools such as `file_write`, `file_edit`, and `apply_patch` in their allowed tool set. `general-purpose` subagents MAY retain the full tool set subject to depth limits on spawn tools and the unified permission pipeline.

#### Scenario: Explore cannot call file_write

- **WHEN** an `explore` subagent attempts to call `file_write` with `explore_readonly=true`
- **THEN** the call SHALL fail as not allowed for that agent type before execution

#### Scenario: Explore can call file_read

- **WHEN** an `explore` subagent calls `file_read` on a path inside the workspace
- **THEN** the tool SHALL be visible and proceed through the unified permission pipeline

### Requirement: Permission outcomes are observable to parents

Permission denials and approval escalations that occur during a subagent run SHALL be recorded in subagent progress/events. When a subagent finishes, the parent-facing summary or progress snapshot SHALL include a concise indication if permission denials occurred (count and/or recent reasons), without requiring the parent to re-read the full child transcript.

#### Scenario: Denials summarized for parent

- **WHEN** a subagent experiences one or more permission denials during its run
- **AND** the subagent reaches a terminal state
- **THEN** parent-visible progress or summary material SHALL indicate that permission denials occurred

### Requirement: Configurable subagent permission defaults

The system SHALL expose configuration for subagent permission mode, ask strategy, explore readonly enforcement, approval timeout, and timeout decision (default deny). When `permission_mode` is unset/null, the subagent SHALL follow the root session permission mode. Defaults SHALL be: follow root mode, escalate-to-user for Ask, explore readonly on, timeout deny.

#### Scenario: Defaults follow root mode and fail closed on ask timeout

- **WHEN** no user overrides are set for subagent permission settings
- **THEN** the subagent permission mode SHALL follow the root session mode
- **AND** explore readonly SHALL be enabled
- **AND** Ask timeout SHALL deny
- **AND** ask strategy SHALL escalate to the user by default

#### Scenario: Optional mode override

- **WHEN** `agent.subagent.permission_mode` is set to a concrete mode
- **THEN** the subagent SHALL use that mode instead of the root session mode

