## Context

wgenty-code 的 `agent-memory` 当前是"检索式"方案:扁平短事实(`MemoryEntry`)+ TF-IDF 关键词召回(`inject.rs`)+ 静态 importance + 本地 `consolidate()`(LLM-free,1h/1session 门限)。compaction 时 LLM 抽取记忆(编码),`dream` 做合并/去重/剪枝(巩固),TTL 决定遗忘。

这套方案的根因局限是**哲学层面的**:它是信息检索(相似度打分取 Top-K,确定性),而人脑记忆是**重构**(线索触发重新拼装,每次召回=微重写,带情绪/语境偏差)。两者差别不是"换 embedding 维度"能弥合的,要在机制层补。

本次 change 把记忆系统从"带 TTL 的检索数据库"演进为"脑式重构 + grounding 校验"。核心原则:**借大脑的架构灵感(情节/语义分层、巩固转移、再巩固、多维 salience、线索驱动召回),加上大脑没有的 grounding(对照代码库自纠错)**。仿脑给组织方式,grounding 给可靠性--这是 coding agent 记忆能比人脑更可靠的根因。

约束:不替换 TF-IDF 为 embedding(独立 track)、不改 compaction 抽取 prompt 引用已有记忆、`consolidate()` 本体须保持 LLM-free、向后兼容、三平台通用、情节文件名限 ASCII。

## 脑机制 -> Agent 映射

| 大脑机制 | 当前 agent | 本次 pillar |
|---------|-----------|------------|
| 工作记忆(有限 scratchpad) | context window | (已有) |
| 海马-皮层分工(情节 vs 语义) | **无情节层**(transcript 压缩即丢) | **P2** 情节/语义分层 |
| 巩固(睡眠 replay,海马->皮层转移) | `dream` 纯本地合并 | **P2** replay_extract(LLM,独立步骤) |
| 再巩固(召回=微重写,可更新) | 召回静态取出 | **P5** 重述 + 读时写回 |
| 强化(retrieval practice) | 无 | **P1** effective importance + engagement |
| 遗忘(主动抑制 + 衰减 + 干扰) | TTL 被动截断 | **P1** 指数衰减 + 命中率阻尼 + 探索 |
| 情绪 salience(杏仁核,多维) | 单维 LLM importance | **P4** pain_score |
| 线索驱动(多弱线索 AND 起爆) | TF-IDF 单相似度 | **P3** 符号感知多线索 |
| Schema(模式压缩) | 无(`extract_insights` 已删) | Deferred |
| **grounding(对照 ground truth 自纠错)** | **无** | **P1 代码库过时 + P5 读时校验** ⭐ agent 独有 |

⭐ grounding 是大脑做不到的:大脑重构会出错且无法自检,agent 能对照代码库验证(文件还在吗、符号签名对吗)。这是 agent 胜过大脑的根本点,贯穿 P1/P5。

## Goals / Non-Goals

**Goals:**
- 补齐五个结构性缺口(无情节层 / importance 静态 / 召回单线索 / salience 单维 / 召回静态取出)
- 所有 LLM 操作隔离到 `dream` 独立后置步骤,`consolidate()` 保持 LLM-free
- 记忆系统闭环:importance 动态自校准、情节可回放、召回线索驱动、salience 多维、召回可重构
- grounding 校验(代码库过时 + 读时校验)作为 agent 可靠性差异化
- 零迁移向后兼容

**Non-Goals:**
- 不替换 TF-IDF 为 embedding(独立 track;P3 用符号信号在无 embedding 下增强召回)
- 不做 agent 侧 engagement 归因(v2,噪声大)
- 不做 Schema 层次化约定压缩(强依赖 replay,作为后续延伸)
- 不做干扰覆盖(需 embedding 语义近邻;其衰减/突发重置/prune 部分已被 P1 覆盖)
- 不引入后台定时器做衰减(惰性读时计算)
- 不硬删被取代记忆(tombstone 保留可审计)

## Pillar 1 -- 动态 importance 与反馈回路(再巩固/强化)

### 机制
- `MemoryEntry` +4 字段:`recall_count`/`hit_count`/`last_reinforced_at`(Option,锚点)/`superseded_by`(Option,tombstone),`#[serde(default)]` 零迁移
- `effective_importance()` 纯函数(读时计算,不写盘):
  `effective = base * decay * (0.5 + 0.5*hitrate)`,`decay = exp(-ln2*hours/half_life)`(anchor=last_reinforced_at 或 timestamp,half_life 复用 `should_keep` 类型 TTL 倍率),`hitrate = (hit_count+1)/(recall_count+2)`(Laplace 平滑)。superseded -> 0。
- 正信号:`add_memory` 兼容扩展(非矛盾)强化旧记忆;词法 engagement 归因窗口
- 负信号:矛盾取代(Tier1 启发式 + tombstone);代码库实证过时(consolidate 时校验路径存在)
- 召回探索 ε

### 关键决策
- **D1 惰性衰减非定时器**:纯函数读时投影,无后台状态机,简化并发
- **D2 命中率阻尼转负反馈**:`recall_count` 不当正信号(那是 rich-get-richer 陷阱),而通过 hitrate 把"频召回零命中"转为衰减动力。Laplace 平滑保护低召回记忆
- **D6 LLM 隔离**:矛盾批分类在 dream 独立后置步骤,**不进 consolidate()**,保住 "Consolidation is LLM-free" invariant 与门限前提

### 落地集成点
- recall 排序(`inject.rs:41`)、`should_keep`(`consolidation.rs:135`)、`format_global`(`inject.rs:74`)改用 effective
- `add_memory`(`mod.rs:451`)去重分支:Compatible->merge+reinforce;Contradicts->tombstone;Ambiguous->merge+flag
- 归因窗口 `RecallAttribution` 挂 agent loop:settle(user msg)->recall->record(注入 ids)
- Tier2 矛盾分类:dream 后置 `resolve_ambiguous_pairs()`,批处理 flagged 对

## Pillar 2 -- 情节/语义分层 + 离线 replay 巩固(海马-皮层分工)

### 问题
当前**无情节层**。transcript 是事实上的情节存储,但 compaction 一压,情节细节(试了方案 A 失败才改 B)永久丢失,只剩 LLM 抽出的语义。无法回放"上次会话发生了什么"。

### 机制
- **情节层存储**:独立目录 `<project>/.wgenty-code/episodes/`,append-mostly(写一次,回放/剪枝,不合并/不被 superseded_by 引用/不进 TF-IDF 索引)
- **replay 巩固**:`dream` 新增独立后置步骤 `replay_extract()`(LLM,在 `consolidate()` 之后):批量读近期 episode -> 抽 fact 合并进语义、去重、矛盾标 superseded、prune 低频低痛情节

### 关键决策:情节文件名 = 日期-slug(讨论结论)
原始想法是"用情节描述当现有记忆文件名"。**经代码核查否决**:现有存储 `storage.rs:72` `save_memory` 用 `format!("{}.json", entry.id)` --**文件名就是 id,而 id 是承重的**(superseded_by/TF-IDF 索引/merge-keep-id/import-dedup 全引用它)。`load_all`(:142)虽从内容读 id 容忍文件名≠id,但写/删/单条加载三条路径都假设文件名==id。让文件名变语义 slug = 让 id 变 slug,破坏稳定性 invariant(slug 语义该变,id 必须稳定),且 slug 会撞、跨平台受限(`validate_id` 已强制)、中文文件名脆弱。

**安全落地**:情节层用**独立目录**(不碰语义存储的承重 id),文件名 `<YYYYMMDD-HHMM>-<ascii-slug>-<shortid>.json`:
- 日期前缀 -> `ls` 天然 chronological 索引(文件系统即索引,不另建 DB,符合"简单"直觉)
- ascii-slug -> 跨平台安全(从符号/关键词派生,非中文自由文本)
- shortid -> 保唯一性,不靠 slug 防撞
- id 在 JSON 内容(episode 是 append-mostly,文件名无需稳定,因不被别的逻辑按 id 引用)

这样"文件名=情节"的直觉原样落地,且零风险--因为情节层没有语义存储的 id 稳定性约束。

### 关键决策:replay 是 LLM 操作,隔离到独立步骤
replay 本质是 LLM(抽 fact/抽象/合并),与 `consolidate()` LLM-free invariant 冲突。解法**复用 P1 的 D6 模式**:`dream = consolidate()(本地) + replay_extract()(独立 LLM)`。所以 **P1 建立的 LLM 分离 invariant 是 P2 的地基**--这是两 pillar 合并的内在递进。

## Pillar 3 -- 符号感知多线索召回(线索驱动)

### 问题
召回只有 TF-IDF 关键词单信号,不利用 coding agent 独有富信号:当前任务符号上下文(open files / 编辑中的函数 / 栈帧)。通用 chat 拿不到,这里浪费。

### 机制
recall 打分扩展为多线索:`score = α·tfidf + β·symbol_overlap(当前任务符号上下文, 记忆内容) + γ·recency`。symbol_overlap 复用 CodeGraph/LSP 符号表或正则提取;当前任务符号上下文在 agent loop 现成。

### 关键决策:不上 embedding,用符号信号
symbol_overlap 是关键词之外唯一能廉价拿到的语义信号,**且是 coding agent 独有**。在无 embedding 时它是召回增强的最优解,符合"grounding > 仿脑"原则--用代码符号当线索,比仿海马靠谱。这比"加 embedding 维度"更对症。

## Pillar 4 -- pain_score 多维 salience(情绪权重)

### 问题
单一 LLM 主观 importance,无校准。大脑用情绪强度(新颖性/惊异/错误代价)给编码优先级。

### 机制
- 从现有摩擦信号近似 pain(v1 不需情节层):`exec_command` 失败重试次数、guardian 拒绝、用户纠正(同意图重述)、`undo` 调用--agent loop 可观测
- compaction 抽取时 LLM 把 pain 写入 importance/metadata;高 pain 记忆 consolidate 权重更高
- effective_importance 可显式加权 pain(标量->多维 salience 的第一步)

### 关键决策:pain 数字化近似,不仿化学信号
人脑化学信号(多巴胺/皮质醇)无法直接仿,pain_score 是数字化近似,够用。v1 从摩擦信号派生避免新存储依赖;P2 情节层落地后,pain 可记入 episode,consolidation 时按 pain 加权回放(高 pain 优先搬进语义)。

## Pillar 5 -- 召回时重构(重述 + 再巩固写回)

### 问题
召回物原样注入,不重述、不校验、不更新。大脑召回=微重写。

### 机制
- **重述**:召回物若是情节条目且较长,经一轮 LLM 重述切出当前 task 相关面再注入(语义短事实跳过)
- **读时再巩固**:召回时若发现记忆与当前代码不符(grounding 校验),触发软"verify me"提示或写回更新,延伸 P1 矛盾检测到读时

### 关键决策:重述被情节层 gate,且避开 hot-path 滥调
对"重述省 token(500t->80t)"的异议:当前记忆是几十字短事实,restate 省不了多少反而每轮多一次 LLM(hot path,延迟敏感)。**重述价值被 P2 情节层 gate**--只有情节带来 rich episode 时才划算,且仅对长情节触发,语义短事实跳过。读时校验是 grounding 的召回侧延伸,低风险高价值。

## 跨切决策(Cross-cutting)

- **CC1 consolidate() 永远 LLM-free**:所有 LLM 操作(replay 抽取 P2、矛盾批分类 P1)均在 `dream` 独立后置步骤。这是整个 change 的承重 invariant,保住现有 spec "Consolidation is LLM-free" 与 1h/1session 门限前提。P1 的 D6 是地基,P2 复用它
- **CC2 情节层独立存储**:独立目录 + date-slug 文件名 + append-mostly,不碰语义存储的承重 id(见 P2 文件名结论)
- **CC3 grounding 贯穿**:代码库过时校验(P1,consolidate 时)+ 读时校验(P5,recall 时)是 agent 独有的可靠性差异化,贯穿而非单点
- **CC4 不上 embedding**:P3 用符号信号在无 embedding 下增强召回;embedding 是独立 track。避免本 change 范围爆炸
- **CC5 零迁移**:语义记忆 serde default;情节层新增目录不影响现有

## Risks / Trade-offs

- **[Risk] change 范围大(5 pillar)** -> 按 pillar 分阶段实现(P1 先,是其他的地基);tasks.md 标注依赖;可分 PR 但同属一 change
- **[Risk] decay_tau/epsilon/pain 权重经验值不准** -> 配置可调,留实测;effective_importance 单测覆盖曲线
- **[Risk] Tier1 矛盾启发式误判** -> tombstone 不硬删可逆可审计;Tier2 LLM + 用户 prune 兜底
- **[Risk] 词法 engagement 假阳性** -> IDF distinctive 门槛 + topic 边界双重过滤;engagement 是中正信号,权重低于 memory_add 扩展
- **[Risk] P5 重述 hot-path LLM 成本** -> 仅对长情节触发,语义短事实跳过;被 P2 gate
- **[Risk] 情节层文件名 slug 信息损失** -> ascii-slug 限 ASCII(跨平台),中文情节描述进 JSON content 不进文件名;shortid 保唯一
- **[Risk] replay LLM 不稳定** -> 批处理降频;三分类(supersede/merge/both)简单;tombstone 可逆
- **[Trade-off] effective_importance 每次召回重算** -> 计算廉价,记忆数有 max_memories 上限,可接受
- **[Trade-off] rich-get-richer 靠 ε 探索缓解** -> 不彻底,彻底解需 embedding 召回(独立 track)

## Migration Plan

1. **语义记忆兼容**:`#[serde(default)]`,旧 JSON 零迁移(新字段默认:0/0/None->用 timestamp/None)
2. **锚定迁移**:首次 dream,对 `last_reinforced_at=None` 的记忆锚定为当前时间,避免老记忆瞬间衰减过猛(一次性,幂等)
3. **情节层**:新增目录,无迁移;首次启用时为空,随会话积累
4. **回滚**:新字段被旧代码忽略,行为退回静态 importance;情节目录独立,可整目录删除;无数据损坏
5. **配置**:新项有默认值,无需用户配置

## Open Questions

1. `decay_tau_turns`(2.0)、`exploration_epsilon`(0.15)、`supersede_penalty`(0.3)、`staleness_penalty`(0.5)、多线索权重 α/β/γ 的最优值需实测
2. replay_extract 的 LLM prompt 批格式(如何呈现 episode 批让 LLM 抽 fact/去重/判矛盾)在 build 阶段设计
3. 情节层的写入粒度:每轮写 vs 每决策点写 vs 会话结束写(初版倾向每决策点 + 会话结束汇总,平衡容量与完整性)
4. `MemoryIndex::distinctive_tokens` 与 symbol 提取是否需索引结构调整(预期仅加查询方法)
5. pain_score 的摩擦信号采集是否影响 agent loop 性能(预期轻量计数)
6. 归因窗口 `RecallAttribution` 持久化粒度:会话级(初版)vs 跨会话
