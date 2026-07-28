# Comet Design Handoff

- Change: brain-inspired-memory-system
- Phase: design
- Mode: compact
- Context hash: 8a541a56b0576e913ada71f827fbf2e42821507f761596c370ae34df8f72f006

Generated-by: comet-handoff.sh

OpenSpec remains the canonical capability spec. This handoff is a deterministic, source-traceable context pack, not an agent-authored summary.

## openspec/changes/brain-inspired-memory-system/proposal.md

- Source: openspec/changes/brain-inspired-memory-system/proposal.md
- Lines: 1-91
- SHA256: de2801e586be0f82774d32bb3b8332570b0772012948915ee04f7f08982af871

[TRUNCATED]

```md
## Why

wgenty-code 的 `agent-memory` 当前是开环检索：扁平短事实 + TF-IDF 关键词召回 + 静态 importance + 本地 `consolidate()`。记忆写一次定终身——有用的不强化、过时的不失效、矛盾的仍被召回。这不是“缺 embedding”，而是**缺少反馈回路与 grounding**。

本次 change 只补最能立刻提升可靠性、且不依赖 agent-loop 大改造 / 热路径 LLM 的地基：

1. **importance 开环** → 读时 `effective_importance`（时间衰减 + 命中率阻尼 + tombstone→0）
2. **矛盾无取代** → Tier-1 启发式 supersede（tombstone，不硬删）
3. **无代码库校验** → consolidate 时对消失路径做**幂等** staleness 标记
4. **rich-get-richer** → 可选 ε 探索（默认关或低）

情节层 / replay / 符号多线索 / pain_score / 热路径 restate 仍是正确方向，但作为**后续 change**，避免本 change 变成无法独立验收的 epic。

## What Changes

### In scope（本 change，可独立合并与回滚）

**M1 — 动态 effective importance（地基）**
- `MemoryEntry` +4 字段：`recall_count` / `hit_count` / `last_reinforced_at` / `superseded_by`（`#[serde(default)]` 零迁移）
- `effective_importance(now, cfg)` 纯读时函数；recall 排序/阈值、`should_keep`、`format_global` 改用 effective
- 首次 dream 对 `last_reinforced_at=None` 做一次性锚定（幂等）

**M2 — 写时矛盾取代（Tier-1 only）**
- `add_memory` 在相似度 ≥ 0.6 时 `classify_relation` → Compatible / Contradicts / Ambiguous
- Compatible → merge + `reinforce`（`hit_count++`, 刷新 `last_reinforced_at`）
- Contradicts → 旧条 tombstone（`superseded_by`），降权，新条独立写入；**不硬删**
- Ambiguous → **保守默认 merge + 结构化 flag**（metadata/pending 列表）；**本 change 不做 Tier-2 LLM 批分类**（避免 dream 范围膨胀；flag 留待 follow-up）

**M3 — 代码库 staleness（幂等 grounding）**
- `consolidate()` 内本地路径存在性检查（保持 LLM-free）
- 使用 `metadata`/`stale_marked_at` 等**一次性标记**，禁止对 base `importance` 反复叠乘
- effective 计算读取 staleness multiplier；不刷新 `last_reinforced_at`

**M4 — 召回探索（可选，默认安全）**
- `exploration_epsilon` 默认 `0`（关闭）；>0 时替换最低档 project memory 为冷记忆
- 轻量 recently-recalled 集合防刷

**M5 — 配置、迁移、文档与回归**
- 配置项带默认；旧 JSON 兼容；`WGENTY.md` 更新
- 单测覆盖 decay / hitrate / supersede / staleness 幂等 / epsilon=0

### Explicitly deferred（文档保留方向，本 change 不实现、不写进 MUST spec）

| 原 pillar | 原因 | 后续 change 建议名 |
|-----------|------|-------------------|
| P1 Tier-2 LLM ambiguous 批分类 | 依赖 dream LLM 管线与 prompt 设计，可独立加 | `memory-ambiguous-llm-resolve` |
| P1 engagement 归因窗口 | agent-loop 集成 + 假阳性风险，非地基 | `memory-engagement-attribution` |
| P2 情节层 + replay_extract | 写入粒度/读路径/与 TF-IDF 关系未钉死；范围大 | `memory-episodic-replay` |
| P3 符号多线索召回 | 符号上下文采集在 loop 中未必“现成” | `memory-symbol-multicue-recall` |
| P4 pain_score | 摩擦信号 inventory 不足；易 overclaim | `memory-pain-salience` |
| P5 热路径 restate + 读时 write-back | 与“情节不进 TF-IDF”冲突；热路径延迟 | 随 episodic；restate 仅 cold path |

### Non-goals（本 change）

- 不替换 TF-IDF 为 embedding
- 不在 `consolidate()` 内引入任何 LLM
- 不改语义记忆 id-as-filename 不变式
- 不硬删被取代记忆
- 不引入后台衰减定时器
- 不增加热路径 LLM 调用

## Capabilities

### New Capabilities
（无新 capability 名；增强已有 `agent-memory`）

### Modified Capabilities
- `agent-memory`:
  - 数据模型：反馈字段 + superseded tombstone
  - recall / global 排序 / consolidation retention：effective importance
  - `add_memory`：关系分类 + reinforce / supersede
  - `consolidate`：幂等 codebase staleness（仍 LLM-free）
  - 可选 recall exploration
  - 配置与首次锚定迁移

## Impact

- **代码**：`src/context/mod.rs`（MemoryEntry / add_memory / reinforce）、`src/context/inject.rs`（recall / format_global）、`src/context/consolidation.rs`（should_keep / staleness）、config、少量 dream 锚定钩子；**不强制大改 agent loop**（exploration 可先在 inject 层完成）
- **数据**：语义 JSON 零迁移；回滚时新字段被旧逻辑忽略；tombstone 文件保留可审计
- **性能**：每轮多一次纯函数计算（记忆数有上限）；staleness 为 consolidate 期路径探测
```

Full source: openspec/changes/brain-inspired-memory-system/proposal.md

## openspec/changes/brain-inspired-memory-system/design.md

- Source: openspec/changes/brain-inspired-memory-system/design.md
- Lines: 1-214
- SHA256: cbd07de7abc89b3f92c607b7e6c2dcbb3c51f470b56fcea8677813bb40531dbd

[TRUNCATED]

```md
## Context

wgenty-code `agent-memory` 今天是检索式闭环缺失系统：

- 编码：compaction / `memory_add` 写入短事实
- 召回：TF-IDF + **静态** `importance`
- 巩固：`consolidate()` 本地 merge + type TTL（**LLM-free**，1h/1session 门限依赖此前提）
- 遗忘：TTL 截断，无强化、无矛盾取代、无代码库 grounding

原 5-pillar 脑启发方案方向正确，但作为**单一 change** 范围过大：情节层读路径未定义、P5 与“情节不进索引”冲突、P3/P4 低估 agent-loop 采集成本、Tier-2/engagement/restate 引入 LLM 与假阳性。

本 design 将范围收束为 **Memory Reliability Foundation（M1–M5）**：只交付可独立合并、可回滚、可测的反馈回路地基；其余 pillar 降为 follow-up 方向（见文末）。

## Goals / Non-Goals

**Goals**
- 读时动态 importance，打破“写一次定终身”
- 写时矛盾 → tombstone 取代（可审计、可逆）
- consolidate 期用代码库做**幂等**过时衰减（agent 独有 grounding）
- 保持 `consolidate()` **永远 LLM-free**
- 语义记忆 id-as-filename 与零迁移兼容
- 默认行为保守（探索默认关；Ambiguous 不误杀）

**Non-Goals（本 change）**
- 情节层 / offline / dream LLM 后置管线
- engagement 归因窗口、符号多线索、pain_score
- 热路径 LLM restate / 读时 LLM write-back
- Tier-2 LLM 批分类 ambiguous pairs
- embedding、后台 timer、硬删

## 脑机制映射（本 change 实际落地部分）

| 大脑/可靠性格言 | 本 change |
|----------------|-----------|
| 强化 / 再巩固（弱） | Compatible merge 时 `reinforce`；计数为后续 engagement 留字段 |
| 遗忘衰减 | `effective_importance` 指数衰减（读时） |
| 主动抑制 / 取代 | `superseded_by` tombstone |
| grounding | path staleness 幂等标记 |
| 探索 vs 利用 | `exploration_epsilon`（默认 0） |
| 情节 / replay / 情绪 salience / 多线索 | **Deferred** |

## M1 — Effective importance

### 机制
`MemoryEntry` 增加（均 `#[serde(default)]`）：
- `recall_count: u32`（默认 0）— 本 change 可在注入时递增，供阻尼与后续 engagement
- `hit_count: u32`（默认 0）— Compatible reinforce 时 +1；engagement follow-up 再扩展
- `last_reinforced_at: Option<DateTime<Utc>>` — None 表示锚在 `timestamp`
- `superseded_by: Option<String>` — Some ⇒ 逻辑删除

```text
hitrate    = (hit_count + 1) / (recall_count + 2)          # Laplace
decay      = exp(-ln2 * hours_since(anchor) / type_half_life)
stale_mul  = staleness_penalty if stale_marked else 1.0
effective  = 0  if superseded_by.is_some()
           else base_importance * decay * (0.5 + 0.5*hitrate) * stale_mul
```

- `type_half_life` **复用**现有 `should_keep` 的 per-type TTL 倍率 × `age_threshold_hours`（与今日保留直觉对齐）
- **纯函数、读时计算、不写盘**
- 排序/过滤切换点：`inject.rs` recall、`format_global`、`consolidation.rs` `should_keep`

### 决策
- **D1 惰性衰减**：无后台 timer，无并发状态机
- **D2 hitrate 阻尼**：高频召回零命中转负反馈；never-recalled 中性（因子 1.0）
- **D3 锚定迁移**：首次 consolidate/dream 路径上，对 `last_reinforced_at=None` 写 `Some(now)` 一次，避免老数据被当成“已衰减很久”；操作幂等（已 Some 不改）

### 风险
- 高 base importance 但长期未强化会被降权甚至在 should_keep 中淘汰——**有意为之**；用配置 half-life/threshold 调节
- `recall_count` 在本 change 若只在 reinforce 路径有 hit、注入时 +recall，需避免每次 consolidate 误增；注入路径更新要持久化时注意锁序（先 memories 写锁，再 index，与现 `add_memory` 一致）

## M2 — Tier-1 contradiction & supersede

### 机制
在现有 Jaccard ≥ 0.6 去重分支上扩展，不再无条件 `merge_into`：

| Relation | 条件（保守启发式） | 行为 |
|----------|-------------------|------|
| Contradicts | 高相似 + 状态变化标记（fixed/resolved/removed/deprecated/migrated/no longer 等）或明显数值漂移 | 旧条 `superseded_by=new_id`，`importance *= supersede_penalty`（或写入固定降权一次），新条 standalone；**文件保留** |
| Compatible | 子集/同向细化 | merge + `reinforce` |
```

Full source: openspec/changes/brain-inspired-memory-system/design.md

## openspec/changes/brain-inspired-memory-system/tasks.md

- Source: openspec/changes/brain-inspired-memory-system/tasks.md
- Lines: 1-70
- SHA256: 8d9f720c23ab5e0204daa93ab6d80bf08b7c552ec4884b00f6007f32a42f0f30

```md
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
```

## openspec/changes/brain-inspired-memory-system/specs/agent-memory/spec.md

- Source: openspec/changes/brain-inspired-memory-system/specs/agent-memory/spec.md
- Lines: 1-293
- SHA256: e6fd7a3c9304774dedb092dbdfcce67a08d4ab0d2752c579c03b3281c9110867

[TRUNCATED]

```md
## MODIFIED Requirements

### Requirement: Memory storage via MemoryManager

All memories SHALL be stored exclusively via `MemoryManager`, using its per-file Storage backend. Memories SHALL be physically separated by scope:
- **Project memories** SHALL be stored at `<project_root>/.wgenty-code/memory/<id>.json`
- **Global memories** SHALL be stored at `~/.wgenty-code/memory/<id>.json`

`project_root` SHALL equal the current working directory (CWD), with no upward search for project markers. Each memory SHALL use the `context::MemoryEntry` type with fields: id, memory_type, content, timestamp, importance, tags, metadata, AND feedback-tracking fields `recall_count`, `hit_count`, `last_reinforced_at` (Option; None means decay anchors at `timestamp`), `superseded_by` (Option; id of the superseding memory), and `stale_marked_at` (Option; set when codebase-staleness has been applied). Feedback and staleness fields SHALL deserialize with defaults (`recall_count=0`, `hit_count=0`, `last_reinforced_at=None`, `superseded_by=None`, `stale_marked_at=None`) when absent so existing memory JSON loads without migration. The memory file's filename SHALL remain the stable `id` (UUID); semantic display slugs SHALL NOT replace id-as-filename. `MemoryManager` SHALL track each loaded memory's origin (Project or Global) and persist memories to the directory matching their scope.

#### Scenario: Project memory persisted to project-local directory

- **WHEN** `MemoryManager::add_memory(entry, Project)` is called with a valid MemoryEntry
- **THEN** the entry is saved as `<CWD>/.wgenty-code/memory/<id>.json`

#### Scenario: Global memory persisted to global directory

- **WHEN** `MemoryManager::add_memory(entry, Global)` is called with a valid MemoryEntry
- **THEN** the entry is saved as `~/.wgenty-code/memory/<id>.json`

#### Scenario: CWD unavailable degrades to global storage

- **WHEN** the project-local memory directory cannot be created (e.g. CWD deleted or unwritable)
- **THEN** project memories SHALL fall back to the global memory directory and a warning SHALL be logged

#### Scenario: CWD equals home directory

- **WHEN** `project_root` resolves to the user's home directory (project root coincides with global root)
- **THEN** project memories SHALL be written to the global memory directory (merged pool) and a warning SHALL be logged

#### Scenario: Legacy memory JSON loads with feedback-field defaults

- **WHEN** a memory JSON file written before this change (lacking feedback/staleness fields) is loaded
- **THEN** it deserializes successfully with `recall_count=0`, `hit_count=0`, `last_reinforced_at=None`, `superseded_by=None`, and `stale_marked_at=None`

### Requirement: Memory recall at session startup

At session startup, `MemoryManager::load()` SHALL load project memories from `<CWD>/.wgenty-code/memory/` and global memories from `~/.wgenty-code/memory/`. `MemoryManager::search_memories(query)` SHALL retrieve only project memories matching the query via the TF-IDF index (global memories are not indexed and are injected verbatim every turn). Recall ranking, threshold filtering, and global-memory soft-cap ordering SHALL use **effective importance** (see "Effective importance evaluation"). A superseded memory (`superseded_by` is Some) SHALL be excluded from recall. When project memories are successfully selected for injection into a `<memory-context>` block, each such memory's `recall_count` SHALL be incremented by one and persisted (so hit-rate damping observes real injection frequency). Global memories injected via `<global-memory>` are outside this counter. When `exploration_epsilon` is greater than zero, recall MAY with that probability replace the lowest-ranked injected project memory with a low-effective-importance project memory not recently recalled (see "Recall exploration injection"). When `exploration_epsilon` is 0 (the default), recall SHALL return the plain effective-importance ranking with no exploration replacement. CLI/`list_memories` style listing SHALL order (and apply minimum-score filters) by effective importance; superseded memories MAY still appear in listings with effective importance 0 for auditability, even though they are excluded from recall injection.

#### Scenario: Global memories injected every turn

- **WHEN** a turn is processed and global memories exist in `~/.wgenty-code/memory/`
- **THEN** a `<global-memory>` block containing all global memories (sorted by effective importance, capped at 50) is injected into the system prompt between the Environment and Skills layers

#### Scenario: Project memories recalled by keyword

- **WHEN** a user message is processed and project memories match the extracted keywords with effective importance >= threshold
- **THEN** a `<memory-context>` block containing the matched project memories is injected (global memories are excluded from this block)

#### Scenario: No global memories

- **WHEN** a turn is processed but no global memories exist
- **THEN** no `<global-memory>` block is injected

#### Scenario: Global memory soft cap exceeded

- **WHEN** more than 50 global memories exist
- **THEN** only the top 50 by effective importance are injected and a warning is logged

#### Scenario: Superseded memory excluded from recall

- **WHEN** a memory has `superseded_by = Some(other_id)` and would otherwise match the recall query
- **THEN** it is not included in the `<memory-context>` block (its effective importance is treated as 0)

#### Scenario: Injected project memories increment recall_count

- **WHEN** one or more project memories are injected into a `<memory-context>` block for a turn
- **THEN** each injected project memory has its `recall_count` increased by one and the updated entry is persisted

#### Scenario: List ordering uses effective importance

- **WHEN** memories are listed via `list_memories` (or equivalent CLI) with a minimum score filter
- **THEN** ordering and the minimum filter use effective importance, and a superseded memory may still appear with effective importance 0

### Requirement: Time-gated memory consolidation

`AutoDreamService::check_and_run()` SHALL be called at session startup before recall, in both TUI/daemon and headless modes. The gate thresholds SHALL be `min_hours = 1` and `min_sessions = 1`. The session-scan throttle SHALL be 10 minutes.

AutoDream SHALL NOT maintain its own disk-based consolidation lock. Cross-process mutual exclusion SHALL be provided solely by `MemoryManager::consolidate()`'s internal `ConsolidationFileLock` (at `~/.wgenty-code/memory/.consolidation.lock`). AutoDream's in-memory `is_consolidating` flag SHALL be reset on each `check_and_run` invocation.

```

Full source: openspec/changes/brain-inspired-memory-system/specs/agent-memory/spec.md

