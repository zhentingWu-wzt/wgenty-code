# subagent-result-delivery Specification

## Purpose
TBD - created by archiving change split-api-and-subagent-result-relief. Update Purpose after archive.
## Requirements
### Requirement: Large subagent results remain accessible without loss
When a subagent produces a result exceeding the persistence threshold, the system SHALL preserve the full content such that the parent agent can access it without lossy truncation. The parent agent SHALL NOT be presented with a fixed-length prefix summary as the only representation of a large result.

#### Scenario: Parent agent can recover full content of a large result
- **WHEN** a subagent produces a result larger than `MAX_INLINE_RESULT_LEN` (4000 chars)
- **THEN** the full content SHALL be persisted to disk
- **AND** the parent agent SHALL be able to access the full content (either inline or via a recovery path)

#### Scenario: Large result not replaced by short prefix-only summary
- **WHEN** a subagent result exceeds the persistence threshold
- **THEN** the parent agent SHALL NOT receive only a 200-character prefix summary as the sole representation
- **AND** the full content SHALL remain recoverable

### Requirement: Large result delivery controls parent context token cost
The system SHALL deliver large subagent results to the parent agent through a mechanism that bounds the parent agent's context token consumption, rather than unconditionally inlining the full content. The specific mechanism (on-demand loading, compaction-time degradation, or hybrid) is determined by design.

#### Scenario: Full content not unconditionally inlined
- **WHEN** a subagent result exceeds the persistence threshold
- **THEN** the system SHALL NOT unconditionally inline the entire content into the parent agent's context as the sole delivery strategy
- **AND** a token-bounding mechanism SHALL be in place

### Requirement: Disk persistence for recovery
When a subagent result exceeds the persistence threshold, the system SHALL persist a copy to the JSONL mailbox so the full content can be recovered later (e.g., after context compaction).

#### Scenario: Large result persisted to disk
- **WHEN** a subagent result exceeds `MAX_INLINE_RESULT_LEN`
- **THEN** a copy SHALL be written to the JSONL mailbox file
- **AND** the recovery path SHALL be communicated to the parent agent

#### Scenario: Persistence failure does not lose content
- **WHEN** disk persistence fails for a large result
- **THEN** the full content SHALL still be returned to the parent agent inline (no truncation)
- **AND** the failure SHALL be logged

### Requirement: Failed subagent delivers structured error code and partial results
When a subagent fails (model-availability failure, timeout, budget exhaustion, stuck detection, parse error, or max-rounds exceeded), the system SHALL return a structured `SubagentError` to the parent agent carrying (1) a categorized error type that maps to a stable error-code string and (2) the subagent's last accumulated text snapshot as a partial result. The parent agent SHALL receive both the structured error code (via `ToolError::code`) and the partial work (via `ToolError::message`, which appends the partial result through `full_message()`), so it can salvage partial work and make informed retry/continue/abort decisions rather than receiving a bare error string with no recoverable output. The failed transcript SHALL record the same `full_message()` as its result snapshot.

When the failure is fallback-eligible (model-availability failure, or a pre-dispatch structural failure such as depth-limit, concurrency-closed, or task-group-add failure), the system SHALL additionally invoke the parent self-execution fallback defined in the `subagent-dispatch-fallback` capability. The fallback takes a hybrid two-path form: a model-availability runtime failure (which passes through `ChildTerminal` and `collect_children_for_synthesis`) SHALL trigger a re-dispatched fallback child using a configured fallback model; a pre-dispatch structural failure (which does NOT pass through `ChildTerminal` or `collect_children_for_synthesis`, since no child ever ran) SHALL be handled directly at the `TaskTool::execute_with_context` dispatch point by synchronous execution of the task's `full_prompt`. Failures that are not fallback-eligible (timeout, stuck/max-rounds, panic, parent-scope cancellation) SHALL continue to be delivered to the parent agent's model for a decision without triggering the fallback.

#### Scenario: Subagent timeout delivers structured error code and partial results
- **WHEN** a subagent exceeds `agent.subagent.timeout_secs`
- **THEN** the parent agent SHALL receive a `ToolError` whose `code` is `subagent_timeout`
- **AND** the `ToolError::message` SHALL include any text the subagent accumulated before timing out, appended via `full_message()`'s "Partial results" section
- **AND** the failed transcript SHALL record the same `full_message()` as its result snapshot
- **AND** the system SHALL NOT trigger the parent self-execution fallback (timeout is not fallback-eligible)

#### Scenario: Budget exhaustion delivers budget_exceeded code and partial results
- **WHEN** a subagent exhausts its token budget
- **THEN** the parent agent SHALL receive a `ToolError` whose `code` is `budget_exceeded`
- **AND** the `ToolError::message` SHALL include the subagent's partial work accumulated before budget exhaustion
- **AND** the system SHALL NOT trigger the parent self-execution fallback

#### Scenario: Model-availability failure triggers parent self-execution fallback
- **WHEN** a subagent fails because its model endpoint is unavailable (non-2xx HTTP response or model-endpoint connection error)
- **THEN** the parent agent SHALL receive a `ToolError` whose `code` is `subagent_model_unavailable`
- **AND** the `ToolError::message` SHALL include any text the subagent accumulated before failing
- **AND** the system SHALL invoke the parent self-execution fallback, executing the task in the dispatching parent's context using a configured fallback model

#### Scenario: Empty partial result does not append an empty segment
- **WHEN** a subagent fails and has no accumulated text snapshot, or the snapshot is empty/whitespace-only
- **THEN** `full_message()` SHALL return only the error message
- **AND** SHALL NOT append a "Partial results (before failure)" section
- **AND** if the failure is fallback-eligible, the fallback SHALL still proceed without salvage context

