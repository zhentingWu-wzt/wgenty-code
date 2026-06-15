## ADDED Requirements

### Requirement: Per-turn input token tracking
The system SHALL estimate and accumulate user input tokens for each turn using `chars/4` formula, applied to the user's message before it is appended to conversation history.

#### Scenario: Single user input
- **WHEN** user submits a message of 100 characters
- **THEN** the turn's input token counter SHALL display `↑ 25` (100/4)

#### Scenario: Multi-byte characters
- **WHEN** user submits a message containing Chinese/UTF-8 characters of 200 bytes
- **THEN** the turn's input token counter SHALL divide by character count (`.len()`), not byte length

### Requirement: Per-turn output token tracking
The system SHALL accumulate model output tokens for each turn by summing `completion_tokens` from each LLM round's `Usage` within the turn.

#### Scenario: Single LLM round
- **WHEN** an LLM round returns `usage.completion_tokens = 800`
- **THEN** the turn's output token counter SHALL be incremented by 800

#### Scenario: Multiple LLM rounds with tool calls
- **WHEN** a turn involves 3 LLM rounds with completion_tokens [800, 500, 300]
- **THEN** the turn's output token counter SHALL accumulate to 1600

### Requirement: Turn reset on new input
The system SHALL reset both input and output token counters to zero at the beginning of each new user turn.

#### Scenario: Second turn starts
- **WHEN** a new user turn begins after a completed turn showing `↑ 25 · ↓ 1.6k`
- **THEN** both input and output counters SHALL reset to 0 before processing the new input

### Requirement: Status bar display format
The status bar SHALL display per-turn token counts in `↑ N · ↓ Mk` format. When a counter is 0, its section SHALL be omitted.

#### Scenario: Both input and output available
- **WHEN** turn has input=25 tokens and output=1600 tokens
- **THEN** status bar SHALL display `↑ 25 · ↓ 1.6k` in the meta section

#### Scenario: Only output available (no input estimated)
- **WHEN** turn has input=0 and output=800 tokens
- **THEN** status bar SHALL display `↓ 800 tokens` without the input section

#### Scenario: Idle state preserves last turn value
- **WHEN** a turn completes and agent enters idle state
- **THEN** the status bar SHALL continue displaying the last turn's token values

### Requirement: Budget counter isolation
The system SHALL maintain the existing `used`/`budget` fields in `TokenCounter` unchanged, independent of the new `turn_input`/`turn_output` counters.

#### Scenario: Budget enforcement unaffected
- **WHEN** turn_input + turn_output reach a different value than the budget counter's `used` field
- **THEN** budget enforcement SHALL be based on the `used` field only, not on turn counters
