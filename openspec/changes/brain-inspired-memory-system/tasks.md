# Implementation Tasks

> 按 pillar 分组。**P1 是其他 pillar 的地基**(effective_importance + LLM 分离 invariant 被 P2/P5 复用),必须先做。P3/P4 无依赖可并行。P2 完成后 P5 的重述才有意义。

## Pillar 1 -- 动态 importance 与反馈回路(地基,最先做)

### 1A. Data Model
- [ ] 1.1 Add `recall_count`/`hit_count`/`last_reinforced_at`/`superseded_by` to `MemoryEntry` (`src/context/mod.rs`) with `#[serde(default)]`
- [ ] 1.2 Init the four fields in `MemoryEntry::new()`; add `reinforce(&mut self, now)` helper
- [ ] 1.3 Unit test: legacy JSON (lacking new fields) loads with documented defaults

### 1B. Effective Importance
- [ ] 1.4 Add `type_half_life_hours(memory_type, base)` reusing `should_keep` TTL multipliers (`consolidation.rs:147`)
- [ ] 1.5 Implement `MemoryEntry::effective_importance(&self, now, cfg) -> f32` (superseded->0; else base*decay*(0.5+0.5*hitrate))
- [ ] 1.6 Replace raw `importance` with `effective_importance` in `recall` (`inject.rs:41`), `should_keep` (`consolidation.rs:135`), `format_global` (`inject.rs:74`)
- [ ] 1.7 Unit tests: decay curve, hit-rate damping, never-recalled neutral, superseded->0

### 1C. Contradiction Detection & Supersede (Tier-1)
- [ ] 1.8 Implement `classify_relation(new, existing) -> Relation` (state-change markers, value drift, subset)
- [ ] 1.9 Modify `add_memory` (`mod.rs:451`): Compatible->merge+reinforce; Contradicts->tombstone+penalty+standalone; Ambiguous->merge+flag
- [ ] 1.10 Add pending-ambiguous-pairs list for Tier-2
- [ ] 1.11 Unit tests: state-change supersede, value-drift supersede, subset compatible, ambiguous flag

### 1D. Dream Tier-2 LLM Supersede Resolution
- [ ] 1.12 Add separate `MemoryManager::resolve_ambiguous_pairs()` invoked by `dream` AFTER `consolidate()`; verify `consolidate()` stays LLM-free
- [ ] 1.13 Design batch LLM classification prompt + JSON parse; apply supersede/merge/both; clear pending list
- [ ] 1.14 No-op (no LLM call) when no pending pairs
- [ ] 1.15 Integration test (mock LLM): batch-classified + applied; empty list -> no call

### 1E. Codebase Staleness Decay
- [ ] 1.16 Add path-extraction regex + filesystem existence check in `consolidate()`, gated by `staleness_check` config
- [ ] 1.17 On stale path: `importance *= staleness_penalty`; do NOT refresh `last_reinforced_at`
- [ ] 1.18 Test: deleted-file reference decayed; existing-file unaffected; verify no LLM call

### 1F. Engagement Attribution Window
- [ ] 1.19 Expose `MemoryIndex::distinctive_tokens(text, min_idf)` reusing IDF table
- [ ] 1.20 Implement `RecallAttribution` (window/last_user_tokens/turn_counter/decay_tau) + `settle` (topic boundary, recency decay, distinctive match) + `record`
- [ ] 1.21 Integrate into agent loop: settle->recall->record per turn
- [ ] 1.22 Integration tests: immediate reinforcement, delayed partial credit, topic boundary close, low-IDF no-trigger, self-expire

### 1G. Recall Exploration
- [ ] 1.23 In `recall`, with prob `exploration_epsilon` replace lowest-ranked project memory with low-effective unrecalled one; maintain recently-recalled set
- [ ] 1.24 Unit test: exploration replaces slot; epsilon=0 disables

## Pillar 3 -- 符号感知多线索召回(无依赖,可与 P1 并行)

- [ ] 3.1 Collect current task symbol context (open files, edited symbol, stack frames) in agent loop
- [ ] 3.2 Add symbol extraction from memory content (CodeGraph/LSP symbol table or regex)
- [ ] 3.3 Extend recall scoring: `score = α·tfidf + β·symbol_overlap + γ·recency`
- [ ] 3.4 Unit tests: symbol overlap augments ranking; works without embeddings; weights configurable

## Pillar 4 -- pain_score salience(无依赖,可与 P1 并行)

- [ ] 4.1 Collect friction signals in agent loop: exec_command failure/retry count, guardian denials, user corrections, undo calls
- [ ] 4.2 Aggregate per-turn `pain_score`; inject into compaction extraction prompt so LLM records it in importance/metadata
- [ ] 4.3 Higher-pain memories get higher consolidation weight (used by P2 replay); slower decay
- [ ] 4.4 Test: friction raises pain_score; high-pain prioritized in replay; no new storage for v1

## Pillar 2 -- 情节/语义分层 + 离线 replay 巩固(依赖 P1 的 LLM 分离 invariant)

### 2A. Episodic Store
- [ ] 2.1 New module `src/context/episodes.rs`: store at `<project>/.wgenty-code/episodes/`, filename `<YYYYMMDD-HHMM>-<ascii-slug>-<shortid>.json`, id in JSON content, append-mostly
- [ ] 2.2 ASCII slug derivation (from symbols/keywords, non-ASCII goes to content only); shortid uniqueness
- [ ] 2.3 Write episodic entries at decision points / session-end summary; record pain_score + decisions/files/bugs/requests
- [ ] 2.4 Exclude episodic entries from semantic TF-IDF index
- [ ] 2.5 Tests: chronological ls, ASCII-safe filename, semantic naming unchanged, excluded from index

### 2B. Offline Replay Consolidation
- [ ] 2.6 Add separate `MemoryManager::replay_extract()` invoked by `dream` AFTER `consolidate()`; verify `consolidate()` stays LLM-free
- [ ] 2.7 Design batch LLM replay prompt: extract fact->semantic, dedupe episodes, resolve contradiction (supersede), prune low-freq-low-pain
- [ ] 2.8 Higher-pain episodes prioritized (consume P4 pain_score)
- [ ] 2.9 No-op (no LLM call) when no unaired episodes
- [ ] 2.10 Integration tests (mock LLM): extract fact, dedupe, contradiction, prune, no-op, runs-after-consolidate

## Pillar 5 -- 召回时重构(依赖 P2 情节层)

### 5A. Restate at Recall
- [ ] 5.1 In recall, if injected memory is episodic and exceeds length threshold, run LLM restate pass (extract query-relevant slice); short semantic facts injected verbatim
- [ ] 5.2 Test: long episodic restated; short fact verbatim (no LLM call)

### 5B. Read-time Reconsolidation (grounding check)
- [ ] 5.3 On recall, grounding-check memory-referenced symbols/files against current codebase; if changed, emit soft "verify me" signal or trigger write-back
- [ ] 5.4 Test: stale-at-recall triggers verify signal; unchanged memory unaffected

## Pillar 6 -- Config, Migration & Verification

- [ ] 6.1 Add config keys: `decay_tau_turns`(2.0), `exploration_epsilon`(0.15), `supersede_penalty`(0.3), `staleness_check`(true), `staleness_penalty`(0.5), multi-cue weights `recall_alpha/beta/gamma`, `restate_length_threshold`, `pain_*` weights
- [ ] 6.2 First-dream anchor migration: set `last_reinforced_at=Some(now)` for all None (idempotent)
- [ ] 6.3 `cargo test --all` passing (all new unit/integration tests)
- [ ] 6.4 `cargo clippy --all-targets -- -D warnings` (zero warnings)
- [ ] 6.5 `cargo fmt -- --check` (clean)
- [ ] 6.6 Update `WGENTY.md` config table with all new memory config keys
- [ ] 6.7 Cross-verify spec compliance: each ADDED/MODIFIED requirement scenario has a corresponding test or verification note
