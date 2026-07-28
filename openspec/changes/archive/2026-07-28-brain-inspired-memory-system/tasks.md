# Implementation Tasks

> **Scope lock:** 仅 M1–M5（effective importance、Tier-1 supersede、幂等 staleness、可选 exploration、配置/迁移/文档）。  
> **不做：** engagement 归因、Tier-2 LLM、情节层/replay、符号多线索、pain_score、热路径 restate。  
> 每完成一个主 section 应可 `cargo test` 相关模块并通过；不要攒到最后再测。

## M1 — Data model & effective importance

- [x] 1.1 Add `recall_count`, `hit_count`, `last_reinforced_at`, `superseded_by` to `MemoryEntry` with `#[serde(default)]` (`src/context/mod.rs`)
- [x] 1.2 Init defaults in `MemoryEntry::new()`; add `reinforce(&mut self, now: DateTime<Utc>)` (`hit_count += 1`, set `last_reinforced_at`)
- [x] 1.3 Add `stale_marked_at: Option<DateTime<Utc>>` (or equivalent) with serde default for idempotent staleness
- [x] 1.4 Implement shared `type_half_life_hours(memory_type, base_age_threshold) -> f64` from existing `should_keep` TTL multipliers
- [x] 1.5 Implement `MemoryEntry::effective_importance(&self, now, cfg) -> f32`:
  - superseded → 0
  - else `base * decay * (0.5 + hitrate) * stale_mul` with `hitrate` clamped to [0, 1]
  - anchor = `last_reinforced_at.unwrap_or(timestamp)`
  - `stale_mul = staleness_penalty` if stale marked else 1.0
- [x] 1.6 Unit tests: legacy JSON loads defaults; decay curve; hit-rate damping; never-recalled neutral (hitrate factor 1.0); superseded → 0; stale multiplier applied once via flag not stacked on base

## M2 — Wire effective importance into recall & retention

- [x] 2.1 `inject` recall path: filter/sort by `effective_importance` instead of raw `importance`; exclude superseded
- [x] 2.2 `format_global` / global soft-cap: order by effective importance
- [x] 2.3 `should_keep`: use effective importance vs threshold; age/TTL path stays coherent with half-life helper
- [x] 2.4 On successful project-memory injection into `<memory-context>`, increment `recall_count` and persist (respect existing lock order: memories write, then index if needed)
- [x] 2.5 `list_memories` (CLI list): sort and min filter by effective importance; superseded remain listable at effective 0
- [x] 2.6 Unit/integration tests: superseded excluded from recall block; global cap uses effective ordering; list order follows effective; inject persists recall_count

## M3 — Tier-1 contradiction & supersede in `add_memory`

- [x] 3.1 Implement `classify_relation(new, existing) -> Compatible | Contradicts | Ambiguous` (state-change markers + numeric drift + subset; **conservative**)
- [x] 3.2 Change `add_memory` similar-branch (Jaccard ≥ 0.6):
  - Compatible → merge + `reinforce` + persist
  - Contradicts → set existing `superseded_by = new.id`, persist existing, insert new standalone (no hard delete)
  - Ambiguous → merge + set metadata/pending flag only (**no LLM**)
- [x] 3.3 Tool/`MemoryAddResult` remains truthful (`merged` / ids); document supersede in result if cheap (optional field ok)
- [x] 3.4 Unit tests: state-change supersede; value-drift supersede; subset compatible + reinforce; ambiguous flags without delete; superseded file still on disk

## M4 — Idempotent codebase staleness in `consolidate`

- [x] 4.1 Path-extraction helper + filesystem existence check; gated by `staleness_check`
- [x] 4.2 Mark only when **all** extracted paths are missing; if not yet marked set `stale_marked_at`; **do not** multiply base `importance`; **do not** refresh `last_reinforced_at`; partial-missing does not mark
- [x] 4.3 Second consolidate on same entry is no-op for staleness
- [x] 4.4 Tests: all-missing marked once; partial-missing unmarked; existing-only untouched; consolidate remains LLM-free; effective reflects penalty after mark

## M5 — Optional exploration

- [x] 5.1 Config `exploration_epsilon` default **0.0**
- [x] 5.2 When epsilon > 0, with that probability replace lowest-ranked injected project memory with low-effective, non-superseded, not-recently-recalled candidate; maintain session-local recent set
- [x] 5.3 Tests: epsilon=0 disables; epsilon=1 with fixture replaces slot when candidate exists

## M6 — First-consolidate anchor migration & config surface

- [x] 6.1 On consolidate (or dream entry that calls consolidate): for each memory with `last_reinforced_at=None`, set `Some(now)` once and persist (idempotent)
- [x] 6.2 Add settings keys + defaults: `exploration_epsilon`, `staleness_check`, `staleness_penalty`（`supersede_penalty` only if still used; prefer tombstone-only）
- [x] 6.3 Thread config into effective_importance / inject / consolidate
- [x] 6.4 Update `WGENTY.md` memory config table
- [x] 6.5 `cargo test` (memory/context-related + full if practical), `cargo clippy --all-targets -- -D warnings`, `cargo fmt -- --check`
- [x] 6.6 Spec compliance pass: every ADDED/MODIFIED scenario in this change’s `specs/agent-memory/spec.md` has a test or explicit verification note

## Deferred (do not implement in this change)

Track only as future changes; no checkboxes to complete here:

- Engagement attribution window
- Dream Tier-2 LLM ambiguous resolution
- Episodic directory + replay_extract
- Symbol multi-cue recall
- pain_score friction aggregation
- Hot-path restate / read-time LLM write-back

## Verification — scenario → test or note (M6.6 / Task 8)

6.3 Config end-to-end: `MemorySettings` → `MemoryManager::with_settings` → inject/consolidate/`effective_importance_cfg()`. Tests: `with_settings_reads_consolidation_thresholds`, `new_for_test_uses_exploration_and_staleness_defaults`, `consolidate_staleness_check_false_skips_mark`.

6.4 `WGENTY.md` table: `exploration_epsilon=0.0`, `staleness_check=true`, `staleness_penalty=0.5` documented.

6.5 Quality gates (context lib + clippy -D warnings + fmt --check): all green on `488eee4`.

### ADDED scenarios (all covered)

| Scenario | Test |
|---|---|
| Decay reduces importance over time | `effective_importance_decays_with_age` |
| Hit-rate damping penalizes recall noise | `effective_importance_hit_rate_damping` |
| Never-recalled memory is neutral on hit-rate | `effective_importance_never_recalled_hitrate_neutral` |
| Superseded memory has zero effective importance | `effective_importance_superseded_is_zero` |
| Stale-marked memory is downweighted | `effective_importance_stale_multiplier` |
| State-change marker triggers supersede | `classify_relation_state_change_marker_is_contradicts` |
| Numeric value drift triggers supersede | `classify_relation_numeric_drift_is_contradicts` |
| Subset relation is compatible, not contradiction | `classify_relation_subset_is_compatible` |
| Ambiguous pair flagged without LLM | `classify_relation_similar_but_unrelated_choice_is_ambiguous` + `add_memory_ambiguous_merges_and_flags_metadata` |
| Superseded memory retained on disk | `add_memory_contradicts_supersedes_and_recall_excludes_old` (disk afterwards) + `add_memory_skips_superseded_as_merge_target` (skipped target) |
| All extracted paths missing marks stale once | `consolidate_marks_stale_when_all_extracted_paths_missing` |
| Partial path missing does not mark stale | `consolidate_partial_missing_paths_does_not_mark_stale` |
| Already-marked stale memory is not stacked | `consolidate_stale_mark_is_idempotent_and_keeps_base_importance` |
| Existing-file-only reference unaffected | `consolidate_existing_only_paths_does_not_mark_stale` |
| Staleness check is LLM-free | `consolidate_remains_llm_free_structural` (covers stale path too) |
| Exploration disabled when epsilon is zero | `recall_exploration_epsilon_zero_never_replaces` |
| Exploration replaces lowest-ranked slot when enabled | `recall_exploration_force_draw_replaces_lowest_with_cold` + `recall_exploration_skips_recently_explored_cold` + `recall_exploration_skips_superseded_cold_candidate` |

### MODIFIED scenarios — changed by this change (covered)

| Scenario | Test |
|---|---|
| Legacy memory JSON loads with feedback-field defaults | `legacy_memory_json_defaults_feedback_fields` |
| Superseded memory excluded from recall | `recall_excludes_superseded_memories`, `format_global_excludes_superseded_memories` |
| Injected project memories increment recall_count | `recall_increments_and_persists_project_recall_count` |
| List ordering uses effective importance | `list_memories_orders_by_effective_not_raw`, `list_memories_keeps_superseded_but_filters_by_effective` |
| Global memories injected every turn (sort by effective) | `format_global_orders_by_effective_not_raw_importance` |
| Global memory soft cap exceeded (>50 + top-50 + warning) | `format_global_returns_all_global_memories_sorted_by_importance` (ordering + cap); warning log path accepted as non-blocking baseline |
| First consolidate anchors missing last_reinforced_at | `consolidate_anchors_missing_last_reinforced_at` |
| Consolidation is LLM-free (incl. stale step) | `consolidate_remains_llm_free_structural` |
| Compatible similar memory merges and reinforces | `add_memory_compatible_merges_and_reinforces` |
| Contradicting similar memory supersedes via tombstone | `add_memory_contradicts_supersedes_and_recall_excludes_old` |
| Ambiguous similar memory merges and flags without LLM | `add_memory_ambiguous_merges_and_flags_metadata` |

### MODIFIED scenarios — pre-existing behavior, unchanged by this change (note, not new test)

| Scenario | Note |
|---|---|
| Project memory persisted to project-local directory | Pre-existing; covered by storage/`new_for_test` isolation |
| Global memory persisted to global directory | Pre-existing; global storage path unchanged |
| CWD unavailable degrades to global storage | Pre-existing dual-storage fallback; no new path |
| CWD equals home directory | Pre-existing merge-to-global; unchanged |
| Global memories injected every turn | Pre-existing inject path; effective sort added above |
| Project memories recalled by keyword | Pre-existing recall + TF-IDF; effective filter/sort added above |
| No global memories | Pre-existing empty-global branch; effective no-op |
| Consolidation gate (pass / fail-time / fail-throttle / lock / autodream-lock / headless / daemon / TUI) | Pre-existing AutoDream gates; no change to triggering |
| Tool returns memory_id on success | Pre-existing result shape; merged/supersede still truthful |
| Invalid memory_type rejected | Pre-existing validation; unchanged |
| Missing content rejected | Pre-existing validation; unchanged |
| Tool registered in daemon/headless/available-to-all-agents | Pre-existing registration; unchanged |

**Summary: 17 ADDED scenarios all covered by explicit tests. 11 MODIFIED scenarios changed are covered by tests. 19 MODIFIED scenarios unchanged are noted as pre-existing baseline.**
