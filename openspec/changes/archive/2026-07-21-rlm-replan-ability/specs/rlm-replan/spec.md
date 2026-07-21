## ADDED Requirements

### Requirement: Replan gated by configuration
The RLM Executor SHALL attempt a task-level replan when a sub-task fails, but only when `RlmSettings.retry_enabled` is true. When replan is disabled, a failed sub-task SHALL be recorded as `[ERROR]` and the pipeline SHALL proceed exactly as before this change (backward compatible).

#### Scenario: Replan disabled preserves legacy behavior
- **WHEN** a sub-task fails during executor phase
- **AND** `settings.rlm.retry_enabled` is false
- **THEN** the Executor SHALL record `results[idx] = "[ERROR] ..."` and proceed to the next dependency level without invoking the replanner
- **AND** the final `RlmResult.failed` count SHALL include the failed sub-task

#### Scenario: Replan enabled attempts recovery
- **WHEN** a sub-task fails during executor phase
- **AND** `settings.rlm.retry_enabled` is true
- **AND** the global replan quota is not exhausted
- **THEN** the Executor SHALL invoke the incremental Planner to produce a replacement sub-task set for the failed sub-task and its downstream dependents

#### Scenario: Join errors do not trigger replan
- **WHEN** a spawned sub-task fails with a join error (panic or cancellation, i.e. the `tokio::spawn` handle returns `Err`)
- **THEN** the Executor SHALL treat it as an infrastructure failure and SHALL NOT invoke the replanner
- **AND** the global replan quota SHALL NOT be consumed

### Requirement: Local replan scope
A replan SHALL re-decompose only the failed sub-task and its downstream dependents (sub-tasks that transitively `depends_on` the failed one). Sub-tasks that already completed successfully SHALL be preserved and their partial results SHALL be passed to the incremental Planner as context. The replanner SHALL NOT re-decompose the entire original plan from scratch.

#### Scenario: Failed sub-task with downstream dependents
- **WHEN** sub-task 2 fails and sub-tasks 4 and 5 transitively depend on sub-task 2
- **THEN** the replanner input SHALL include the original plan, sub-task 2's id and failure reason, and the partial results of completed sub-tasks
- **AND** the replanner output SHALL be a replacement set covering sub-task 2 and its downstream dependents 4 and 5
- **AND** the replacement set SHALL NOT include already-completed sub-tasks

#### Scenario: Failed sub-task with no downstream dependents
- **WHEN** a leaf sub-task (no dependents) fails
- **THEN** the replanner SHALL produce a replacement for only that single failed sub-task

#### Scenario: Executor determines downstream dependents via graph analysis
- **WHEN** a sub-task fails and the Executor identifies its downstream dependents
- **THEN** the Executor SHALL compute the transitive closure of `depends_on` (deterministic graph analysis) to determine which sub-task ids to replace
- **AND** the Planner SHALL NOT be responsible for identifying dependents; it SHALL only decompose the replacement set for the ids the Executor supplies

### Requirement: Global replan quota
The total number of replans across a single `run_rlm_pipeline` invocation SHALL NOT exceed `RlmSettings.max_replan_cycles` (default 2). The quota is a global shared counter across all failed sub-tasks in the run, not a per-sub-task limit. When the quota is exhausted, subsequent failures SHALL be recorded as `[ERROR]` without replan.

#### Scenario: Quota shared across failures
- **WHEN** two different sub-tasks fail in the same pipeline run
- **AND** `max_replan_cycles` is 2
- **THEN** the first failure SHALL consume one replan and the second failure SHALL consume the second replan
- **AND** a third failure in the same run SHALL be recorded as `[ERROR]` without replan

#### Scenario: Quota exhausted falls back to legacy marking
- **WHEN** a sub-task fails and the replan quota is already exhausted
- **THEN** the Executor SHALL record `results[idx] = "[ERROR] ..."` and proceed without invoking the replanner

### Requirement: Replan output deduplication
The Executor SHALL reject a replacement sub-task whose prompt is semantically identical to the failed sub-task it replaces, using Jaccard similarity on the prompt text with threshold `RlmSettings.jaccard_threshold` (default 0.8). A rejected replacement SHALL NOT be executed; the Executor SHALL either request another replacement from the Planner or, if none is available, record the sub-task as `[ERROR]`.

#### Scenario: Replacement identical to failed sub-task is rejected
- **WHEN** the replanner produces a replacement sub-task whose prompt has Jaccard similarity >= `jaccard_threshold` to the failed sub-task's prompt
- **THEN** the Executor SHALL reject that replacement and SHALL NOT execute it
- **AND** the Executor SHALL either request a distinct replacement or record the sub-task as `[ERROR]`

#### Scenario: Distinct replacement is accepted
- **WHEN** the replanner produces a replacement sub-task whose prompt has Jaccard similarity < `jaccard_threshold` to the failed sub-task's prompt
- **THEN** the Executor SHALL execute the replacement through the same coordinator permit and structural-fallback path as a normal sub-task

### Requirement: Incremental Planner mode
The Planner SHALL expose an incremental replan mode distinct from the initial decomposition mode. The incremental mode input SHALL be: the original plan, the failed sub-task id(s), the failure reason(s), and the partial results of completed sub-tasks. The output SHALL be a replacement sub-task set with `depends_on` references valid against the preserved (non-replaced) sub-task ids, so the replacement set integrates into the existing dependency graph without invalidating completed work.

#### Scenario: Incremental replan preserves completed work
- **WHEN** the incremental Planner is invoked after sub-task 2 fails
- **AND** sub-tasks 0 and 1 have completed successfully
- **THEN** the replacement sub-tasks SHALL reference preserved sub-task ids (e.g. depend on 0 or 1) where dependencies exist
- **AND** the replacement SHALL NOT redefine or invalidate sub-tasks 0 and 1

#### Scenario: Replacement integrates into dependency graph
- **WHEN** the replacement set is produced for failed sub-task 2 and its dependent sub-task 4
- **THEN** the replacement sub-tasks SHALL be executable against the existing `depth`/dependency structure without requiring the entire plan to be re-leveled from scratch

### Requirement: Replacement sub-task execution path
Replacement sub-tasks produced by a replan SHALL execute through the same `AgentCoordinator::reserve_child` permit model and structural-fallback path as normal sub-tasks, so global concurrency limits and the existing fallback semantics apply uniformly. A replacement SHALL NOT bypass the coordinator permit except via the same structural-fallback ghost path used for normal sub-tasks.

#### Scenario: Replacement respects global concurrency
- **WHEN** a replacement sub-task is executed
- **THEN** the Executor SHALL call `coordinator.reserve_child` to acquire a permit before spawning the sub-agent loop
- **AND** SHALL call `coordinator.finish_child` to release the permit after the sub-agent reaches a terminal state

#### Scenario: Replacement uses structural fallback on permit saturation
- **WHEN** a replacement sub-task cannot acquire a permit (concurrency saturated)
- **THEN** the Executor SHALL use the same structural-fallback ghost path as normal sub-tasks to self-execute inline

### Requirement: Subagent-level override
When the RLM pipeline is invoked by a subagent, the `SubagentRlmOverride` settings SHALL override the top-level `RlmSettings.retry_enabled` and `max_replan_cycles` for that invocation. Absent an override, the top-level `RlmSettings` SHALL apply.

#### Scenario: Subagent override disables replan
- **WHEN** a subagent invokes the RLM pipeline
- **AND** `SubagentRlmOverride.retry_enabled` is set to false
- **THEN** the pipeline SHALL NOT attempt any replan, regardless of the top-level `retry_enabled`

#### Scenario: No override falls back to top-level settings
- **WHEN** a subagent invokes the RLM pipeline
- **AND** no `SubagentRlmOverride` is configured
- **THEN** the pipeline SHALL use the top-level `RlmSettings.retry_enabled` and `max_replan_cycles`
