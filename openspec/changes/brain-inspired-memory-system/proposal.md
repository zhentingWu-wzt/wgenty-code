## Why

wgenty-code 的 `agent-memory` 能力当前是"检索式"的:扁平短事实 + TF-IDF 关键词召回 + 静态 importance + 本地 consolidate。这套方案本质是信息检索思路,而人脑记忆不是检索,是**重构**。两者哲学差别导致当前系统存在结构性的五个缺口:

1. **无情节层**:对话 transcript 是事实上的情节存储,但 compaction 一压就只剩 LLM 抽出的语义,情节细节(上次试了方案 A 失败、才改 B)**永久丢失**。没有独立、持久、可回放的情节层。
2. **importance 写一次定终身**:LLM 压缩时打分,之后只靠静态 TTL。记忆系统开环--rich-get-richer、有用记忆不强化、陈旧事实不失效。
3. **召回单线索**:只有 TF-IDF 关键词,不利用 coding agent 独有的富信号--当前任务符号上下文(open files / 编辑中的函数 / 栈帧)。通用 chat 拿不到的信号,这里白白浪费。
4. **salience 单维标量**:单一 LLM 主观 importance,无校准。大脑用情绪强度(新颖性/惊异/错误代价)给编码优先级,agent 完全没有等价物。
5. **召回即静态取出**:召回物原样注入,不重述、不校验、召回时不更新。大脑召回=微重写(再巩固),agent 是死检索。

本次 change 把这五个缺口一次性补齐,把记忆系统从"带 TTL 的检索数据库"演进为"脑式重构 + grounding 校验"--借大脑的架构(情节/语义分层、巩固转移、再巩固、多维 salience、线索驱动召回),加上大脑没有的 grounding(对照代码库自纠错)。

## What Changes

本次 change 由 5 个 pillar 构成:

**Pillar 1 -- 动态 importance 与反馈回路(再巩固/强化)**
- `MemoryEntry` +4 字段(`recall_count`/`hit_count`/`last_reinforced_at`/`superseded_by`),serde default 零迁移
- `effective_importance()` 惰性纯函数(base × 时间衰减 × 命中率阻尼),recall/consolidation/global 排序改用 effective
- 正信号:`add_memory` 兼容扩展强化;词法 engagement 归因窗口(recency 衰减 + IDF distinctive 门槛 + topic 边界)
- 负信号:矛盾取代(Tier1 启发式 + dream 独立后置 LLM 批分类,tombstone 不硬删);代码库实证过时校验
- 召回探索 ε 打破 rich-get-richer

**Pillar 2 -- 情节/语义分层 + 离线 replay 巩固(海马-皮层分工)**
- 新增**情节层**:独立目录 `<project>/.wgenty-code/episodes/`,文件名 `<YYYYMMDD-HHMM>-<ascii-slug>-<shortid>.json`(日期前缀免费 chronological 索引,ascii-slug 跨平台安全,shortid 保唯一),id 在 JSON 内容;append-mostly(写一次,回放/剪枝,不合并)
- `dream` 新增独立后置步骤 `replay_extract()`:批量读近期 episode -> 抽 fact 合并进语义、去重、矛盾标 superseded、prune 低频低痛;**`consolidate()` 本体保持 LLM-free 不变**(复用 Pillar 1 建立的 LLM 分离 invariant)
- 情节层记录 pain_score(Pillar 4)与决策/文件/bug/用户诉求

**Pillar 3 -- 符号感知多线索召回(线索驱动,非相似度驱动)**
- recall 打分从单 TF-IDF 扩展为多线索:`score = α·tfidf + β·symbol_overlap(当前任务符号上下文, 记忆内容) + γ·recency`
- symbol_overlap 复用 CodeGraph/LSP 符号表或正则提取;当前任务符号上下文(open files / 编辑函数 / 栈帧)在 agent loop 现成
- 不引入 embedding--symbol_overlap 是 coding agent 独有的廉价语义信号,在无 embedding 时是关键词之外最强的召回增强

**Pillar 4 -- pain_score 多维 salience(情绪权重)**
- 从现有摩擦信号近似 pain:`exec_command` 失败重试次数、guardian 拒绝、用户纠正(同意图重述)、`undo` 调用--agent loop 可观测,无需新存储
- compaction 抽取时 LLM 把 pain 写入 memory 的 importance/metadata;高 pain 的记忆 consolidate 权重更高
- effective_importance 可显式加权 pain(从标量 importance 迈向多维 salience 的第一步)

**Pillar 5 -- 召回时重构(重述 + 再巩固写回)**
- 召回物若是**情节条目且较长**,经一轮 LLM 重述切出当前 task 相关面再注入(语义短事实跳过,避免 hot-path 无谓 LLM 调用);被 Pillar 2 的情节层 gate
- 召回时若发现记忆与当前代码不符(grounding 校验),触发软"verify me"提示或写回更新(读时再巩固),延伸 Pillar 1 的矛盾检测到读时

**Deferred(本次不做,文档记录方向)**
- Schema 层次化约定压缩:强依赖 replay,作为 replay 之后的自然延伸
- 干扰覆盖(interference overwrite):需 embedding 语义近邻,deferred 到 embedding 时代;其指数衰减+突发重置+prune 部分已被 Pillar 1 覆盖

## Capabilities

### New Capabilities
(无新 capability;全部增强已有 `agent-memory`)

### Modified Capabilities
- `agent-memory`:
  - **数据模型**:MemoryEntry +4 反馈字段;新增情节存储(独立目录 + slug 文件名 + Episodic 类型)
  - **recall**:effective importance 排序 + 符号感知多线索打分 + ε 探索 + 情节重述 + 读时校验
  - **consolidation**:should_keep 用 effective;新增代码库过时校验;新增独立后置 LLM 步骤(replay 抽取 + 矛盾批分类),`consolidate()` 本体 LLM-free 不变
  - **add_memory**:矛盾分类(Compatible/Contradicts/Ambiguous)+ 强化 + tombstone
  - **salience**:pain_score 从摩擦信号注入 importance
  - **engagement 归因窗口**:recency + IDF + topic 边界(v1 user 侧)

## Impact

- **代码**:`context/mod.rs`(MemoryEntry/effective_importance/add_memory/reinforce/pain)、`context/inject.rs`(recall 多线索/归因/探索/重述)、`context/consolidation.rs`(should_keep/classify_relation/staleness/replay/矛盾批分类)、`context/episodes.rs`(新,情节存储)、`MemoryIndex`(distinctive_tokens/symbol 提取)、agent loop(RecallAttribution + 符号上下文采集 + 摩擦采集)、config
- **数据兼容**:语义记忆 JSON 零迁移(serde default);情节层为新增目录,不影响现有;建议首次 dream 锚定旧记忆 last_reinforced_at
- **性能**:recall 每轮多一次 settle + 符号打分(轻量);重述仅对长情节触发;replay/矛盾 LLM 仅在有待处理项时触发,按批
- **现有 spec 关系**:与"Consolidation is LLM-free"**不冲突**--所有 LLM 操作(replay 抽取、矛盾批分类)均在独立后置步骤,`consolidate()` 本体与 1h/1session 门限前提保留
- **安全**:无新增 guardian/sandbox 敏感面;LLM 调用复用现有 API 客户端
- **平台**:情节文件名限 ASCII(跨平台安全),三平台通用;符号提取复用现有 codegraph/lsp
- **设计哲学**:从"检索"转向"重构 + grounding"--仿脑给架构灵感,grounding(对照代码库自纠错)给可靠性,这是 coding agent 记忆能比人脑更可靠的根因
