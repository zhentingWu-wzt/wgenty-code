# subagent-dispatch-fallback Specification

## Purpose
TBD - created by archiving change subagent-dispatch-fallback. Update Purpose after archive.
## Requirements
### Requirement: Fallback triggers on model-unavailable and pre-dispatch structural failures

When a child subagent dispatch fails due to either (a) a model-availability failure (model unavailable, API HTTP error, connection error attributable to the model endpoint) or (b) a pre-dispatch structural failure (depth limit reached, concurrency closed, task-group add failure), the system SHALL invoke a parent self-execution fallback in which the parent agent that dispatched the child performs the task in its own context. The fallback SHALL NOT be triggered for timeout, stuck/max-rounds, panic, or parent-scope cancellation failures; those continue to follow existing behavior (parent model decides, or cancellation propagates).

The fallback uses a hybrid two-path form determined by where the failure is detected:
- A **pre-dispatch structural failure** is detected at the dispatch point inside `TaskTool::execute_with_context` (before any child runs). It SHALL be handled by the TaskTool itself synchronously executing the task's `full_prompt` using the parent agent's current api client and tool registry, returning the result as the task tool's output. The parent agent's model SHALL NOT be aware that a fallback occurred. The task's `full_prompt` is available directly at the dispatch point.
- A **model-availability runtime failure** is detected after a child ran and failed (`ChildResult` with `subagent_model_unavailable`). It SHALL be handled by re-dispatching a fallback child subagent that uses a configured fallback model (overriding only the model name on the original endpoint), with the task's `user_prompt` recovered from the transcript store.

#### Scenario: Model-unavailable failure triggers fallback with model switch
- **WHEN** a dispatched child subagent fails because its model endpoint is unavailable (e.g. `deepseek-reasoner` returns a non-2xx or connection error)
- **AND** the failure is classified as a model-availability failure
- **THEN** the system SHALL trigger the parent self-execution fallback (interception point 2)
- **AND** the parent agent SHALL re-dispatch a fallback child subagent using a configured fallback model (overriding only the model name on the original endpoint, not the failed child model)
- **AND** the failed child's `user_prompt` SHALL be recovered from the transcript store to drive the fallback child
- **AND** the failed child's partial result SHALL be made available to the fallback as salvage context

#### Scenario: Pre-dispatch depth-limit failure triggers fallback without model switch
- **WHEN** dispatch is rejected because the caller depth equals `max_depth` (`CoordinatorError::DepthLimitReached`)
- **THEN** the system SHALL trigger the parent self-execution fallback (interception point 1, inside `TaskTool::execute_with_context`)
- **AND** the TaskTool SHALL synchronously execute the task's `full_prompt` using the parent agent's current api client and tool registry (no model switch, since the failure is not model-related)
- **AND** the system SHALL NOT spawn any deeper subagent for this task
- **AND** the parent agent's model SHALL receive the result as the task tool's output without awareness of the fallback

#### Scenario: Pre-dispatch concurrency-closed failure triggers fallback
- **WHEN** dispatch is rejected because concurrency is closed (`CoordinatorError::ConcurrencyClosed`)
- **THEN** the system SHALL trigger the parent self-execution fallback (interception point 1) using the parent's current model via TaskTool-internal synchronous execution

#### Scenario: Task-group add failure triggers fallback
- **WHEN** dispatch fails during task-group reservation (`CoordinatorError` from `reserve_child_in_group`)
- **THEN** the child permit SHALL be released via `finish_child(Cancelled)`
- **AND** the system SHALL trigger the parent self-execution fallback (interception point 1) using the parent's current model via TaskTool-internal synchronous execution

#### Scenario: Pre-dispatch failure fallback does not go through synthesis
- **WHEN** a pre-dispatch structural failure triggers the fallback
- **THEN** the fallback SHALL be handled at the `TaskTool::execute_with_context` dispatch point
- **AND** SHALL NOT pass through `ChildTerminal` or `collect_children_for_synthesis` (those paths only apply to runtime failures)

#### Scenario: Timeout does not trigger fallback
- **WHEN** a child subagent fails with `subagent_timeout`
- **THEN** the system SHALL NOT trigger the parent self-execution fallback
- **AND** the failure SHALL be delivered to the parent agent's model as today (structured error code + partial result)

#### Scenario: Parent-scope cancellation does not trigger fallback
- **WHEN** a child subagent is cancelled by its parent scope (`ChildTerminal::Cancelled`)
- **THEN** the system SHALL NOT trigger any self-execution fallback

#### Scenario: Fallback model also fails does not recurse
- **WHEN** the fallback child subagent (interception point 2) is dispatched with the fallback model and that model's endpoint is also unavailable
- **THEN** the system SHALL NOT trigger a second fallback for the same task
- **AND** the final failure SHALL be delivered to the root coordinator's model as a terminal failure

#### Scenario: Missing transcript prompt degrades gracefully
- **WHEN** a model-availability failure triggers the fallback (interception point 2)
- **AND** the failed child's `user_prompt` cannot be recovered from the transcript store
- **THEN** the fallback SHALL NOT re-dispatch a fallback child
- **AND** the failure SHALL be delivered to the parent agent's model for a decision (equivalent to today's behavior)
- **AND** the missing-prompt condition SHALL be logged

### Requirement: Fallback is single-shot and non-recursive

Each task SHALL be allowed at most one parent self-execution fallback. If the fallback execution itself fails, the system SHALL NOT recursively trigger another fallback for the same task. The final failure SHALL be delivered to the root coordinator's model for a decision (retry with different configuration, skip, or abort), preserving a terminal signal rather than looping.

#### Scenario: Fallback success completes the task
- **WHEN** the parent self-execution fallback runs and produces a result
- **THEN** the task SHALL be marked completed with the fallback result
- **AND** no further fallback SHALL be attempted

#### Scenario: Fallback failure does not recurse
- **WHEN** the parent self-execution fallback runs and fails
- **THEN** the system SHALL NOT trigger a second fallback for the same task
- **AND** the final failure SHALL be delivered to the root coordinator's model as a terminal failure

#### Scenario: Fallback counter is per-task
- **WHEN** multiple tasks are dispatched and more than one fails with a fallback-eligible failure
- **THEN** each task SHALL independently be granted exactly one fallback attempt
- **AND** a fallback consumed by one task SHALL NOT count against another task

### Requirement: Fallback executor is the dispatching parent agent, not the root coordinator

The self-execution fallback SHALL be performed by the subagent that dispatched the failing child (the parent agent in the dispatch chain), never by the root coordinator / main session. This preserves the Comet build-phase isolation rule that the main session SHALL NOT execute tasks directly.

#### Scenario: Non-root parent executes fallback
- **WHEN** a non-root subagent (itself a child) dispatches a child that fails with a fallback-eligible failure
- **THEN** the fallback SHALL be executed within that non-root parent agent's context
- **AND** the root coordinator / main session SHALL NOT execute the task

#### Scenario: Root coordinator dispatching a child does not self-execute on fallback
- **WHEN** the root coordinator directly dispatches a child that fails with a fallback-eligible failure
- **THEN** the fallback self-execution SHALL NOT run in the root coordinator / main session
- **AND** the failure SHALL be surfaced to the root coordinator's model for a decision (the model may itself decide to retry or accept the failure)

### Requirement: Model-availability classification distinguishes model failures from other runtime failures

The system SHALL classify runtime subagent failures so that model-availability failures (model unavailable, API HTTP error, model-endpoint connection error) are distinguishable from other runtime failures (timeout, stuck, parse, panic). Failures currently collapsed into `ErrorType::Unknown` that originate from the model endpoint SHALL be reclassified into a model-availability category that the fallback decision logic can match.

#### Scenario: API HTTP error classified as model-availability failure
- **WHEN** a child subagent's model endpoint returns a non-2xx HTTP response (`RuntimeError::Stream("API error (...)")`)
- **THEN** the failure SHALL be classified as a model-availability failure (not generic `Unknown`)
- **AND** the classification SHALL be available to the fallback decision logic

#### Scenario: Model-endpoint connection error classified as model-availability failure
- **WHEN** a child subagent's model endpoint cannot be reached (connection error from `llm_api.rs`)
- **THEN** the failure SHALL be classified as a model-availability failure

#### Scenario: Non-model runtime failure not reclassified
- **WHEN** a child subagent fails due to timeout, stuck detection, or max-rounds
- **THEN** the failure SHALL retain its existing category (`subagent_timeout`, `subagent_stuck`)
- **AND** SHALL NOT be classified as a model-availability failure

### Requirement: Fallback model selection uses configured fallback chain only for model failures

When the fallback is triggered by a model-availability failure, the fallback execution SHALL use a configured fallback model (selected from a fallback model source defined at design time). When the fallback is triggered by a pre-dispatch structural failure, the fallback execution SHALL use the parent agent's current model and SHALL NOT perform model switching.

#### Scenario: Model failure uses fallback model
- **WHEN** the fallback is triggered by a model-availability failure
- **AND** a fallback model is configured
- **THEN** the fallback execution SHALL use the configured fallback model

#### Scenario: Model failure with no fallback model configured degrades gracefully
- **WHEN** the fallback is triggered by a model-availability failure
- **AND** no fallback model is configured
- **THEN** the fallback SHALL NOT self-execute
- **AND** the failure SHALL be delivered to the parent agent's model for a decision (equivalent to today's behavior)

#### Scenario: Structural failure does not switch model
- **WHEN** the fallback is triggered by a pre-dispatch structural failure (depth, concurrency, task-group)
- **THEN** the fallback execution SHALL use the parent agent's current model
- **AND** SHALL NOT consult or switch to a fallback model

### Requirement: Failed child partial result is offered to fallback execution

When the fallback is triggered after a child ran and accumulated output before failing, the system SHALL offer the failed child's accumulated work as salvageable context to the fallback execution. The recovery path is the transcript store: the failed child's full event history (including accumulated text) is persisted by `subagent-transcript-storage` and remains accessible. The fallback re-dispatches with the child's original `user_prompt` recovered from the transcript; explicit injection of a separate `text_snapshot` salvaged segment is best-effort and may be omitted when the transcript history already serves as the recovery path. When `ChildResult.partial_result` is unavailable (e.g. `None` for failed children), the fallback SHALL still proceed using the recovered `user_prompt` without salvage context.

#### Scenario: Transcript history serves as recovery path
- **WHEN** a child subagent fails at runtime after accumulating output
- **AND** the failure is fallback-eligible
- **THEN** the fallback execution SHALL proceed using the child's original `user_prompt` recovered from the transcript store
- **AND** the failed child's accumulated work SHALL remain accessible via the transcript's event history

#### Scenario: Pre-dispatch failure has no partial result to offer
- **WHEN** the fallback is triggered by a pre-dispatch failure (no child ever ran)
- **THEN** no partial result SHALL be offered
- **AND** the fallback SHALL proceed without salvage context

