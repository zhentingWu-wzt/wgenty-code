# Implementation Tasks

> **Scope lock:** 仅 M1–M5（effective importance、Tier-1 supersede、幂等 staleness、可选 exploration、配置/迁移/文档）。  
> **不做：** engagement 归因、Tier-2 LLM、情节层/replay、符号多线索、pain_score、热路径 restate。  
> 每完成一个主 section 应可 `cargo test` 相关模块并通过；不要攒到最后再测。

## M1 — Data model & effective importance

- [ ] 1.1 Add `recall_count`, `hit_count`, `last_reinforced_at`, `superseded_by` to `MemoryEntry` with `#[serde(default)]` (`src/context/mod.rs`)
- [ ] 1.2 Init defaults in `MemoryEntry::new()`; add `reinforce(&mut self, now: DateTime<Utc>)` (`hit_count += 1`, set `last_reinforced_at`)
- [ ] 1.3 Add `stale_marked_at: Option<DateTime<Utc>>` (or equivalent) with serde default for idempotent staleness
- [ ] 1.4 Implement shared `type_half_life_hours(memory_type, base_age_threshold) -> f64` from existing `should_keep` TTL multipliers
- [ ] 1.5 Implement `MemoryEntry::effective_importance(&self, now, cfg) -> f32`:
  - superseded → 0
  - else `base * decay * (0.5 + 0.5 * hitrate) * stale_mul`
  - anchor = `last_reinforced_at.unwrap_or(timestamp)`
  - `stale_mul = staleness_penalty` if stale marked else 1.0
- [ ] 1.6 Unit tests: legacy JSON loads defaults; decay curve; hit-rate damping; never-recalled neutral (hitrate factor 1.0); superseded → 0; stale multiplier applied once via flag not stacked on base

## M2 — Wire effective importance into recall & retention

- [ ] 2.1 `inject` recall path: filter/sort by `effective_importance` instead of raw `importance`; exclude superseded
- [ ] 2.2 `format_global` / global soft-cap: order by effective importance
- [ ] 2.3 `should_keep`: use effective importance vs threshold; age/TTL path stays coherent with half-life helper
- [ ] 2.4 On successful project-memory injection into `<memory-context>`, increment `recall_count` and persist (respect existing lock order: memories write, then index if needed)
- [ ] 2.5 `list_memories` (CLI list): sort and min filter by effective importance; superseded remain listable at effective 0
- [ ] 2.6 Unit/integration tests: superseded excluded from recall block; global cap uses effective ordering; list order follows effective; inject persists recall_count

## M3 — Tier-1 contradiction & supersede in `add_memory`

- [ ] 3.1 Implement `classify_relation(new, existing) -> Compatible | Contradicts | Ambiguous` (state-change markers + numeric drift + subset; **conservative**)
- [ ] 3.2 Change `add_memory` similar-branch (Jaccard ≥ 0.6):
  - Compatible → merge + `reinforce` + persist
  - Contradicts → set existing `superseded_by = new.id`, persist existing, insert new standalone (no hard delete)
  - Ambiguous → merge + set metadata/pending flag only (**no LLM**)
- [ ] 3.3 Tool/`MemoryAddResult` remains truthful (`merged` / ids); document supersede in result if cheap (optional field ok)
- [ ] 3.4 Unit tests: state-change supersede; value-drift supersede; subset compatible + reinforce; ambiguous flags without delete; superseded file still on disk

## M4 — Idempotent codebase staleness in `consolidate`

- [ ] 4.1 Path-extraction helper + filesystem existence check; gated by `staleness_check`
- [ ] 4.2 Mark only when **all** extracted paths are missing; if not yet marked set `stale_marked_at`; **do not** multiply base `importance`; **do not** refresh `last_reinforced_at`; partial-missing does not mark
- [ ] 4.3 Second consolidate on same entry is no-op for staleness
- [ ] 4.4 Tests: all-missing marked once; partial-missing unmarked; existing-only untouched; consolidate remains LLM-free; effective reflects penalty after mark

## M5 — Optional exploration

- [ ] 5.1 Config `exploration_epsilon` default **0.0**
- [ ] 5.2 When epsilon > 0, with that probability replace lowest-ranked injected project memory with low-effective, non-superseded, not-recently-recalled candidate; maintain session-local recent set
- [ ] 5.3 Tests: epsilon=0 disables; epsilon=1 with fixture replaces slot when candidate exists

## M6 — First-consolidate anchor migration & config surface

- [ ] 6.1 On consolidate (or dream entry that calls consolidate): for each memory with `last_reinforced_at=None`, set `Some(now)` once and persist (idempotent)
- [ ] 6.2 Add settings keys + defaults: `exploration_epsilon`, `staleness_check`, `staleness_penalty`（`supersede_penalty` only if still used; prefer tombstone-only）
- [ ] 6.3 Thread config into effective_importance / inject / consolidate
- [ ] 6.4 Update `WGENTY.md` memory config table
- [ ] 6.5 `cargo test` (memory/context-related + full if practical), `cargo clippy --all-targets -- -D warnings`, `cargo fmt -- --check`
- [ ] 6.6 Spec compliance pass: every ADDED/MODIFIED scenario in this change’s `specs/agent-memory/spec.md` has a test or explicit verification note

## Deferred (do not implement in this change)

Track only as future changes; no checkboxes to complete here:

- Engagement attribution window
- Dream Tier-2 LLM ambiguous resolution
- Episodic directory + replay_extract
- Symbol multi-cue recall
- pain_score friction aggregation
- Hot-path restate / read-time LLM write-back
