# Subagent Dispatch Fallback

## Overview

When a subagent dispatch fails, the parent agent (the one that dispatched the
child) automatically attempts a **single** fallback execution. This prevents
dispatch failures from becoming task failures. The classic trigger is a model
endpoint being unavailable (e.g. `deepseek-reasoner` returning 503), but
structural pre-dispatch failures are also covered.

## Two Interception Points

The fallback uses a hybrid design because the two failure classes travel
different code paths.

### Interception 1: Pre-dispatch structural failure

- **Location:** `TaskTool::execute_with_context` (`src/tools/meta/task.rs`)
- **Trigger:** `reserve_child_in_group` returns `CoordinatorError::DepthLimitReached`,
  `ConcurrencyClosed`, or `TaskGroup`.
- **Action:** TaskTool synchronously executes `full_prompt` via
  `run_subagent_loop_with_permissions`, using the parent agent's api client and
  tool registry. The result is returned as the `task` tool output. The parent
  agent's model is unaware that a fallback occurred.
- **Model:** Reuses the parent's current model (structural failure does not
  swap the model).
- **Prompt source:** `full_prompt` is still in scope at the dispatch point (the
  child never ran, so no transcript exists).

### Interception 2: Runtime model failure

- **Location:** `SubagentSynthesis::on_candidate_final` (`src/teams/subagent_loop.rs`),
  after `collect_children_for_synthesis` returns.
- **Trigger:** A `ChildResult` with `status = Failed` and
  `error_code = subagent_model_unavailable`.
- **Action:** Re-dispatches the child with a fallback model (the first entry in
  `agent.subagent.fallback_models` different from the failed model). The
  `user_prompt` is recovered from `SubagentTranscriptStore::get_by_id(child_id)`.
- **Model:** Swaps `models.main.name` to the fallback model; reuses the original
  endpoint (`base_url` / `api_key` / `provider` are preserved). If the endpoint
  itself is down, the fallback fails and degrades to the parent model.

## Constraints

- **Single-shot:** Each child can only fall back once. The `fallback_used`
  marker on `AgentCoordinator` (keyed by child id for interception 2, or
  `pending:<description>` for interception 1) prevents recursion.
- **Root exclusion:** Root callers (`parent_id.is_none()`) never self-execute a
  fallback (Comet build-phase isolation rules forbid the main session from
  executing tasks directly). A root caller that hits a fallback-eligible failure
  gets `fallback_root_blocked` and the failure is surfaced to the root model.
- **No fallback for:** Timeout, stuck, max-rounds, panic, parent-scope
  cancellation. These keep today's behavior (parent model decides).
- **Endpoint failure:** If the fallback model's endpoint is also down, the
  fallback fails and degrades to the parent model (no recursion, single-shot).

## Configuration

```toml
[agent.subagent]
fallback_models = ["claude-sonnet-5", "gpt-4o"]
```

- Entries are model names only. The fallback reuses the original endpoint
  (`base_url` / `api_key` / `provider`); only `models.main.name` is swapped.
- Empty list (default) => model-availability failures degrade to the parent
  model (current behavior, no fallback).

## Observability

- `tracing` logs: fallback trigger (interception point, kind, model name),
  success, and failure (with reason). Look for `fallback = "interception1"` /
  `fallback = "interception2"`.
- `FailureMode::ModelUnavailable` bucket in `subagent_health.rs` classifies
  `subagent_model_unavailable` / "model unavailable" messages; severity
  Critical; the health panel surfaces counts and a recommendation to configure
  `fallback_models`.

## Error codes

| Code | Meaning |
|------|---------|
| `subagent_model_unavailable` | Child failed because its model endpoint was unavailable (eligible for interception 2). |
| `fallback_root_blocked` | A root caller hit a fallback-eligible failure; self-execution is forbidden. |
| `fallback_already_used` | Single-shot constraint: this child already used its fallback. |
| `fallback_no_registry` | The tool registry was dropped and could not be upgraded for fallback. |
| `fallback_execution_failed` | The fallback execution itself failed; original failure preserved, degrades to parent/root model. |

## Compatibility with Comet Isolation

The fallback executor is always the parent agent (a subagent itself), never the
root coordinator or main session. This complies with Comet build-phase
isolation rules that forbid the main session from executing tasks directly.
The `subagent-driven-development` dual-review flow is unchanged.

## Known limitations

- **Salvage context not injected:** Interception 2 re-dispatches with the
  original `user_prompt` recovered from the transcript. The failed child's
  accumulated `text_snapshot` is not explicitly injected as salvage context
  (the transcript already preserves the failed run's history, and
  `ChildResult.partial_result` is currently always `None` for failed children).
  The spec's "partial result offered to fallback" SHALL is downgraded to a
  best-effort: the transcript's event history is the recovery path. See the
  delta spec for the amended requirement.
- **Fallback child is a "ghost" -- cannot spawn grandchildren:** The
  synthesized fallback child context (both interceptions) is NOT registered
  with the coordinator's `scopes` (it bypasses `reserve_child`/`register_task`
  because the original reserve failed or the re-dispatch is a fresh synthetic
  agent). Consequently, if the fallback child itself calls the `task` tool to
  spawn a grandchild, `reserve_child` returns `CoordinatorError::ParentNotRunning`
  (the synthetic agent is not in `scopes`), which is not fallback-eligible and
  surfaces to the fallback child's model. Tasks whose prompt requires nested
  subagent dispatch therefore cannot be completed by the fallback path. A
  fuller fix would register the synthetic child scope before running the loop;
  deferred. For now, fallback suits leaf tasks (file inspection, single
  command, research) rather than coordinator-style nested-dispatch tasks.
- **Headless path:** `run_subagent_loop` (headless wrapper, used by run-script
  / rlm) passes `Settings::default()` with no transcript store, so interception
  2 degrades to the parent model on those paths. Rich contexts that go through
  `run_subagent_loop_with_permissions` directly get the full fallback.
- **Live-endpoint re-dispatch not unit-tested:** The re-dispatch in
  `attempt_model_fallback` needs a real model endpoint, so it is exercised via
  the interception-point code paths rather than an isolated unit test.
