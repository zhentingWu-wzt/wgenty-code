## ADDED Requirements

### Requirement: ESC interrupts a running agent turn

The TUI REPL SHALL interrupt the currently running agent turn when the user presses ESC and a turn task is live (`current_turn_handle` is present). Interrupting SHALL abort the turn's spawned task, set the agent phase to `Idle`, and enable `suppress_phase_updates` so residual phase-changing events from the aborted task do not flip the status bar back to a busy state.

#### Scenario: ESC interrupts during streaming response

- **WHEN** an agent turn is streaming an LLM response (`current_turn_handle` is present) and the user presses ESC
- **THEN** the turn task SHALL be aborted
- **AND** the agent phase SHALL become `Idle`
- **AND** the streaming indicator SHALL stop (`streaming_active = false`)

#### Scenario: ESC interrupts during tool execution

- **WHEN** an agent turn is executing a tool (`current_turn_handle` is present, a tool placeholder is running) and the user presses ESC
- **THEN** the turn task SHALL be aborted
- **AND** the running tool placeholder SHALL be finalized (`tool_running = false`) so the spinner stops
- **AND** the agent phase SHALL become `Idle`

#### Scenario: ESC interrupts during compaction

- **WHEN** a `/compact` turn is running (`current_turn_handle` is present, phase is `Compacting`) and the user presses ESC
- **THEN** the compaction turn SHALL be aborted (best-effort)
- **AND** the agent phase SHALL become `Idle`

#### Scenario: ESC while idle does nothing

- **WHEN** no turn is running (`current_turn_handle` is absent) and the user presses ESC
- **THEN** no turn SHALL be interrupted
- **AND** the application SHALL NOT quit

### Requirement: Interrupt preserves partial streamed content

When ESC interrupts a turn that has produced partial streamed text, the system SHALL commit the non-empty, non-hint `streaming_content` as an `Assistant` chat message before clearing the streaming buffer, so the user can see what was generated before the interruption.

#### Scenario: Partial response remains visible after interrupt

- **WHEN** a turn has streamed partial text into `streaming_content` and the user presses ESC
- **THEN** the partial text SHALL be committed as an `Assistant` message in the chat
- **AND** `streaming_content` SHALL be cleared and `streaming_active` SHALL be false
- **AND** the partial text SHALL remain visible in the conversation

#### Scenario: No partial content leaves no artifact

- **WHEN** a turn has no streamed text (empty `streaming_content` or only the "preparing tools..." hint) and the user presses ESC
- **THEN** no `Assistant` message SHALL be committed from the streaming buffer
- **AND** `streaming_active` SHALL be false

### Requirement: Interrupt surfaces user feedback

The system SHALL surface a `⏹ Interrupted by user` system message in the chat when a turn is interrupted via ESC, so the user has a clear visual confirmation that the interrupt was applied.

#### Scenario: System message shown on interrupt

- **WHEN** the user presses ESC and a running turn is interrupted
- **THEN** a system message with content `⏹ Interrupted by user` SHALL be appended to the committed messages

### Requirement: Interrupt cancels running subagents

When ESC interrupts a turn, the system SHALL advance the agent generation on the daemon (via `reset_agent_generation`) so that daemon-side subagent subtrees belonging to the interrupted turn are cancelled, and the next turn adopts a fresh generation.

#### Scenario: Subagents cancelled on interrupt

- **WHEN** a turn with running subagents is interrupted via ESC
- **THEN** the daemon agent generation SHALL be advanced
- **AND** the subagent tree SHALL be cleared from the UI
- **AND** the next turn SHALL use the new generation

### Requirement: ESC no longer quits the application

The TUI REPL SHALL NOT quit when ESC is pressed. The previous ESC-to-quit fallback SHALL be removed. Quitting the application SHALL remain via the existing Ctrl+C double-press within 500ms.

#### Scenario: ESC does not quit when idle

- **WHEN** no turn is running and no contextual panel is open and the user presses ESC
- **THEN** the application SHALL NOT quit
- **AND** `should_quit` SHALL remain false

#### Scenario: Ctrl+C double-press still quits

- **WHEN** the user presses Ctrl+C twice within 500ms
- **THEN** the application SHALL quit (`should_quit = true`)

### Requirement: Contextual panels retain ESC priority over interrupt

Contextual UI panels that already consume ESC SHALL retain their existing priority and semantics and SHALL intercept ESC before the turn-interrupt logic. Specifically: the subagent focus view exits on ESC; the completion panel closes on ESC; the permission panel interprets ESC as Deny; the question panel handles ESC; the session popup dismisses on ESC; the subagent status bar unfocuses on ESC.

#### Scenario: ESC during a permission prompt denies, not interrupts

- **WHEN** a permission panel is visible (phase `AwaitingPermission`) and the user presses ESC
- **THEN** the permission SHALL be denied (existing behavior)
- **AND** the running turn SHALL NOT be aborted by ESC

#### Scenario: ESC dismisses an open popup instead of interrupting

- **WHEN** the session popup (or completion panel or focus view) is open and a turn is running and the user presses ESC
- **THEN** the open popup SHALL be dismissed
- **AND** the running turn SHALL NOT be interrupted
