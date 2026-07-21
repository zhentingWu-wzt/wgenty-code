## ADDED Requirements

### Requirement: Failure root-cause classification
The system SHALL classify every subagent failure into a structured `FailureRootCause` enum captured at failure time: `TokenBudgetExceeded`, `GuardianRejected` (with reason), `SandboxFailed`, `ApiError`, `ToolPanic`, `Timeout`, `UserCancelled`, or `Unknown`. Classification SHALL be determined from structured signals available at the capture site (e.g., guardian decision, sandbox error variant, token-budget counter), not solely from post-hoc string matching of the error message.

#### Scenario: Guardian rejection classified structurally
- **WHEN** a subagent fails because the guardian denied a tool call
- **THEN** the failure `root_cause` SHALL be `GuardianRejected` carrying the guardian's denial reason, regardless of the error message wording

#### Scenario: Token budget exceeded classified from counter
- **WHEN** a subagent fails because its cumulative token usage exceeds the configured budget
- **THEN** the failure `root_cause` SHALL be `TokenBudgetExceeded`, determined from the token counter rather than message text

#### Scenario: Unknown fallback preserves raw message
- **WHEN** a failure cannot be mapped to a known root cause
- **THEN** the `root_cause` SHALL be `Unknown` and the original `error_message` SHALL be preserved verbatim for manual inspection

### Requirement: Complete failed tool-call sequence captured
On failure, the system SHALL capture the complete ordered sequence of tool calls executed during the failing round (or the failing attempt), each entry recording tool name, a redacted summary of key parameters, and per-step elapsed milliseconds--not only the last tool call.

#### Scenario: Full sequence retained on failure
- **WHEN** a subagent fails after invoking tools A, B, C in the failing round
- **THEN** the failure diagnostics SHALL contain all three tool-call steps (A, B, C) in order, each with its tool name, redacted parameter summary, and elapsed_ms

#### Scenario: Sensitive parameters redacted
- **WHEN** a tool call in the failing sequence carries parameters with sensitive keys (api_key, token, secret, password)
- **THEN** those values SHALL be redacted in the captured parameter summary before persistence or emission

### Requirement: Failed-round full context captured
On failure, the system SHALL capture the failing round's assistant text and the final tool's raw output, each truncated to a configurable character limit (`subagent.trace.context_char_limit`, default 2000), to allow post-hoc reconstruction of the subagent's reasoning at failure.

#### Scenario: Assistant text and tool output retained truncated
- **WHEN** a subagent fails in round N
- **THEN** the diagnostics SHALL include the round-N assistant text and the final tool raw output, each truncated at char boundaries to the configured limit

#### Scenario: Truncation is char-boundary safe
- **WHEN** the configured truncation limit falls within a multi-byte UTF-8 character
- **THEN** truncation SHALL adjust to the nearest valid character boundary without panicking

### Requirement: Retry history recorded
When a subagent execution is retried, the system SHALL record a `RetryAttempt` per attempt, capturing that attempt's error, root cause, the retry strategy that triggered the retry, and the final outcome (succeeded/failed). The historical `retryable: bool` flag SHALL remain available for backward compatibility.

#### Scenario: Multiple retries each recorded
- **WHEN** a subagent is retried twice (attempts 1, 2, 3) and attempt 3 succeeds
- **THEN** the diagnostics SHALL contain three `RetryAttempt` entries with their respective errors/root causes and a final `succeeded` outcome on attempt 3

#### Scenario: No retry yields empty history
- **WHEN** a subagent fails with no retries
- **THEN** `retry_history` SHALL be empty and `retryable` SHALL reflect whether a retry was permitted
