---
comet_change: brain-inspired-memory-system
role: technical-design
canonical_spec: openspec
---

# Memory Reliability Foundation — Technical Design

## 1. Purpose

Close the open loop in `agent-memory` without expanding into episodic/LLM/dream pipelines:

1. Read-time **effective importance** (decay + hit-rate damping + staleness + tombstone→0)
2. Write-time **Tier-1 supersede** (tombstone, no hard delete)
3. Consolidate-time **idempotent path staleness** (still LLM-free)
4. Optional **exploration** (default off)
5. **Consistent surfaces**: recall, retention, global cap, CLI list

Canonical capability requirements live in OpenSpec:

- Baseline: `openspec/specs/agent-memory/spec.md`
- Delta: `openspec/changes/brain-inspired-memory-system/specs/agent-memory/spec.md`

This document is implementation design only.

## 2. Non-goals

- Engagement attribution window
- Tier-2 LLM ambiguous resolution in dream
- Episodic store / replay_extract
- Symbol multi-cue recall, pain_score, hot-path restate
- Embeddings, background decay timers, hard-delete of superseded rows
- Any LLM inside `consolidate()`

## 3. Code map

| Concern | Location |
|---------|----------|
| `MemoryEntry`, `add_memory`, `consolidate`, `list_memories` | `src/context/mod.rs` |
| Per-turn recall / global formatting | `src/context/inject.rs` |
| `should_keep`, similarity, `ConsolidationConfig` | `src/context/consolidation.rs` |
| Persistence | `src/context/storage.rs` (filename = id unchanged) |
| Settings | `storage.memory` in config / `ConsolidationConfig::from_memory_settings` |
| Tool surface | `src/tools/meta/memory_add.rs` (result shape preserved) |
| Docs | `WGENTY.md` config table |

No new top-level subsystem.

## 4. Data model

Extend `MemoryEntry` with `#[serde(default)]` fields:

| Field | Type | Default | Role |
|-------|------|---------|------|
| `recall_count` | `u32` | 0 | Times injected into `<memory-context>` |
| `hit_count` | `u32` | 0 | Positive feedback (Compatible `reinforce`) |
| `last_reinforced_at` | `Option<DateTime<Utc>>` | `None` | Decay anchor; `None` → use `timestamp` until first consolidate anchors |
| `superseded_by` | `Option<String>` | `None` | Tombstone target id |
| `stale_marked_at` | `Option<DateTime<Utc>>` | `None` | Idempotent codebase-staleness mark |

Invariants:

- On-disk filename remains stable UUID `id`
- Superseded rows are retained on disk
- Legacy JSON loads without migration

Helpers on `MemoryEntry`:

- `reinforce(now)` → `hit_count += 1`, `last_reinforced_at = Some(now)` (does **not** raise base `importance`)
- `effective_importance(now, cfg) -> f32` (pure)

## 5. Effective importance

```text
anchor     = last_reinforced_at.unwrap_or(timestamp)
hitrate    = (hit_count + 1) / (recall_count + 2)          # Laplace
decay      = exp(-ln2 * hours_since(anchor) / type_half_life)
stale_mul  = cfg.staleness_penalty if stale_marked_at.is_some() else 1.0
effective  = 0.0 if superseded_by.is_some()
             else base_importance * decay * (0.5 + 0.5 * hitrate) * stale_mul
```

`type_half_life_hours(memory_type, age_threshold_hours)` **shares** the existing `should_keep` per-type TTL multipliers:

| Types | Multiplier vs base age threshold |
|-------|----------------------------------|
| Knowledge, Preference | ×4 |
| Decision, Insight | ×2 |
| Error | ×0.5 (min 1h semantics preserved in helper) |
| Session, Conversation, Task | ×1 |

Call sites that switch from raw `importance` to effective:

- `inject::recall` filter + sort (+ display score)
- `inject::format_global` sort / soft cap
- `ConsolidationEngine::should_keep` retention threshold comparison
- `MemoryManager::list_memories` min filter + sort

`now` is `Utc::now()` at the call site (or injected in tests).

## 6. Write path: `add_memory` relation classification

When same-scope Jaccard similarity ≥ `0.6` (existing threshold), run Tier-1 `classify_relation(new, existing)` instead of unconditional merge.

### 6.1 Classification (conservative)

| Relation | Rule sketch |
|----------|-------------|
| **Contradicts** | High similarity **and** (state-change marker in new/old pair such as fixed/resolved/removed/deprecated/migrated/no longer **or** shared key-like token with clear numeric drift) |
| **Compatible** | Subset / same-direction refinement |
| **Ambiguous** | Everything else — **prefer Ambiguous over false Contradicts** |

Gold-unit cases must lock the above (including “auth bug exists” / “auth bug fixed”).

### 6.2 Actions

| Relation | Persist behavior |
|----------|------------------|
| Compatible | `merge_into` existing; `reinforce(now)` on merged; save; project → `index.replace_entry` |
| Contradicts | Set `existing.superseded_by = new.id`; save existing; **insert new standalone** (new id); do **not** change existing base `importance`; do **not** delete file |
| Ambiguous | `merge_into`; set `metadata["relation_ambiguous"] = true` (optional peer id); save; **no LLM** |

### 6.3 Tool result

Keep `success`, `memory_id`, `merged`:

- Compatible / Ambiguous merge: `merged=true`, `memory_id=existing id`
- Contradicts: `merged=false`, `memory_id=new id`

## 7. Read path: recall / global / list

### 7.1 `inject::recall`

1. Existing keyword extraction + `search_memories`
2. Drop entries with `superseded_by.is_some()`
3. Filter `effective >= threshold`, sort by effective desc, take `top_n`
4. Optional exploration if `exploration_epsilon > 0` (default **0** = off):
   - With that probability, replace lowest-ranked injected project memory with a cold candidate: not superseded, not in current top, not in session-local recently-explored set, prefer low effective / low recall_count
5. For each injected **project** memory: `recall_count += 1`, persist via storage under the same lock discipline as `add_memory` (memories write lock, then save; touch index only if content tokens changed — count-only updates may skip index rebuild)
6. Format block; prefer printing **effective** in the importance field for honesty

Global memories are not in this block → no `recall_count` bump on globals here.

### 7.2 `format_global`

Sort by effective; soft cap 50; superseded globals sort as 0 (normally absent from useful head).

### 7.3 `list_memories`

- `min_importance` compares against **effective**
- Sort by effective desc, then timestamp
- Superseded rows **remain listable** (effective 0) for audit

## 8. Consolidate path (LLM-free)

Inside `MemoryManager::consolidate` (after lock, while holding project write lock), before or as part of feeding the engine:

1. **Anchor migration (idempotent):** each memory with `last_reinforced_at.is_none()` → `Some(consolidate_now)`
2. **Staleness (if `staleness_check`):**
   - Extract conservative filesystem-relative / source paths from `content` (e.g. `src/...` with common suffixes; ignore bare URLs)
   - If **one or more** paths extracted **and all** are missing on disk **and** `stale_marked_at.is_none()` → set `stale_marked_at = Some(now)`
   - Never multiply base `importance` for staleness
   - Never refresh `last_reinforced_at` solely due to staleness check
   - Already marked → no-op
3. Run `ConsolidationEngine::consolidate` with `should_keep` using effective importance
4. Existing `reconcile` + TF-IDF `rebuild`

Global prune path that reuses the engine should apply the same anchor/stale/effective retention rules for consistency when practical (project path is the spec focus for path checks).

## 9. Configuration

Extend memory settings (names may match existing nesting under `storage.memory`):

| Key | Default | Used by |
|-----|---------|---------|
| `exploration_epsilon` | `0.0` | recall |
| `staleness_check` | `true` | consolidate |
| `staleness_penalty` | `0.5` | effective_importance |

Do **not** add `supersede_penalty` in v1 (tombstone already forces effective 0).

Thread cfg into `MemoryManager` / inject / consolidation helpers as needed (constructor `with_settings` already reads memory settings).

Document keys in `WGENTY.md`.

## 10. Concurrency and performance

- `add_memory` already waits on `consolidating`; keep that
- `recall_count` updates: bounded by `top_n` saves per turn; acceptable under `max_memories`
- Avoid lock-order inversion: never hold index lock while acquiring memories write lock if other paths take memories first (match `search_memories` comments)
- Count-only field updates: saving full JSON is fine; index postings unchanged if content/tags unchanged

## 11. Testing

### Unit

- Legacy JSON → defaults for new fields
- effective: decay over half-lives; hit-rate damping; never-recalled hitrate factor 1.0; superseded 0; stale multiplier once via flag
- `classify_relation` gold cases: state-change, numeric drift, subset compatible, ambiguous
- Staleness: all-missing marks once; partial-missing does not mark; second consolidate no-op
- `exploration_epsilon = 0` disables replacement

### Integration

- Supersede then recall/search injection path excludes old id
- After recall inject, reloaded entry has higher `recall_count`
- `list_memories` order follows effective (fixture with controlled timestamps/counts)

### Structural

- Consolidate path does not construct/call LLM client

## 12. Migration and rollback

1. Deploy binary → old files load via serde defaults  
2. First consolidate anchors `last_reinforced_at` and may set stale marks  
3. Rollback binary → new fields ignored; behavior returns to static importance (tombstone field residual is harmless)

## 13. Spec patch summary

Delta spec updates (same change folder):

- Document inject-time `recall_count` persistence
- Stale rule = all extracted paths missing + idempotent `stale_marked_at`
- List/CLI effective ordering; superseded listable
- No supersede_penalty requirement

## 14. Implementation order (aligns with tasks.md)

1. Data model + effective helper + unit tests  
2. Wire effective into recall / global / should_keep / list  
3. `add_memory` classify + reinforce/supersede  
4. Consolidate anchor + staleness  
5. Exploration + config + WGENTY.md + full lint/test gate  
