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
- **不变式**：`consolidate()` LLM-free 与 1h/1session 门限前提保持
- **产品行为**：矛盾记忆不再双双召回；长期未强化记忆排序下降；默认关闭探索以免干扰预期

## Success criteria（可验收）

1. Legacy memory JSON 无迁移加载成功
2. `effective_importance` 单测：衰减、命中率阻尼、never-recalled 中性、superseded→0、staleness multiplier
3. Contradicts 路径：旧条不进 recall，文件仍在磁盘
4. `consolidate()` 路径探测零 LLM；同一 stale 记忆多次 consolidate 不反复打穿 importance
5. `exploration_epsilon=0` 时行为与“仅 effective 排序”一致
6. `cargo test` / clippy -D warnings / fmt 通过；`WGENTY.md` 配置表已更新
