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
hitrate    = clamp((hit_count + 1) / (recall_count + 2), 0, 1)   # Laplace
decay      = exp(-ln2 * hours_since(anchor) / type_half_life)
stale_mul  = staleness_penalty if stale_marked else 1.0
effective  = 0  if superseded_by.is_some()
           else base_importance * decay * (0.5 + hitrate) * stale_mul
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
| Ambiguous | 其它 | merge + `metadata` flag / pending 列表 **仅记录**；不 LLM |

### 决策
- **D4 宁可 Ambiguous 不误 supersede**：标记词必须结合高相似；单测固定用例，不追求 NLP 完备
- **D5 本 change 无 Tier-2 LLM**：保住 dream/consolidate 范围；pending flag 模式稳定即可，解析器 follow-up 可消费同一 metadata
- **D6 tombstone 不硬删**：审计、回滚、用户 prune 另议

### `reinforce`
```text
hit_count += 1
last_reinforced_at = now
// 可选：轻微提升 base importance cap 到 1.0 —— v1 可不改 base，只靠 hitrate/anchor
```

注入召回时：`recall_count += 1` 并持久化（若性能敏感可会话聚合后写回；v1 简单按次写可接受，记忆量有上限）。

## M3 — Codebase staleness（幂等）

### 问题
若 `importance *= penalty` 每次 consolidate 执行，base 会被打穿且失去可解释性。

### 机制
1. 从 content 用保守 regex 提取类路径 token（如 `src/...rs`、带可选 `:line`）
2. 对 project memory：若**至少一个**被引用路径曾存在逻辑所需——v1 采用：提取到路径且**全部** missing 才标 stale（避免 URL/示例误伤；具体启发式单测钉死）
3. 标记：
   - `metadata["stale_paths"] = true` 或一等字段 `stale_marked_at: Option<DateTime>`（更清晰则一等字段 + serde default）
   - **若已标记则跳过**（幂等）
4. `effective_importance` 读标记施加 `staleness_penalty`（默认 0.5）
5. **不**刷新 `last_reinforced_at`；**不**在每次 consolidate 修改 base importance（推荐）

若希望用户在 JSON 里直接看见降权，允许**首次标记时**一次性改 base，并写 `stale_applied=true` 防二次乘——二选一，实现只保留一种并在单测锁死。**优选：不改 base，只改 effective 乘数。**

### 决策
- **D7 staleness 只在 consolidate，保持 LLM-free**
- **D8 配置门闸** `staleness_check` 默认 true

## M4 — Exploration（默认关）

```text
if epsilon > 0 && bernoulli(epsilon):
  replace lowest-ranked injected project memory
  with a low-effective, not-recently-recalled, not-superseded candidate
```

- 默认 `exploration_epsilon = 0`
- recently set：进程内 / 会话内即可（v1）
- 不引入新存储

## M5 — Config / migration / surface

### Config keys（均有默认）
| key | default | 含义 |
|-----|---------|------|
| `memory.exploration_epsilon` | `0.0` | 召回探索概率 |
| `memory.supersede_penalty` | `0.3` | tombstone 时对旧条 base 的一次性乘子（若采用改 base）；若仅 effective 路径可作展示降权 |
| `memory.staleness_check` | `true` | consolidate 路径检查 |
| `memory.staleness_penalty` | `0.5` | effective 乘数 |
| （复用）type TTL / importance_threshold | 现有 | half-life 与 should_keep |

不必引入 `decay_tau_turns`（属 engagement follow-up）。

### Migration
1. serde default 加载旧 JSON
2. 首次 consolidate：anchor `last_reinforced_at`
3. 回滚：旧二进制忽略新字段；删除 tombstone 字段即恢复（或用户工具后续做）

### 产品表面（最小）
- 不强制 TUI 大改；若 list API 原样吐 JSON，新字段自然可见
- CLI/面板若过滤召回，应尊重 superseded（与 inject 一致）

## 集成点（现码）

| 点 | 文件（约） | 改动 |
|----|-----------|------|
| 结构体 | `context/mod.rs` `MemoryEntry` | +4 字段、`new`/`reinforce`/`effective_importance` |
| 写入去重 | `MemoryManager::add_memory` | 关系分类分支 |
| 召回 | `context/inject.rs` | effective 排序/阈值；可选 exploration；recall_count |
| 保留 | `consolidation.rs` `should_keep` | effective；half-life 抽取共享 |
| 巩固 | `MemoryManager::consolidate` / engine | staleness 标记；anchor 迁移 |
| 配置 | settings / `ConsolidationConfig` / memory settings | 新 key |
| 文档 | `WGENTY.md` | 配置表 |

**不改**：compaction 抽取 prompt 结构（除文档说明外）、AutoDream 门限、存储目录布局、id 文件名。

## 跨切不变式

1. **`consolidate()` 永远 LLM-free`** — 本 change 无任何 dream LLM 步骤
2. **语义 id 文件名稳定** — superseded 只改内容字段
3. **默认保守** — epsilon=0；Ambiguous 不删条
4. **幂等 grounding** — stale 不叠乘
5. **零迁移** — serde default + 可选 anchor

## Risks / Trade-offs

| 风险 | 缓解 |
|------|------|
| Tier-1 误 supersede | 保守规则 + tombstone 可逆 + 单测金标句对 |
| 衰减过猛 | anchor 迁移；half-life 跟 type TTL；配置 |
| recall_count 写盘频繁 | 记忆上限小；必要时后续改批量 |
| stale regex 误伤 | 保守路径形态；全 missing 才标；可关配置 |
| 范围再膨胀 | tasks 按 M1–M5；deferred 另开 change |

## Follow-up roadmap（非本 change tasks）

1. **engagement attribution** — user-side distinctive IDF 窗口（依赖 recall_count/hit_count 字段）
2. **ambiguous LLM resolve** — dream 后置批分类（严格 after consolidate）
3. **symbol multi-cue** — 先 inventory agent-loop 符号信号，再改打分
4. **episodic store + replay** — 必须先写清：写入粒度、**读路径**（仅 cold dream vs 热召回）、与 TF-IDF 隔离、文件名 ASCII 方案（原 P2 结论可复用）
5. **pain_score** — 先 friction counter inventory，再进 metadata；最后才进 replay 权重
6. **cold-path restate / read-time verify 标记** — 禁止默认热路径 LLM；verify 以本地探针为先

### 原 P2/P5 自洽性结论（留给 episodic change）

> 若 episodic **不进** TF-IDF，则热路径无法“召回长情节再 restate”，除非另建 episode 候选通道。  
> 因此 **restate 不得作为无读路径的 MUST**；episodic v1 应先 **只写 + dream replay 进语义**，热路径只读语义。

## Open questions（build 前收口，允许实现时用默认）

1. stale 标记用一等字段还是 metadata —— 推荐一等 `stale_marked_at: Option<...>`
2. supersede 是否改旧条 base importance，或只靠 effective=0（tombstone 已是 0）—— 推荐 tombstone⇒effective 0，**不必再乘 penalty**；penalty 留给非 tombstone 的软降权场景。为减少概念，**Contradicts 只设 superseded_by，不再改 importance**
3. `recall_count` 是否本 change 就持久化 —— 推荐是，否则 hitrate 永中性；至少 inject 路径 +1
4. 探索候选池定义（最低 20% effective？未召回优先？）—— 实现选简单策略并单测

## 测试策略

- 单元：effective 曲线、legacy serde、classify_relation 金标、staleness 幂等、epsilon=0
- 集成：`add_memory` supersede 后 `search`/`recall` 不含旧条；consolidate 不调 LLM（现有模式）
- 不做：大型对话金标（留给 engagement/episodic）

## Migration Plan

1. 部署新二进制 → 旧 JSON 直接读  
2. 下一次 consolidate → anchor + 可选 stale 标记  
3. 回滚旧二进制 → 忽略新字段，行为回静态 importance（tombstone 字段残留无害）
