
# 长上下文与长任务可靠性 —— 深度分析

> 分析时间：2026-07-26 | 项目：wgenty-code v0.1.0

---

## 一、现状盘点：当前系统已有的"可靠性拼图"

### 1.1 硬限制

| 机制 | 位置 | 默认值 | 效果 |
|------|------|--------|------|
| `max_rounds` | `src/agent/runtime/config.rs:25` | 100 | 主 Agent 和子 Agent 到达后直接 abort (`MaxRoundsExceeded`) |
| `max_subagent_depth` | `settings.agent.subagent.max_depth` | 1 | 子 Agent 不能再 spawn 孙子 Agent |
| `max_concurrent_subagents` | `settings.agent.subagent.max_concurrent` | 5 | 同一时刻最多 5 个并行子任务 |
| `subagent.timeout_secs` | `settings.agent.subagent.timeout_secs` | 1800s | 单个子 Agent 的超时 |
| `max_replan_cycles` | `settings.agent.rlm.max_replan_cycles` | 2 | RLM 失败后的最大重规划次数 |

配置路径：
- 主 Agent: `settings.agent.max_rounds` → `unwrap_or(100)`
- Subagent: `settings.agent.subagent.max_rounds` → `unwrap_or(100)`
- RLM 嵌套: 硬编码 `Some(100)` (`src/tools/meta/rlm/pipeline.rs:58`)

**已知瑕疵**：`src/config/mod.rs:198` 注释说 `Some(0)` means "unlimited"，但实际 `None → unwrap_or(100)` = 100，并非真正无限。

### 1.2 软检测

| 机制 | 实现位置 | 行为 |
|------|---------|------|
| **StuckDetector** | `src/utils/stuck_detector.rs` | 检测连续完全相同的 tool call（通过 `args_signature` 对参数 key-value 排序做字符串比较）。阈值：0-7 次正常，8-9 次 warning，10+ 次 force abort |
| **Auto Compaction** | `src/agent/runtime/loop_.rs:263-270` | 每轮结束后自动检查 token 估算是否超过 context_window 的 80%（通过 `needs_compaction()`），超过则自动压缩：前半部分替换为摘要，后半部分保留。支持手动触发（`compact_requested` 标志）和自动触发双路径 |
| **Micro Compaction** | `src/agent/runtime/compaction.rs` | 把压缩边界之前的旧 tool result 替换为 `[tool result truncated by compaction]` 标记，进一步缩减 token 占用 |
| **Calibration** | `src/agent/runtime/compaction.rs` | 用真实的 `usage.prompt_tokens` 校准 token 估算，比 chars/4 更精确 |
| **RLM Replan** | `src/tools/meta/rlm/pipeline.rs:472-650` | 子任务失败后最多 2 次增量重规划。把失败的任务发给 Planner 重新分解，Jaccard 去重后重新执行 |
| **80% warning** | `src/agent/runtime/loop_.rs` | 到达 max_rounds 的 80% 时打 warning 日志（`warn_rounds = max_rounds * 8 / 10`） |
| **Stepped timeout** | RLM pipeline | `PlanTimeout`, `ExecTimeout1`, `ExecTimeout2` 多级超时管理 |

### 1.3 状态持久化

| 机制 | 实现 |
|------|------|
| **Checkpoint** | Per-turn 文件快照到 `.wgenty-code/checkpoints/<turn-id>/`，只覆盖 agent 编辑的文件。`undo` 仅还原被编辑的文件，不触及 git 状态 |
| **Memory** | 双源（project+global），compact 时 LLM 抽取语义记忆，TF-IDF 召回，按 importance 分级 |
| **Subagent trace** | JSONL 记录完整的子 Agent 执行过程，支持 call_tree / html / chrome_trace 渲染 |
| **ConsolidationEngine** | 合并相似记忆、裁剪过期项、基于 TF-IDF 计算相似度 |

---

## 二、核心问题诊断

### 2.1 现有 compaction 机制有效但不够细粒度

系统**已有自动 compaction**（`loop_.rs:263-270`）：每轮结束后通过 `needs_compaction()` 检查校准后的 token 估算，超过 `context_window` 的 80% 时自动触发压缩。压缩流程：
- 前半部分消息 → LLM 生成摘要
- 后半部分消息 → 完整保留
- `compaction_boundary` 推进，后续 API 请求只发送 boundary 之后的内容
- `micro_compact_messages` 把旧 tool result 替换为 `[tool result truncated by compaction]` 标记

同时还支持模型通过 `compact_requested` 标志主动触发，以及 `compaction_failed` 保护防止死循环重试。

**但现有机制有三个关键缺口**：

**缺口 1：摘要非结构化**。当前 compaction 产生的是**自由文本摘要**，模型可以自由描述，但也容易遗漏关键信息。没有结构化的字段约束（哪个文件被改了、改了什么、测试结果、子任务状态等），导致摘要质量方差大。

**缺口 2：压缩后不可检索**。compaction 是"压缩后丢弃"模型——老消息被摘要替代后就没了。后续轮次中如果模型需要回顾第 30 轮读过的一个具体文件内容，只能依赖摘要里是否提到了它。缺少**按需检索的"冷层"**——按文件路径或主题从磁盘取回历史上下文。

**缺口 3：触发条件单一**。当前仅按 token 阈值触发，不与任务状态联动。更理想的情况是：连续 N 轮无进度 → 触发整理；接近 max_rounds → 主动压缩释放空间；模型行为退化 → 注入 refocus 提示。

### 2.2 StuckDetector 太粗粒度

当前只检测**完全相同的重复**：

```
tool_call_1: grep("foo")  → 结果A
tool_call_2: grep("foo")  → 结果A  ← 检测到
tool_call_3: grep("foo")  → 结果A  ← 警告
```

但真实场景中更常见的是**语义循环**：

```
tool_call_1: grep("error handling")  → 零结果
tool_call_2: grep("Error")           → 找到了，读了文件
tool_call_3: grep("error handling")  → 忘了已经搜过，又搜一遍
```

这种循环的 tool call 不完全相同（参数变了），字符串比较检测不到——**隐性卡死**。

### 2.3 子 Agent 失败后缺少"状态继承"

子 Agent 因超时/max_rounds/stuck 失败时：

- 中间状态全部丢失：已读文件、已执行命令、中间结论
- Replan 只基于最终结果判断：只知道 "[ERROR] task failed"
- 替换任务从零开始：新子 Agent 重新读文件、重新执行命令，浪费 token 和时间

### 2.4 Memory 系统与会话上下文是两个割裂的世界

当前记忆系统定位是**跨会话长期记忆**，不是**当前会话上下文管理**。compact 过程会同时做两件事——自动压缩触发时的对话历史摘要和 LLM 语义记忆抽取——但二者的联动是单向的：compaction → memory 写入，没有 memory → compaction 的反向指导。缺失的能力是：让持久化的项目知识（文件结构、之前修过的 bug、惯用模式）在压缩时**主动注入摘要**，而非仅作为独立 `<relevant_memories>` 层。

理想模型：
```
┌──────────────────────────────────┐
│     当前会话上下文窗口              │
│  ┌──────┐ ┌──────┐ ┌──────┐      │
│  │热层  │ │温层  │ │冷层  │      │
│  │N轮全量│ │压缩摘要│ │按需检索│    │
│  └──────┘ └──────┘ └──────┘      │
└──────────┬───────────────────────┘
           │ 语义记忆写入
┌──────────▼───────────────────────┐
│     持久化记忆存储                  │
│  project / global                │
└──────────────────────────────────┘
```

### 2.5 RLM 重规划的质量瓶颈

1. **Jaccard 去重太弱**：基于词袋的相似度（阈值 0.8）只能过滤几乎一字不差的重复提示词
2. **缺乏根因分析**：Planner 只知道 "task_i failed"，不知道失败原因（超时？stuck？权限被拒？）
3. **无 learning**：同一失败模式在同一个会话中重复出现但系统不记忆

---

## 三、解决方案：三层长任务可靠性体系

```
Layer 1: 检测层 —— 在问题发生前预警
Layer 2: 恢复层 —— 在问题发生时优雅降级
Layer 3: 持久层 —— 跨中断恢复和跨任务学习
```

---

### Layer 1: 检测层

#### 3.1.1 语义 StuckDetector

升级当前字符串比较为语义嵌入比较：

```rust
struct SemanticStuckDetector {
    recent_embeddings: VecDeque<(f32, Vec<f32>)>,   // (time, embedding)
    semantic_threshold: f32,                         // cosine similarity 阈值
    pattern_memory: HashMap<String, usize>,          // 失败模式计数
}
```

- 不比较 tool call 是否相同，而是比较 embedding 的余弦相似度
- 用 `fastembed` 或 API embedding 端点（轻量，不需要本地模型）
- 检测模式：`cos_sim(tool_call_i, tool_call_j) > 0.9` 且中间操作未改变文件系统状态 → 语义循环
- 检测到后：注入 `<stuck-warning>` 提示而非直接 abort

#### 3.1.2 进度指标追踪

```rust
struct ProgressTracker {
    files_read: HashSet<PathBuf>,
    files_modified: HashSet<PathBuf>,
    commands_succeeded: usize,
    commands_failed: usize,
    unique_tools_used: HashSet<String>,
    subagent_results: usize,
}
```

两个关键信号：
- **进度停滞**：连续 N 轮 `files_modified` 和 `commands_succeeded` 不增长
- **过度探索**：`files_read` 增长但 `files_modified` 停滞（可能在迷路）

#### 3.1.3 Token 密度信号

追踪有效信息密度：当 `useful_chars / total_chars` 持续下降 → 模型在读取大量无用信息 → 可能需要缩小搜索范围。

---

### Layer 2: 恢复层

#### 3.2.1 会话级分层上下文（热/温/冷三级）

**基础**：当前系统已有自动 compaction（token 达 80% 触发）和 micro-compaction（tool result 截断）。以下方案是在此基础上的**升级**——把自由文本摘要替换为结构化摘要，并新增冷层按需检索能力。

| 层级 | 范围 | 内容 | 注入方式 |
|------|------|------|---------|
| **热层** | 最近 15 轮 | 完整消息保留 | 常规 history |
| **温层** | 15-50 轮 | 结构化压缩摘要（文件操作序列、关键决策、未解决问题） | `<context-summary>` 系统消息 |
| **冷层** | 50+ 轮 | 持久化 + 嵌入索引 | 按文件路径/主题自动检索注入 |

**温层摘要格式**：
```json
{
  "rounds_summarized": "16-45",
  "files_touched": {
    "src/foo.rs": {
      "actions": ["read", "edited: error handling", "test failed"],
      "current_state": "compilation error: missing import",
      "decisions": ["chose anyhow::Context over custom Error type"]
    }
  },
  "subagent_outcomes": {
    "task_1": "completed: found 3 callers of deprecated fn",
    "task_2": "failed: timeout"
  },
  "open_issues": ["test_bar still failing", "need to update docs"],
  "task_progress": "60% complete: 3/5 modules refactored"
}
```

**压缩触发时机**：Agent Loop 每轮结束后检查热层 token 数，超过阈值自动执行温层压缩。

**压缩模型选择**：
- 方案A：调用模型的小模型 completion endpoint
- 方案B：本地 fastembed + 抽取式摘要 + LLM 仅在合并时介入

#### 3.2.2 Subagent 状态快照（Checkpoint & Resume）

```rust
struct SubagentSnapshot {
    completed_steps: Vec<StepSummary>,
    current_file_state: HashMap<PathBuf, Option<String>>,
    last_error: Option<String>,
    files_modified: HashSet<PathBuf>,
    rounds_used: usize,
}

enum StepOutcome {
    Success { key_finding: String },
    Failed { reason: String },
    Incomplete,
}
```

子 Agent 超时/stuck/max_rounds 耗尽时，replan 的新子 Agent 收到此快照作为 prompt 一部分。

#### 3.2.3 渐进式降级

| 检测信号 | 降级策略 |
|---------|---------|
| 接近 max_rounds 80% | 自动触发温层压缩，释放上下文空间 |
| 子 Agent 接近 timeout | 保存快照，返回部分结果 + `status: "partial"` |
| 模型行为退化（重复/迷路） | 注入 `<refocus>` 提示 |
| 主 Agent 接近 max_rounds 90% | 未完成任务自动委托给新子 Agent，附带完整温层摘要 |

---

### Layer 3: 持久层

#### 3.3.1 会话恢复（Session Resume）

```rust
struct SessionCheckpoint {
    hot_context: Vec<Message>,
    warm_summary: WarmSummary,
    cold_index: ColdIndex,
    progress: ProgressTracker,
    pending_subagents: Vec<PendingSubagent>,
    working_set: HashSet<PathBuf>,
}
```

用户中断、会话超时或进程崩溃后重启时可恢复热/温/冷层状态。

#### 3.3.2 跨任务模式学习

新增 `ExecutionPattern` 记忆类型：
- "在模块 src/auth/ 中 grep 时，结果通常很多，建议加文件类型过滤"
- "cargo test 在 CI 中超时后，先单独跑失败的测试更快"
- "编辑 src/config/ 下的文件后，需要同时更新 settings.json schema"

Session 结束时由 LLM 从 trace 中抽取，写入 project memory。下一次 TF-IDF 召回时自动注入。

---

## 四、实现路线图

### 阶段 1：最小可行改进（1-2 周）

1. **Semantic StuckDetector**：替换当前字符串比较，用 embedding API
2. **ProgressTracker**：Agent Loop 中增加进度信号追踪
3. **自动 refocus 注入**：接近 80% 轮次时注入 refocus/reframe 提示
4. **Subagent 失败上下文保留**：失败时保留最后 N 轮消息传给 replan

### 阶段 2：分层上下文（3-4 周）

5. **结构化温层压缩升级**：在现有 auto compaction 基础上，用小模型驱动生成结构化 JSON 摘要（替代自由文本），含文件操作序列、决策记录、子任务状态
6. **冷层磁盘存储 + embedding 索引**：`fastembed` 或 `text-embedding-3-small`
7. **冷层自动注入**：工具调用时按文件路径检索并追加历史上下文

### 阶段 3：断点续传（2-3 周）

8. **SubagentSnapshot**：子 Agent 状态保存和恢复
9. **SessionCheckpoint**：完整会话保存和恢复
10. **渐进式降级**：rounds/timeout 耗尽时的优雅退出

### 阶段 4：学习与优化（持续）

11. **ExecutionPattern 记忆**：跨任务经验积累
12. **自适应 max_rounds**：根据任务复杂度动态调整轮次限制
13. **分层上下文 A/B 测试**：对比有/无温冷层的任务完成率

---

## 五、待讨论的设计决策

1. **温层压缩的模型选择**：小模型 API vs 本地 fastembed 抽取式
2. **冷层嵌入基础设施**：与 memory 系统共用还是独立引入
3. **分层上下文的注入方式**：system message vs user message prepend
4. **优先级选择**：单次长任务（分层上下文优先） vs 多子任务编排（SubagentSnapshot + Replan 增强优先）

---

## 六、相关文件索引

| 文件 | 内容 |
|------|------|
| `src/agent/runtime/loop_.rs` | Agent 主循环，max_rounds 检查，stuck detection |
| `src/agent/runtime/config.rs` | RuntimeConfig，max_rounds 默认值 |
| `src/utils/stuck_detector.rs` | 当前 StuckDetector（`args_signature` 字符串比较） |
| `src/context/consolidation.rs` | ConsolidationEngine，记忆合并 |
| `src/context/memory.rs` | MemoryManager，双源存储 |
| `src/tools/meta/rlm/pipeline.rs` | RLM 管线，replan 逻辑 |
| `src/teams/subagent_loop.rs` | 子 Agent 循环 |
| `src/config/agent.rs` | SubagentConfig / AgentConfig |
| `src/tools/meta/task.rs` | task 工具，子 Agent 调度 |
| `src/config/mod.rs:188-199` | subagent override 中 max_rounds 映射 |
