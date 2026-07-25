# Brainstorm Summary

- Change: brain-inspired-memory-system
- Date: 2026-07-25
- Status: **已确认定稿（方案 A）**

## 确认的技术方案

**主方案 A — 最小可靠闭环 + 一致表面**

落点不新开子系统：`context/mod.rs`、`inject.rs`、`consolidation.rs`、现有 `storage.memory` 配置。

### 数据模型
`MemoryEntry` +serde default：
- `recall_count` / `hit_count` / `last_reinforced_at` / `superseded_by` / `stale_marked_at`
- 文件名仍 = UUID id；不硬删；consolidate 零 LLM；旧 JSON 零迁移

### effective_importance（纯读时）
```
hitrate = (hit_count+1)/(recall_count+2)
decay = exp(-ln2 * hours_since(anchor) / type_half_life)
stale_mul = staleness_penalty if stale_marked_at.is_some() else 1.0
effective = 0 if superseded else base * decay * (0.5 + 0.5*hitrate) * stale_mul
```
half-life 复用 should_keep 的 type TTL 倍率 × `age_threshold_hours`。

### 写路径 add_memory（Jaccard ≥ 0.6）
- Compatible → merge + reinforce（hit++，anchor=now）；不抬 base
- Contradicts → 旧条 `superseded_by=new.id`；新条 standalone；不改旧 base；不删文件
- Ambiguous → merge + `metadata["relation_ambiguous"]=true`；无 LLM
- 工具：Contradicts 时 `merged=false`，`memory_id=新 id`

### 读路径
- recall：排除 superseded；effective 过滤/排序；注入 project 条 `recall_count+=1` 并 save
- format_global / list_memories：effective 排序（list min 过滤同）；superseded 可 list（effective=0）
- exploration_epsilon 默认 0

### consolidate（LLM-free）
- 锚定 `last_reinforced_at=None` → Some(now) 一次
- staleness：提取路径且**全部 missing** → 幂等 `stale_marked_at`；不改 base、不刷新 anchor
- should_keep 用 effective

### 配置默认
`exploration_epsilon=0.0`, `staleness_check=true`, `staleness_penalty=0.5`  
不引入 supersede_penalty。

## 关键取舍与风险

| 取舍 | 选择 | 风险/缓解 |
|------|------|-----------|
| recall_count 热路径写 | inject 持久化 | ≤top_n save/轮；锁序同 add_memory |
| stale 启发式 | 全 missing | 漏标优于误伤；可关 staleness_check |
| 矛盾默认 | 宁可 Ambiguous | tombstone 可逆；无 Tier-2 |
| CLI | effective 一致 | superseded 仍列出便审计 |
| 范围 | 无 episodic/LLM/engagement | follow-up change |

## 测试策略

- Unit：serde defaults；decay/hitrate/neutral/superseded/stale_mul；classify 金标；stale 幂等；ε=0
- Integration：supersede 后 recall 无旧条；inject 后 count 落盘；list 序与 effective 一致
- 结构：consolidate 无 LLM client 调用

## Spec Patch

回写 `specs/agent-memory/spec.md`：
1. inject 成功注入 project 记忆后递增并持久化 `recall_count`
2. stale = 全部 extracted 路径 missing；幂等 `stale_marked_at`
3. `list_memories`/等价 CLI 排序与 min 过滤用 effective；superseded 可被 list（effective=0）
4. 展示/文档层不强制 supersede_penalty（tombstone⇒effective 0）
