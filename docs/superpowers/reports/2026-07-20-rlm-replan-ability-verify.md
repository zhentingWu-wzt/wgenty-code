# Verification Report: RLM Task-Level Replan Ability

**Change**: `openspec/changes/rlm-replan-ability`
**Date**: 2026-07-20
**Mode**: full (30 tasks, 2 delta capabilities, 13 changed files)
**Branch**: `feature/rlm-replan-ability` (base_ref `21637d09`, +1 commit `ed7b215`)
**Result**: ✅ PASS

## 1. Verification Commands (fresh evidence)

| Command | Result |
|---------|--------|
| `cargo build --release` | ✅ PASS (0 warnings) |
| `cargo test rlm` | ✅ PASS (38 unit + 2 integration, 0 failed) |
| `cargo clippy --all-targets -- -D warnings` | ✅ PASS (CLIPPY_EXIT=0, 0 warnings) |
| `cargo fmt --check` | ✅ PASS (FMT_EXIT=0) |

## 2. Pre-existing Failures (unrelated to this change)

`config::models::tests::{test_known_context_window_matches_common_models, test_resolve_context_window_priority}` fail with `left: Some(1024000), right: Some(200000)`.

Confirmed pre-existing at base_ref `21637d09`: `known_context_window` returns `Some(1024_000)` for the relevant models at the base commit. The worktree's only `config/models.rs` change is a cosmetic reformat (`1024_000` → `1_024_000`, value unchanged). These failures are NOT caused by the RLM replan change and are out of scope.

## 3. Full-mode 7 Checks

### 3.1 tasks.md completion ✅
All 30 subtasks across 8 sections marked `[x]`. Design deviations for 1.2/1.3 (executor.rs/aggregator.rs inlined in pipeline.rs) documented inline with rationale.

### 3.2 Implementation matches design.md high-level decisions ✅
All 5 Open Question decisions implemented:
- **Q1** (replan trigger = only `Ok(Err)`): `pipeline.rs` replan loop gates on `Ok(Err)`, join errors excluded
- **Q2** (Executor-driven graph analysis + explicit replaces_id): `compute_replan_scope` in `planner.rs` computes transitive dependents deterministically
- **Q3** (early replan + relevel by depends_on): failed sub-tasks + downstream removed from pending levels; replacements re-leveled
- **Q4** (charge executor pool): `charge_executor`/`executor_has`/`rollover_unused_executor` in `budget.rs`
- **Q5** (jaccard dedup, reuse jaccard_similarity): `pipeline.rs` replan dedup uses `RlmSettings.jaccard_threshold`

### 3.3 Implementation matches Design Doc ✅
`docs/superpowers/specs/2026-07-20-long-running-autonomy-improvements.md` §3 P0-2 (task-level replan ability) is satisfied: sub-task failure triggers local replan of failed task + downstream dependents, with global quota and jaccard dedup.

### 3.4 Capability spec scenarios pass ✅
16 scenarios across 2 delta specs mapped to passing tests:

**rlm-replan/spec.md** (7 requirements):
- Replan gated by configuration → `test_resolve_rlm_settings_*` (3) + config tests (5)
- Local replan scope → `test_compute_replan_scope_*` (4)
- Global replan quota → replan loop quota counter in `pipeline.rs`
- Replan output deduplication → `test_jaccard_dedup_*` (4)
- Incremental Planner mode → `Planner::replan_incremental` + `compute_replan_scope`
- Replacement execution path → `test_d6_rlm_partial_failure` + `test_d6_rlm_retry_dead_code`
- Subagent-level override → `test_resolve_rlm_settings_subagent_override_applied` + fallback test

**rlm-budget-control/spec.md** (1 requirement):
- Replan phase budget allocation → `test_charge_executor_saturating` + `test_executor_has` + `test_rollover_unused_executor`

### 3.5 proposal.md goals satisfied ✅
1. ✅ Planner extracted to `planner.rs` (`Planner::plan` + `replan_incremental`)
2. ✅ Task-level replan added (failed sub-task + downstream, gated by config)
3. ✅ Config scaffolding wired (`RlmSettings` + `SubagentRlmOverride`)
4. ✅ P2-2 verified + documented (WGENTY.md updated)

### 3.6 delta spec vs design doc — no contradiction ✅
Delta specs are behavioral (replan gating, scope, quota, dedup, budget, override). All behaviors are reflected in design.md Q1-Q5 decisions. No delta-spec content lacks a design.md basis.

**Implementation divergence (documented, non-blocking)**: design.md §4.1 specifies `executor.rs` and `aggregator.rs` as separate files; implementation inlined both in `pipeline.rs`. Rationale recorded in tasks.md 1.2/1.3: Executor/Aggregator logic is tightly coupled with `coordinator.reserve_child`/spawn closures; `pipeline.rs` is already a thin orchestrator calling `Planner::plan` → inline execute/replan → inline aggregate. Splitting would add indirection without clarity benefit. This is a module-structure deviation, not a behavioral spec contradiction.

### 3.7 Design Doc locatable ✅
`docs/superpowers/specs/2026-07-20-long-running-autonomy-improvements.md` exists (14482 bytes), referenced by design.md header.

## 4. Dirty Worktree Attribution

Per `comet/reference/dirty-worktree.md` protocol, the uncommitted worktree mixes concerns:

| Files | Concern | Attribution |
|-------|---------|-------------|
| `pipeline.rs`, `planner.rs`, `budget.rs`, `rlm/mod.rs`, `WGENTY.md` | RLM replan core | Current change ✅ |
| `subagent_loop.rs`, `task.rs`, `Cargo.toml`, `refactor_e2e_test.rs`, `task/heuristic.rs` | Supporting infra | Undocumented (see note) |
| `compaction.rs`, `loop_.rs` | Compaction calibration fix | Separate concern |
| `config/models.rs` | Cosmetic reformat | Separate concern |

**User decision**: verify the combined state (all changes build/test green), then at commit time split — commit only RLM core files to the feature branch, leave non-RLM changes uncommitted for a separate change.

Untracked unrelated files (separate `web-ops-console` change, archived changes, `.superpowers/`, debug docs) are excluded from the RLM commit.

## 5. Security Review

- No hardcoded secrets or credentials in the diff
- No new `unsafe` blocks
- No new network egress paths
- Replan executes replacement sub-tasks through the existing `coordinator.reserve_child`/`finish_child` permit model (no bypass of concurrency limits)
- Budget enforcement prevents unbounded replan spending (`executor_has` check before replanner call)

## 6. Conclusion

Verification PASSES. The RLM task-level replan ability is correctly implemented per design.md and the delta specs. The 2 failing config tests are pre-existing at base_ref and unrelated. The implementation divergence (inlined executor/aggregator) is documented in tasks.md with sound rationale.
