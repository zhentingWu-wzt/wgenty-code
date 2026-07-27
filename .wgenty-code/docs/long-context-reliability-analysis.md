# 长上下文与长任务可靠性 -- 基于代码现状的重新分析

> 分析时间：2026-07-27 | 项目：wgenty-code v0.1.0
>
> 本文是对前版分析的重写。前版最大的问题是"现状盘点"存在多处事实性错误，导致核心论断与代码实际相反、方案与已实现能力重叠。本次重写以逐文件源码核实为准，每个结论附 `file:line` 引用，并明确区分**已实现能力**、**真实缺口**与**前版误判**。

---

## 一、现状盘点：系统已有的可靠性拼图

### 1.1 硬限制与轮次治理

| 机制 | 位置 | 默认值 | 效果 |
|------|------|--------|------|
| `max_rounds` | 主 Agent `RuntimeConfig` | 100 | 到达后终止循环 |
| 子 Agent `max_rounds` | `settings.agent.subagent.max_rounds` | override | `Some(0)` 被映射为 `None`（真正 unlimited） |
| `max_subagent_depth` | `settings.agent.subagent.max_depth` | 1 | 子 Agent 不能再 spawn 孙子 Agent |
| `max_concurrent_subagents` | `settings.agent.subagent.max_concurrent` | 5 | 同一时刻最多 5 个并行子任务 |
| `subagent.timeout_secs` | `settings.agent.subagent.timeout_secs` | 1800s | 单个子 Agent 超时 |
| `max_replan_cycles` | `settings.agent.rlm.max_replan_cycles` | 2 | RLM 失败后最大重规划次数 |
| `jaccard_threshold` | `settings.agent.rlm.jaccard_threshold` | 0.8 | replan 替换提示词去重阈值 |

**`Some(0)` = unlimited 的正确性**（纠正前版"已知瑕疵"）：`resolve_subagent_config` 明确 `s.agent.max_rounds = if r == 0 { None } else { Some(r) }`（`src/config/mod.rs:198-199`），并有测试 `test_resolve_subagent_max_rounds_zero_means_unlimited` 锁定（`src/config/tests.rs:195-200`）。注释（`src/config/mod.rs:188`）准确，前版声称的"瑕疵"不存在。

**80% 轮次预警**：主循环在到达 `max_rounds` 约 80% 时打 warning 日志，但仅日志告警，不触发更主动行为（无自动 refocus、无自动委托）。这是真实缺口（见 §2.2）。

### 1.2 上下文压缩续传（核心机制，前版完全遗漏）

这是前版最严重的错误：前版断言"上下文线性累积、不做中间压缩""compact 只做记忆抽取、不是历史压缩续传""需模型自己决定调用"。三点全部与实现相反。

**（1）compact 同时做历史压缩 + 记忆抽取**：`COMPACTION_SYSTEM_PROMPT` 任务 1 就是 "Summarize the conversation history, preserving key details: project context, files modified, decisions made, bugs found, commands executed, and any pending tasks"，任务 2 才是记忆抽取，输出 `{summary, memories}` JSON（`src/agent/runtime/compactor.rs:22-54`）。

**（2）自动触发，非模型主动**：每轮检查 `want_compact = state.compact_requested || needs_compaction(...)`（`src/agent/runtime/loop_.rs:263-270`）。`needs_compaction` 阈值公式为 `context_window * 4/5 − max_tokens`，且**计入固定 tool-definition overhead** 和 **calibration 校准**（`src/agent/runtime/compaction.rs:376-394` 有测试锁定，注释明确 `context_window=1000, max_tokens=100 -> threshold = 700`）。

**（3）压缩续传：前缀被 summary 替换**：压缩后 `assemble_post_compaction_history(system_messages, &summary, &tail_msgs)` 重组发往 API 的视图，`state.compaction_boundary` 切分"已压缩前缀"与"保留尾部"，summarized prefix 被 summary 替代（`src/agent/runtime/loop_.rs:277-295`）。完整 history 仍保留用于 session 保存。

**（4）热/温/冷分层已事实存在**：

| 层级 | 对应实现 | 内容 |
|------|---------|------|
| 热层 | `compaction_boundary` 之后的 tail | 最近若干轮完整消息 |
| 温层 | `compacted_summary` | LLM 生成的结构化摘要 |
| 冷层 | 双源 memory + TF-IDF 召回 | 持久化、按相关性检索注入 |

**（5）配套增强机制**：
- `micro_compact_messages`：微观压缩（长 tool result 截断等），`fallback_micro_compact` 在 413/body-too-large 错误时降级（`src/agent/runtime/compactor.rs:146-162`）。
- **calibration 校准**：用上次 API 返回的真实 `prompt_tokens` 校准 char→token 比率，防 CJK 等场景下过早/过晚压缩（`src/agent/runtime/compaction.rs:397-430` 测试注释详述动机）。
- **防无限压缩循环**：压缩后仍超阈值时置 `state.compaction_failed`，停止重试避免死循环（`src/agent/runtime/loop_.rs:302-316`）。

> 结论：前版 §3.2.1 把"温层压缩引擎（热/温/冷三级，15 轮界压缩）"当作**待建核心方案**，实际上系统已实现等价机制。真正值得讨论的是压缩的**质量与边界策略**（见 §2.8），而非"系统没有压缩"。

### 1.3 卡死检测

`StuckDetector` 基于 `args_signature`（参数按 key 排序后 `key=value` 拼接）的**精确字符串相等比较**（非哈希），`signatures == self.prev_signatures`（`src/utils/stuck_detector.rs:31-45`）。阈值：

- `0..=7` => `Ok`（前 7 次重复视为合法多步操作）
- `8..=9` => `Warn`（注入 tool result 提示换方法）
- `10+` => `Abort`（终止循环）

> 纠正前版：前版称"3 次警告、5 次 abort"，实际为 8 次警告、10 次 abort，阈值差一倍多。前版"基于哈希比较"措辞也不准，实为精确字符串比较。

### 1.4 RLM 重规划（前版严重低估）

前版称"Planner 只知道 task failed，不知道失败原因""replan 从零开始"。实际远比此丰富：

**（1）失败原因 + 已完成成果都传给 Planner**：`compute_replan_scope` 返回 `failure_reasons: HashMap<usize, String>`，含每个失败任务的错误消息 + 下游传播通知（`"Upstream dependencies failed: [...]"`），`pipeline.rs:530-536` 将其连同 `partial_results`（已完成任务的成果 snippet）传给 `Planner::replan_incremental`。Planner prompt 明确包含 `failures_json`（`{id, original_prompt, failure_reason}`）和 `partial_json`，并指示 "Use the failure reasons to produce a DIFFERENT, more viable decomposition... Do not repeat the failed approach"（`src/tools/meta/rlm/planner.rs:205-264`）。

**（2）传递性下游依赖重做**：`compute_replan_scope` 用 BFS 遍历 `deps`，失败任务的**所有传递性下游依赖**也被纳入 replan 范围（`src/tools/meta/rlm/pipeline.rs:936-945`）。前版完全未提及此机制。

**（3）executor 预算约束**：replanner 调用与替换执行从 executor pool 扣预算，预算不足时跳过 replan（`src/tools/meta/rlm/pipeline.rs:481-504, 551-554`）。

**（4）Jaccard 去重**：对比 replacement prompt 与 failed original prompt 的相似度，超阈值则丢弃（避免重复同样失败方法），并对同一 cycle 内已接受的 replacement 去重（`src/tools/meta/rlm/pipeline.rs:975-1008`）。实现为 token-level 词袋（`src/tools/meta/rlm/formats.rs:181-200`）。

**（5）结构化失败根因**：`FailureRootCause` 枚举（`TokenBudgetExceeded / GuardianRejected / SandboxFailed / ApiError / ToolPanic / Timeout / UserCancelled / Unknown`，`src/teams/failure_diagnostics.rs:6-18`），配合 `FailedRoundContext`（assistant_text + final_tool_output）、`ToolCallStep`（脱敏后的工具调用步骤）记录失败轮次上下文，用于 trace 与按根因分组的健康统计。

### 1.5 持久化与记忆

**Checkpoint**：per-turn 文件快照，mutating 工具在编辑前**自动捕获** pre-edit 内容；`undo` 只还原该 turn 记录的文件，**不触及 git 状态、不碰无关 untracked 文件**；manual create 不扫描整个项目（`src/tools/checkpoint.rs:1-5, 49-52, 71-83`）。前版此点基本准确。

**Memory consolidation（前版严重低估）**：不只是"合并相似项"，实际包含：
- **Staleness 检查**：`extract_memory_paths` 用正则提取记忆内容中的代码路径，`paths_all_missing` 判断全部缺失则标记 `stale_marked_at`（`src/context/consolidation.rs:17-103`）。
- **按类型差异化半衰期 decay**：`type_half_life_hours` 对 Knowledge/Preference ×4、Decision/Insight ×2、Error ×0.5、其余 ×1（`src/context/consolidation.rs:109-117`）。
- **关系分类**：`classify_relation` 返回 `Compatible / Contradicts / Ambiguous`，用本地启发式（状态变化标记 + 数值漂移判矛盾、子集关系判兼容）决定新旧记忆如何合并（`src/context/consolidation.rs:119-160`）。
- **相似度算法是 Jaccard（非 TF-IDF）**：`calculate_similarity`/`content_similarity` 为 "Type-agnostic Jaccard similarity over meaningful content tokens"（`src/context/consolidation.rs:638-658`），与 RLM replan 的 `jaccard_similarity` 同源；TF-IDF 仅用于**召回索引**，不参与 consolidation 合并。
- 双源存储（project 参与 TF-IDF 召回索引，global 每轮全量注入）、`exploration_epsilon` 探索召回、compact 抽取记忆质量门（importance 阈值 + ephemeral 噪声过滤 + Task 类型剔除，`src/agent/runtime/compactor.rs:183-236`）。

**Subagent trace**：完整 JSONL 记录子 Agent 执行过程，支持 `call_tree / html / chrome_trace` 渲染，`health` 按 `FailureRootCause` 分组统计失败模式。

---

## 二、真实缺口诊断

下表区分"真实缺口"与"前版误判"，避免把已实现能力当待建方案。

| # | 缺口 | 真实性 | 说明 |
|---|------|--------|------|
| 2.1 | 卡死检测仅精确匹配，无语义循环检测 | ✅ 真缺口 | 无法检测参数变体循环、语义循环（`grep("error handling")` → `grep("Error")` → 又搜 `error handling`）。前版方向对，但阈值数字错 |
| 2.2 | 无进度追踪与主动 refocus | ✅ 真缺口 | 无 `ProgressTracker`（files_read/files_modified/commands 计数、进度停滞/过度探索检测）；80% 轮次预警只打日志，不注入 refocus、不自动委托 |
| 2.3 | 失败子 Agent 中间状态未快照给新子 Agent | ✅ 真缺口（前版夸大） | `partial_results` 只传**已完成**任务成果；失败任务的新子 Agent 从零开始（仅 prompt + failure_reason 字符串）。但 trace 保留了完整过程，只是 replan 不消费它做状态继承。前版"中间状态全部丢失"夸大 |
| 2.4 | 结构化根因未传给 replan | ✅ 真缺口 | `FailureRootCause` 只用于 trace/health；replan 拿到的是字符串 error，Planner 无法据 `Timeout` vs `GuardianRejected` 采取针对性策略。前版"Planner 不知道失败原因"完全错误 |
| 2.5 | Jaccard 中文失效 | ✅ 真缺口（前版未指出要害） | 词袋按空白+ASCII 标点分词，中文无空格，整句成超长 token，Jaccard 基本失效。前版"词袋太弱"方向对但没抓到要害 |
| 2.6 | 会话级断点续传缺失 | ✅ 真缺口 | checkpoint 只做 per-turn 文件快照，非会话级上下文快照；进程崩溃/中断后能恢复完整 history，但压缩摘要 + 进度 + pending 子 Agent 状态无专门 checkpoint |
| 2.7 | 跨任务模式学习缺失 | ✅ 真缺口 | 无 `ExecutionPattern` 记忆类型，同一失败模式在会话内不记忆 |
| 2.8 | 上下文压缩的质量与边界 | ✅ 真缺口（前版把"已有"当"没有"） | 压缩已存在，缺口在：压缩粒度单一（一刀切 boundary）、无任务感知的 selective 压缩；summary 有丢失关键细节风险、无压缩质量评估；子 Agent 上下文是否复用主 Agent 压缩摘要（可能重复压缩/上下文割裂） |

---

## 三、解决方案：针对真实缺口的增量改进

原则：**只补真实缺口，不重复造已有轮子**。前版 §3.2.1 温层压缩、§3.2.2 SubagentSnapshot 部分功能已存在，此处仅保留真正增量。

### Layer 1: 检测层增强

#### 3.1 语义 StuckDetector
当前 `args_signature` 精确匹配检测不到参数变体/语义循环。升级为 embedding 余弦相似度：
- 对最近 N 轮 tool call 计算嵌入，`cos_sim(call_i, call_j) > 0.9` 且中间未改变文件系统状态 → 语义循环
- 用 API embedding 端点（轻量，无需本地模型）
- 检测后注入 `<stuck-warning>` 提示而非直接 abort

#### 3.2 ProgressTracker + 主动 refocus
```rust
struct ProgressTracker {
    files_read: HashSet<PathBuf>,
    files_modified: HashSet<PathBuf>,
    commands_succeeded: usize,
    commands_failed: usize,
}
```
- **进度停滞**：连续 N 轮 `files_modified` 与 `commands_succeeded` 不增长
- **过度探索**：`files_read` 增长但 `files_modified` 停滞
- 到达 80% 轮次时**主动注入 `<refocus>` 提示**（当前仅打日志），而非被动等到 abort

#### 3.3 Token 密度信号
追踪有效信息密度 `useful_chars / total_chars`，持续下降时提示缩小搜索范围。前版此点可保留。

### Layer 2: 失败恢复增强

#### 3.4 结构化根因注入 replan（中改、高价值）

**前置核实结论**（传递链已追完）：结构化根因在子 Agent 侧**已被捕获**，但**未回流到 RLM**，故本项不是纯 prompt 改动，需补回流链路。

传递链现状：
- 子 Agent 失败时，`FailureDiagnostics`（含 `root_cause: FailureRootCause`）在捕获点组装，写进 `ErrorInfo` -> `SubagentProgress` -> trace（`src/teams/subagent_loop.rs:818-840`）
- `run_subagent_loop_with_permissions` 返回 `Result<String, SubagentError>`（`src/teams/subagent_loop.rs:1192`），`SubagentError` 含 `error_type: ErrorType`（BudgetExceeded/Timeout/Stuck/ToolError/Cancelled/ModelUnavailable/Unknown）和 `code()`，但 `Display` 只输出 `message`，**丢弃 error_type**（`src/teams/subagent_loop.rs:177-180`）
- RLM pipeline `format!("Sub-task {} failed: {}", idx, e)`（`src/tools/meta/rlm/pipeline.rs:459`）只取 message 字符串，**error_type 和 root_cause 在此丢失**，进 `task_errors` -> `failure_reasons` -> Planner 的 `failure_reason`（纯字符串）

系统存在**两套分类体系**：
- `ErrorType`（子 Agent 循环层，粗粒度）：已在 `SubagentError` 返回链路，回流成本低
- `FailureRootCause`（根因层，细粒度，含 GuardianRejected/SandboxFailed/ToolPanic）：只在 `ErrorInfo`/trace，不在 `SubagentError`，回流需额外打通

两条改造路径：
- **路径 A（小改）**：`pipeline.rs:459` 保留 `SubagentError` 而非 format 成 String，提取 `error_type`/`code()` 传入 `compute_replan_scope` -> `failure_reasons` 带结构化字段 -> Planner prompt。覆盖 Timeout/Stuck/BudgetExceeded/ModelUnavailable 等循环层根因。
- **路径 B（中改）**：让 `SubagentError` 携带 `root_cause: FailureRootCause`（从 `FailureDiagnostics` 带出），或 RLM 从 progress store 读 `ErrorInfo.root_cause`。覆盖 GuardianRejected/SandboxFailed/ToolPanic 等细粒度根因，Planner 可据此采取针对性策略（权限被拒->换只读路径、沙箱失败->降级、超时->拆小）。

建议先做路径 A（成本低、覆盖主要失败模式），再按需补路径 B。

#### 3.5 SubagentSnapshot：失败子 Agent 状态继承
当前失败任务的新子 Agent 从零开始。补充结构化快照：
```rust
struct SubagentSnapshot {
    completed_steps: Vec<StepSummary>,
    files_read: HashSet<PathBuf>,
    files_modified: HashSet<PathBuf>,
    last_error: Option<String>,
    root_cause: FailureRootCause,
}
```
失败子 Agent（超时/stuck/max_rounds 耗尽）产出快照，replan 产生的新子 Agent 收到快照作为 prompt 一部分，避免重新读文件、重新执行命令。

#### 3.6 trace 消费做状态继承/根因佐证
trace 已完整记录子 Agent 过程（含 `FailedRoundContext`），但 replan 不消费它。在 §3.5 基础上，让 replan 从失败子 Agent trace 提取"已读文件清单 + 中间结论"填入 snapshot，而非让新子 Agent 重新探索。

#### 3.7 Jaccard 中文适配
当前 `jaccard_similarity` 按空白+ASCII 标点分词（`formats.rs:182-188`），中文整句成超长 token。适配方案：
- 中文段引入字符级 n-gram（bigram/trigram）或轻量分词
- 或对短 prompt 改用编辑距离/字符 Jaccard 兜底

### Layer 3: 持久化与学习

#### 3.8 会话级 SessionCheckpoint
当前 checkpoint 只做 per-turn 文件快照。补充会话级断点续传：
```rust
struct SessionCheckpoint {
    compaction_boundary: usize,
    compacted_summary: String,
    progress: ProgressTracker,
    pending_subagents: Vec<PendingSubagent>,
    working_set: HashSet<PathBuf>,
}
```
进程崩溃/用户中断后重启时恢复热/温上下文 + 进度 + pending 子 Agent，而非只能从完整 history 重新压缩。

#### 3.9 ExecutionPattern 跨任务学习
新增记忆类型，会话结束时由 LLM 从 trace 抽取执行模式（如"编辑 src/config/ 后需同步更新 settings.json schema"），写入 project memory，下次 TF-IDF 召回自动注入。

#### 3.10 压缩质量评估 + selective 压缩
- **质量评估**：压缩后对关键事实（文件路径、决策、待办）做召回校验，summary 丢失关键项时告警或保留原始片段
- **selective 压缩**：对当前任务 working_set 相关的轮次保留更多细节，无关轮次激进压缩
- **子 Agent 上下文复用**：子 Agent 继承主 Agent 的 `compacted_summary`，避免重复压缩/上下文割裂

---

## 四、实现路线图（按增量价值/成本排序）

### P0：小改、高价值（1-2 周）
1. **结构化根因注入 replan 路径 A**（§3.4）：保留 `SubagentError` 提取 `error_type`/`code()` 回流到 `failure_reasons`，覆盖 Timeout/Stuck/BudgetExceeded 等循环层根因
2. **Jaccard 中文适配**（§3.7）：分词器局部改动
3. **80% 轮次主动 refocus 注入**（§3.2 一部分）：把日志告警升级为提示注入

### P1：中改（2-4 周）
4. **结构化根因路径 B**（§3.4）：`SubagentError` 携带 `root_cause`，覆盖 GuardianRejected/SandboxFailed 细粒度根因
5. **ProgressTracker**（§3.2）：进度停滞/过度探索检测
6. **SubagentSnapshot + trace 消费**（§3.5、§3.6）：失败子 Agent 状态继承
7. **压缩质量评估**（§3.10 一部分）：summary 关键事实召回校验

### P2：大改（持续）
8. **语义 StuckDetector**（§3.1）：引入 embedding
9. **SessionCheckpoint**（§3.8）：会话级断点续传
10. **ExecutionPattern 学习**（§3.9）：跨任务经验积累
11. **selective 压缩 + 子 Agent 上下文复用**（§3.10）

---

## 五、对前版分析的勘误表

| 前版论断 | 实际 | 证据 |
|---------|------|------|
| `Some(0)` 注释说 unlimited 但"实际 None→unwrap_or(100) 并非无限"，是已知瑕疵 | `Some(0)` 确实映射 `None`（unlimited），有测试锁定，瑕疵不存在 | `config/mod.rs:198-199`、`tests.rs:195-200` |
| StuckDetector "3 次警告、5 次 abort" | 8 次警告、10 次 abort | `stuck_detector.rs:47-57` |
| "上下文线性累积、不做中间压缩" | 每轮自动检查阈值，超限自动压缩 | `loop_.rs:263-270`、`compaction.rs:376-394` |
| "compact 只做记忆抽取、不是历史压缩续传" | compact 同时做历史摘要 + 记忆抽取，输出 `{summary, memories}` | `compactor.rs:22-54` |
| "compact 需模型自己决定调用" | `needs_compaction` 阈值自动触发 | `loop_.rs:263`、`compaction.rs:376-394` |
| "Planner 只知道 task failed，不知道失败原因" | Planner 收到 `failure_reasons` + `partial_results`，prompt 明确指示用失败原因产生不同分解 | `pipeline.rs:530-536`、`planner.rs:205-264` |
| "子 Agent 失败后中间状态全部丢失" | trace 保留完整过程，`partial_results` 保留已完成任务成果；真缺口是失败任务中间状态未快照给新子 Agent | `failure_diagnostics.rs:30-33`、`pipeline.rs:507-518` |
| "memory 合并相似项"，且称"基于 TF-IDF 计算相似度" | 实际有 staleness 检查、类型差异化半衰期 decay、关系分类；**相似度用 Jaccard 非 TF-IDF**（TF-IDF 仅用于召回索引） | `consolidation.rs:17-103, 109-117, 119-160, 638-658` |
| §3.2.1"温层压缩引擎"作为待建核心方案 | 热/温/冷分层已事实存在，应改为讨论压缩质量与边界 | `loop_.rs:277-295` |
| 未提及 replan 传递性下游依赖重做、executor 预算约束 | 均已实现 | `pipeline.rs:936-945, 481-504` |

---

## 六、相关文件索引

| 文件 | 内容 |
|------|------|
| `src/agent/runtime/loop_.rs` | 主循环：压缩触发、`compaction_boundary`、`max_rounds`、stuck 集成 |
| `src/agent/runtime/compaction.rs` | `needs_compaction` 阈值、`split_for_compaction`、`assemble_post_compaction_history`、`micro_compact_messages`、calibration |
| `src/agent/runtime/compactor.rs` | `Compactor` trait + `ApiCompactor`：`compact()`、`COMPACTION_SYSTEM_PROMPT`、`fallback_micro_compact`、记忆质量门 |
| `src/utils/stuck_detector.rs` | `StuckDetector`：`args_signature` 精确匹配，8/10 阈值 |
| `src/tools/meta/rlm/pipeline.rs` | RLM 管线：replan 主流程、`compute_replan_scope`（下游传播）、`jaccard_dedup_replacements`、executor 预算 |
| `src/tools/meta/rlm/planner.rs` | `replan_incremental`：replan prompt 构造，`failures_json` + `partial_json` |
| `src/tools/meta/rlm/formats.rs` | `jaccard_similarity`：token-level 词袋（中文失效） |
| `src/teams/failure_diagnostics.rs` | `FailureRootCause` 枚举、`FailedRoundContext`、`ToolCallStep`、脱敏 |
| `src/teams/subagent_health.rs` | 按 `FailureRootCause` 分组的子 Agent 健康统计 |
| `src/tools/checkpoint.rs` | per-turn 文件快照、`undo`（不碰 git） |
| `src/context/consolidation.rs` | staleness 检查、类型半衰期 decay、`MemoryRelation` 关系分类 |
| `src/context/memory.rs` | `MemoryManager`：双源存储、TF-IDF 召回 |
| `src/config/mod.rs` | `resolve_subagent_config`：`Some(0)`→`None` 映射 |

---

## 七、待讨论的设计决策

1. **语义 StuckDetector 的 embedding 来源**：API embedding 端点 vs 本地 fastembed（延迟/成本/离线权衡）
2. **SubagentSnapshot 的序列化边界**：`files_read` 全量传递 vs 仅传 working_set 差额（避免 snapshot 本身过大）
3. **结构化根因注入 replan 的回流链路**（已核实）：`FailureRootCause`/`ErrorType` 在子 Agent 侧已捕获，但 `SubagentError::Display` 丢弃 error_type、RLM `format!` 只取 message，故根因未回流到 `failure_reasons`。§3.4 需补回流（路径 A 保留 SubagentError 提取 error_type，路径 B 携带 root_cause），非纯 prompt 改动。详见 §3.4。
4. **SessionCheckpoint 的存储位置**：复用 `<project>/.wgenty-code/sessions/` 还是独立目录
5. **优先级取舍**：单次长任务可靠性（§3.1/3.2/3.10 优先）vs 多子任务编排可靠性（§3.4/3.5/3.6 优先）
