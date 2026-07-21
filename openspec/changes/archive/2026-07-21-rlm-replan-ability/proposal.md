# Proposal: RLM Task-Level Replan Ability

## Why

The RLM pipeline (`run_rlm_pipeline`) decomposes a complex task into up to 8 sub-tasks
(via a single Planner API call), executes them level-by-level in parallel through the
`AgentCoordinator` permit model, and aggregates results. When a sub-task fails, the
Executor only records `results[idx] = "[ERROR] ..."` and proceeds to the next dependency
level; the Aggregator then merges the `[FAILED]` entries as-is. There is **no retry and no
re-decomposition** at the task-orchestration layer.

This leaves long-running autonomy with a core gap: a complex task that fails once at
decomposition or execution is abandoned, with no ability to dynamically adjust. The node
state machine already provides node-level AutoRetry, but task-level replan (re-decompose a
failed sub-task + its downstream dependents) is a blank. Together they would form a
complete two-tier retry system (task-level + node-level).

Notably, the configuration scaffolding for this already exists but is **unwired**:
`RlmSettings { retry_enabled: true, max_replan_cycles: 2, jaccard_threshold: 0.8 }`
(`src/config/guardian.rs`) plus a `SubagentRlmOverride` path (`src/config/agent.rs`), with
config tests asserting the defaults. A grep for `replan|retry|max_replan` across
`src/tools/meta/rlm/` returns no matches: the pipeline never reads these fields. The
`jaccard_threshold` value also appears hardcoded as `0.8` in `formats.rs:232`
(`deduplicate_claims`) within a structured `Aggregator` that the live pipeline never calls
(the pipeline's aggregator phase does a plain LLM merge). So all three fields are
effectively unused scaffolding awaiting this feature.

## What Changes

1. **Extract `Planner` and `Executor` from the monolithic `run_rlm_pipeline`**
   (`src/tools/meta/rlm/pipeline.rs`, ~630 lines) into `planner.rs` and `executor.rs`,
   matching the module layout referenced in the improvement spec. The pipeline becomes a
   thin orchestrator. Behavior-preserving refactor; no functional change.

2. **Add task-level replan to the Executor**: when a sub-task fails (and replan is enabled
   and the global quota is not exhausted), re-invoke the Planner in **incremental mode** —
   input is the original plan + the failed sub-task id + failure reason (+ completed
   sub-tasks' partial results as context), output is a replacement sub-task set for the
   failed sub-task and its downstream dependents. Replacement sub-tasks are executed
   through the same coordinator permit + structural-fallback path. The pipeline runs at
   most `max_replan_cycles` replans total across the whole run (global shared quota).

3. **Wire the existing `RlmSettings`** into the pipeline: `retry_enabled` gates replan;
   `max_replan_cycles` bounds total replans per run; `jaccard_threshold` deduplicates
   replan output (reject a replacement sub-task whose prompt is semantically identical,
   Jaccard >= threshold, to the failed one — do not regenerate the same failure).
   Subagent-level `SubagentRlmOverride` applies when the RLM caller is a subagent.

4. **P2-2 verification (already-done confirmation)**: verify and document that RLM
   sub-tasks already flow through `AgentCoordinator::reserve_child` (acquires the global
   concurrency semaphore, `coordinator.rs:448`) and `finish_child` (releases the permit,
   `coordinator.rs:671`), called at `pipeline.rs:329` / `pipeline.rs:483`. The only permit
   "bypass" is the structural-fallback ghost path, which is the intended degradation when
   permits are saturated (mirrors the `task` tool). Update `WGENTY.md` to mark P2-2
   (unified parallel orchestration) as satisfied by the existing design.

## Capabilities Affected

- `rlm-replan` (NEW) — task-level replan: trigger on sub-task failure, local scope
  (failed sub-task + downstream dependents), `max_replan_cycles` global quota,
  `jaccard_threshold` output dedup, `retry_enabled` gate, incremental Planner mode.
- `rlm-budget-control` (MODIFIED) — replan consumes budget (replanner call +
  re-execution of replacement sub-tasks); define how replan budget is carved from the
  executor pool and how unused replan budget rolls forward.

## Out of Scope

- P0-1 memory dedup — already implemented (`add_memory` dedup-on-write + `prune`
  consolidation + orphan cleanup); the improvement spec is stale on this point.
- Other improvement points (P1-1 comet verify subagentization, P1-2 micro-compaction,
  P1-3 cross-session resume, P2-1 node hierarchy, P2-3 verify-contract checks,
  P3-1/P3-2/P3-3) — each is an independent change per the roadmap.
- Node-level AutoRetry — orthogonal; RLM replan is task-level and does not modify node
  state machine behavior.
- Single-shot Planner / Aggregator core logic — unchanged; only a new incremental replan
  path is added.
- Structured `Aggregator` dead code in `formats.rs` — observed but not cleaned up here
  (unrelated; avoid scope creep).
- New configuration fields — reuse existing `RlmSettings` / `SubagentRlmOverride`; no
  new keys (backward compatible).

## Open Questions (to resolve in design)

- Replan trigger failure-type boundary: which failures trigger replan (subagent loop
  error / structural-fallback ghost / timeout / join error)?
- Incremental Planner prompt design: exact input contract (does it include completed
  sub-tasks' partial results? how are downstream dependents identified for replacement?).
- Replacement sub-task depth attribution: inherit the failed sub-task's dependency level
  or append a new level?
- Replan budget interaction with `BudgetAllocation` (how much of the executor pool is
  reserved for replan, rollover semantics).
