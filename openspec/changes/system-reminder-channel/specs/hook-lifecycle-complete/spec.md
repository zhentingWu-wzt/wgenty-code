## MODIFIED Requirements

### Requirement: UserPromptSubmit hook fires before agent turn starts

The system SHALL fire `UserPromptSubmit` hooks inside the agent turn task and `await` their outcomes, so that injected content can be consumed by the next outgoing user message in the model request.

**Previous behavior**: hooks were fired via `tokio::spawn` from the TUI input handler the instant the user submitted, with outcomes discarded.

**New behavior**: hooks fire inside `AgentLoop::process_input_inner` at the start of each turn task. The fire is `await`-ed (not spawn-and-forget) and outcomes are passed to the reminder builder. Hook execution is bounded by a 10-second timeout; on timeout the turn proceeds with empty outcomes.

#### Scenario: Hook fires inside agent turn task
- **WHEN** the user submits a prompt and a `UserPromptSubmit` hook is configured
- **THEN** the hook SHALL fire inside the agent turn task before the user message is sent to the model
- **AND** the hook outcomes SHALL be consumed by the reminder builder for `injected_content` extraction

#### Scenario: Hook timeout degrades gracefully
- **WHEN** a `UserPromptSubmit` hook does not complete within 10 seconds
- **THEN** the system SHALL log a warning
- **AND** proceed with empty outcomes
- **AND** the user turn SHALL continue without blocking

#### Scenario: Hook does not fire on built-in commands
- **WHEN** the user input is a built-in slash command (e.g. `/help`)
- **THEN** the `UserPromptSubmit` hook SHALL NOT fire (unchanged behavior)

---

## ADDED Requirements

### Requirement: HookAction::InjectContext content reaches the next user turn

The system SHALL consume `outcomes[].injected_content` produced by hook actions (especially from `UserPromptSubmit` hooks) and surface the content to the next outgoing user message in the model request.

#### Scenario: UserPromptSubmit hook returns injected content
- **WHEN** a `UserPromptSubmit` hook is configured with a `HookAction::InjectContext` action that produces `injected_content = "<extra context>"`
- **AND** the hook fires after the user submits a prompt
- **THEN** the next outgoing user message to the model SHALL contain the string `<extra context>` accessible to the model
- **AND** the injection SHALL persist independently from static reminder file sources

#### Scenario: Multiple injecting hooks are concatenated
- **WHEN** two hooks both produce `injected_content` for the same `UserPromptSubmit` event
- **THEN** both contents SHALL be included in the next user message
- **AND** the concatenation order SHALL follow the order in which the hooks are declared in `settings.json`

#### Scenario: Hook returns no injected content
- **WHEN** a hook fires but its outcome's `injected_content` is `None` or empty
- **THEN** no extra content SHALL be added to the next user message from that hook

#### Scenario: Hook with continue_execution=false still injects
- **WHEN** a hook returns `{ continue_execution: false, injected_content: "blocked context" }`
- **THEN** the turn SHALL be blocked (per existing semantics)
- **AND** the injected content SHALL still be appended to the next user message that does eventually proceed

---

### Requirement: Injected hook content coordinates with reminder block

The system SHALL place hook-injected content in a deterministic position relative to the static `<system-reminder>` block when both are present.

#### Scenario: Reminder present, hook injects content
- **WHEN** both a non-empty `<system-reminder>` block (from file sources) and a non-empty hook `injected_content` exist for the same user turn
- **THEN** the user message content SHALL contain the `<system-reminder>` block first, followed by the hook-injected content, followed by the user's original prompt text

#### Scenario: Only hook content (no file sources)
- **WHEN** all four reminder file sources are missing but a hook produces `injected_content`
- **THEN** the user message SHALL contain a `<system-reminder>` block wrapping the hook-injected content (between the standard opening and closing preambles), followed by the user's prompt
- **AND** no orphan file-source attribution headers (`Contents of ...`) SHALL appear inside the block

#### Scenario: Only reminder block (no hook content)
- **WHEN** the reminder block is non-empty but no hook produces `injected_content`
- **THEN** the user message SHALL contain only the `<system-reminder>` block followed by the user's prompt

---

### Requirement: Inject visibility honored

The system SHALL respect the `LayerVisibility` field on `HookAction::InjectContext` when wiring content into the next user message.

#### Scenario: Visible layer reaches the model
- **WHEN** a hook injects with `visibility: Visible`
- **THEN** the content SHALL appear in the user message sent to the model

#### Scenario: Internal layer reaches the model but is flagged
- **WHEN** a hook injects with `visibility: Internal`
- **THEN** the content SHALL appear in the user message sent to the model (model can read it)
- **AND** the TUI SHALL NOT echo the content to the visible chat transcript

---

### Requirement: Inject priority orders multiple sources

When more than one hook injects content for the same user turn, the system SHALL order the contents by their `priority` field (lower priority value renders earlier), with ties broken by hook declaration order in `settings.json`.

#### Scenario: Two hooks with different priorities
- **WHEN** hook A injects with `priority: 10` and hook B injects with `priority: 1` for the same turn
- **THEN** hook B's content SHALL appear before hook A's content in the user message

#### Scenario: Two hooks with identical priorities
- **WHEN** hook A and hook B both inject with `priority: 5` for the same turn, and hook A is declared before hook B in `settings.json`
- **THEN** hook A's content SHALL appear before hook B's content
