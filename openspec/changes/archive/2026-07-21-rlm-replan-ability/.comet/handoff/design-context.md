# Comet Design Handoff

- Change: rlm-replan-ability
- Phase: design
- Mode: compact
- Context hash: 60500181942cd3efba8a98532167749517472cac059c58b57e0357db9efcc64f

Generated-by: comet-handoff.sh

OpenSpec remains the canonical capability spec. This handoff is a deterministic, source-traceable context pack, not an agent-authored summary.

## openspec/changes/rlm-replan-ability/proposal.md

- Source: openspec/changes/rlm-replan-ability/proposal.md
- Lines: 1-91
- SHA256: 973950f1ecfb72a25034e404ace512b4d4b32fa6945473a4a9e8acaefce77cd0

[TRUNCATED]

```md
# Proposal: RLM Task-Level Replan Ability

## Why

The RLM pipeline (`run_rlm_pipeline`) decomposes a complex task into up to 8 sub-tasks
(via a single Planner API call), executes them level-by-level in parallel through the
`AgentCoordinator` permit model, and aggregates results. When a sub-task fails, the
Executor only records `results[idx] = "[ERROR] ..."` and proceeds to the next dependency
level; the Aggregator then merges the `[FAILED]` entries as-is. There is **no retry and no
re-decomposition** at the task-orchestration layer.

This leaves long-running autonomy with a core gap: a complex task that fails once at
decomposition or execution is abandoned, with no ability to dynamically adjust. The node
state machine already provides node-level AutoRetry, but task-level replan (re-decompose a
failed sub-task + its downstream dependents) is a blank. Together they would form a
complete two-tier retry system (task-level + node-level).

Notably, the configuration scaffolding for this already exists but is **unwired**:
`RlmSettings { retry_enabled: true, max_replan_cycles: 2, jaccard_threshold: 0.8 }`
(`src/config/guardian.rs`) plus a `SubagentRlmOverride` path (`src/config/agent.rs`), with
config tests asserting the defaults. A grep for `replan|retry|max_replan` across
`src/tools/meta/rlm/` returns no matches: the pipeline never reads these fields. The
`jaccard_threshold` value also appears hardcoded as `0.8` in `formats.rs:232`
(`deduplicate_claims`) within a structured `Aggregator` that the live pipeline never calls
(the pipeline's aggregator phase does a plain LLM merge). So all three fields are
effectively unused scaffolding awaiting this feature.

## What Changes

1. **Extract `Planner` and `Executor` from the monolithic `run_rlm_pipeline`**
   (`src/tools/meta/rlm/pipeline.rs`, ~630 lines) into `planner.rs` and `executor.rs`,
   matching the module layout referenced in the improvement spec. The pipeline becomes a
   thin orchestrator. Behavior-preserving refactor; no functional change.

2. **Add task-level replan to the Executor**: when a sub-task fails (and replan is enabled
   and the global quota is not exhausted), re-invoke the Planner in **incremental mode** —
   input is the original plan + the failed sub-task id + failure reason (+ completed
   sub-tasks' partial results as context), output is a replacement sub-task set for the
   failed sub-task and its downstream dependents. Replacement sub-tasks are executed
   through the same coordinator permit + structural-fallback path. The pipeline runs at
   most `max_replan_cycles` replans total across the whole run (global shared quota).

3. **Wire the existing `RlmSettings`** into the pipeline: `retry_enabled` gates replan;
   `max_replan_cycles` bounds total replans per run; `jaccard_threshold` deduplicates
   replan output (reject a replacement sub-task whose prompt is semantically identical,
   Jaccard >= threshold, to the failed one — do not regenerate the same failure).
   Subagent-level `SubagentRlmOverride` applies when the RLM caller is a subagent.

4. **P2-2 verification (already-done confirmation)**: verify and document that RLM
   sub-tasks already flow through `AgentCoordinator::reserve_child` (acquires the global
   concurrency semaphore, `coordinator.rs:448`) and `finish_child` (releases the permit,
   `coordinator.rs:671`), called at `pipeline.rs:329` / `pipeline.rs:483`. The only permit
   "bypass" is the structural-fallback ghost path, which is the intended degradation when
   permits are saturated (mirrors the `task` tool). Update `WGENTY.md` to mark P2-2
   (unified parallel orchestration) as satisfied by the existing design.

## Capabilities Affected

- `rlm-replan` (NEW) — task-level replan: trigger on sub-task failure, local scope
  (failed sub-task + downstream dependents), `max_replan_cycles` global quota,
  `jaccard_threshold` output dedup, `retry_enabled` gate, incremental Planner mode.
- `rlm-budget-control` (MODIFIED) — replan consumes budget (replanner call +
  re-execution of replacement sub-tasks); define how replan budget is carved from the
  executor pool and how unused replan budget rolls forward.

## Out of Scope

- P0-1 memory dedup — already implemented (`add_memory` dedup-on-write + `prune`
  consolidation + orphan cleanup); the improvement spec is stale on this point.
- Other improvement points (P1-1 comet verify subagentization, P1-2 micro-compaction,
  P1-3 cross-session resume, P2-1 node hierarchy, P2-3 verify-contract checks,
  P3-1/P3-2/P3-3) — each is an independent change per the roadmap.
- Node-level AutoRetry — orthogonal; RLM replan is task-level and does not modify node
  state machine behavior.
- Single-shot Planner / Aggregator core logic — unchanged; only a new incremental replan
  path is added.
- Structured `Aggregator` dead code in `formats.rs` — observed but not cleaned up here
  (unrelated; avoid scope creep).
- New configuration fields — reuse existing `RlmSettings` / `SubagentRlmOverride`; no
  new keys (backward compatible).
```

Full source: openspec/changes/rlm-replan-ability/proposal.md

## openspec/changes/rlm-replan-ability/design.md

- Source: openspec/changes/rlm-replan-ability/design.md
- Lines: 1-173
- SHA256: 11f816b84543a29503b57ad0ec46811c13b80e405f62460e63192a57cda5a5a6

[TRUNCATED]

```md
---
comet_change: rlm-replan-ability
role: technical-design
canonical_spec: openspec
---

# Design Doc: RLM Task-Level Replan Ability

**日期**: 2026-07-20
**状态**: design（定稿，5 个 Open Question 已决议）
**change**: `openspec/changes/rlm-replan-ability`
**关联**: `docs/superpowers/specs/2026-07-20-long-running-autonomy-improvements.md` §3 P0-2

## 1. Context（探索结论）

`run_rlm_pipeline`（`src/tools/meta/rlm/pipeline.rs`，~630 行单体函数）三阶段：
Planner（L113-178，单次 API 产出最多 8 个 `{prompt, use_small_model, depends_on}`
子任务）-> Executor（L200-506，按依赖 `depth` 分层并行，经 `coordinator.reserve_child`
+ 结构性 fallback）-> Aggregator（L532-578，单次 LLM merge）。失败处理（L489-505）：
`Ok(Err) -> task_errors[idx]=Some; results[idx]="[ERROR] ..."`，**无重试、无重新分解**。

**关键发现**：
- 配置脚手架已存在但**未接入**：`RlmSettings { retry_enabled: true,
  max_replan_cycles: 2, jaccard_threshold: 0.8 }`（`src/config/guardian.rs`）+
  `SubagentRlmOverride`（`src/config/agent.rs`）。`rg "replan|retry|max_replan"
  src/tools/meta/rlm/` 无匹配。`jaccard_threshold` 在 `formats.rs:232`
  `deduplicate_claims` 硬编码 0.8，且该结构化 `Aggregator` **未被 pipeline 调用**
 （死代码）。三字段实质都是未用脚手架。
- **P2-2 已完成**：RLM 子任务已走 `reserve_child`/`finish_child` permit 路径
 （`pipeline.rs:329/483` + `coordinator.rs:448/671`）。唯一"绕过"是结构性
  fallback ghost 路径（permit 饱和时设计内降级，镜像 `task` 工具）。
- **P0-1 已实现**（记忆去重），不在本 change 范围。

## 2. Goals / Non-Goals

**Goals**：子任务失败时触发局部 replan（重新分解失败子任务 + 下游依赖）；接入已有
`RlmSettings`；提取 Planner/Executor 模块；P2-2 验证 + 文档。

**Non-Goals**：不改 node 级 AutoRetry；不重做单次 Planner/Aggregator 核心逻辑；
不清理 `formats.rs` 死代码；不新增配置字段；不做 P0-1 及其他改进点。

## 3. Decisions（5 Open Question 决议）

| # | 决策 | 选项 | 理由 |
|---|------|------|------|
| Q1 | replan 触发边界 | **A：仅 `Ok(Err)`** | replan 针对「分解错误/子任务太难」的逻辑失败（含 fallback-then-fail）。Join error（panic/cancel）是基础设施故障，不 replan。符合 spec「试错修正」。 |
| Q2 | Planner 契约 | **A：Executor 驱动图分析 + 显式替换映射** | 依赖图是确定性数据，Executor 拥有（不信任 LLM 算依赖）；显式 `{replaces_id, ...}` 映射无歧义；Planner 聚焦分解。 |
| Q3 | replan 时机 + depth | **A：早 replan + 重 level 子树** | 每层完成后检查失败，下游依赖（未跑）从后续层移除，防止 doomed 依赖白跑；替换子任务按 `depends_on` 重 level，正确性优先。 |
| Q4 | replan 预算 | **B：按需扣 executor pool** | 复用现有 `rollover_unused` 语义；无 replan 时全额预算可用（常见情况）；预算不足不 replan（标 `[ERROR]`）。 |
| Q5 | jaccard 去重 + 死代码 | **A：复用 `jaccard_similarity`，不动死代码** | DRY；config 字段首次活用；死代码清理 out of scope。 |

## 4. 详细设计

### 4.1 模块结构（行为保持重构）

```
src/tools/meta/rlm/
  mod.rs          # pub use + 模块声明
  pipeline.rs     # 薄编排器：run_rlm_pipeline -> Planner::plan -> Executor::execute -> Aggregator::aggregate
  planner.rs      # Planner { plan(), replan_incremental() } + SubTask 结构体
  executor.rs     # Executor { execute() } + replan 循环 + ReplanQuota
  aggregator.rs   # Aggregator { aggregate() }（从 pipeline.rs 迁移）
  budget.rs       # 现有 BudgetAllocation（replan 复用）
  formats.rs      # 现有（死代码 Aggregator 不动；复用 jaccard_similarity）
  error.rs / ports.rs  # 现有
```

`run_rlm_pipeline` 公开签名不变（向后兼容）。`Planner`/`Executor`/`Aggregator` 为
内部结构体，构造时接收所需依赖（settings、tool_registry、coordinator、caller 等）。

### 4.2 Replan 数据流

```
Executor::execute(sub_tasks, rlm_settings, budget, quota):
  compute depth[] (现有逻辑)
  for level in 0..max_depth:
    level_data = sub_tasks.filter(depth==level && !replaced)   # 跳过被 replan 移除的
    并行执行 level_data (reserve_child + fallback，现有路径)
    收集 failures = [(idx, reason)]  # 仅 Ok(Err)，Q1
    if rlm_settings.retry_enabled && failures.non_empty() && quota.remaining > 0:
```

Full source: openspec/changes/rlm-replan-ability/design.md

## openspec/changes/rlm-replan-ability/tasks.md

- Source: openspec/changes/rlm-replan-ability/tasks.md
- Lines: 1-53
- SHA256: e8038cca2d97702c6604155c703eb4e6f823324f89c7b7bdf7a30ac7074e79ba

```md
## 1. 模块提取（行为保持的重构）

- [ ] 1.1 新建 `src/tools/meta/rlm/planner.rs`，将 `run_rlm_pipeline` 的 Planner 阶段（L113-185：prompt 构造、API 调用、JSON 解析、`take(8)`）迁移为 `Planner::plan(task, context) -> Vec<SubTask>`，定义 `SubTask` 结构体
- [ ] 1.2 新建 `src/tools/meta/rlm/executor.rs`，将 Executor 阶段（L200-506：depth 计算、分层并行、`reserve_child` + 结构性 fallback、失败收集）迁移为 `Executor::execute(...) -> ExecutorOutcome`
- [ ] 1.3 将 Aggregator 阶段（L532-578）迁移为 `Aggregator::aggregate(results) -> String`（可放 `planner.rs` 或新建 `aggregator.rs`）
- [ ] 1.4 `pipeline.rs` 重构为薄编排器，调用 `Planner::plan` -> `Executor::execute` -> `Aggregator::aggregate`，保留 `run_rlm_pipeline` 公开签名
- [ ] 1.5 `cargo test` 全绿 + `cargo clippy --all-targets -- -D warnings` 零 warning，确认行为无变化（无 replan，失败仍标 `[ERROR]`）

## 2. 配置接入

- [ ] 2.1 在 `run_rlm_pipeline` 签名中传入 `RlmSettings`（从 `Settings.rlm` 读取），或在函数内从 `settings` 读取；保留向后兼容默认
- [ ] 2.2 解析 `SubagentRlmOverride`：当 RLM caller 是 subagent 时，用 override 覆盖 `retry_enabled`/`max_replan_cycles`；无 override 时用 top-level `RlmSettings`
- [ ] 2.3 单测：top-level `RlmSettings` 生效；subagent override 覆盖；无 override 回退 top-level

## 3. Incremental Planner mode（replan 输入/输出契约）

- [ ] 3.1 在 `Planner` 新增 `replan_incremental(original_plan, failed_ids, reasons, partial_results) -> Vec<SubTask>`（增量模式，与 `plan` 区分）
- [ ] 3.2 设计 incremental replan prompt（输入：原 plan + 失败子任务 id + 失败原因 + 已完成子任务 partial result；输出：替换子任务集合，`depends_on` 引用保留的子任务 id）——精确契约留待 /comet-design 定稿
- [ ] 3.3 单测：replan 输出的 `depends_on` 引用仅指向保留（未替换）子任务 id；替换集不含已完成子任务

## 4. Executor replan 循环

- [ ] 4.1 在 Executor 层内失败收集后加入 replan 决策：`retry_enabled && failures.non_empty() && replan_quota > 0` 时触发
- [ ] 4.2 实现局部 replan 范围：识别失败子任务的下游依赖（transitive `depends_on`），仅替换失败子任务 + 下游
- [ ] 4.3 实现全局配额计数器：`max_replan_cycles` 跨整个 pipeline run 共享，每次 replan 递减；耗尽后失败直接标 `[ERROR]`（向后兼容）
- [ ] 4.4 替换子任务经同一 `coordinator.reserve_child` + `finish_child` permit 路径 + 结构性 fallback 执行
- [ ] 4.5 替换子任务的 depth 归属策略（继承失败子任务 level / 追加新 level）——留待 /comet-design 定稿
- [ ] 4.6 单测：核心成功场景（1 失败 -> replan -> 替换成功 -> `failed=0`）；配额耗尽场景（2 次 replan 都失败 -> `[ERROR]` 聚合继续）；关闭 replan 场景（`retry_enabled=false` -> 不 replan）

## 5. Replan 输出去重

- [ ] 5.1 对 replanner 产出的替换子任务，按 prompt 文本计算与对应失败子任务的 Jaccard 相似度，阈值用 `RlmSettings.jaccard_threshold`
- [ ] 5.2 相似度 >= 阈值的替换被拒绝（不执行）；请求新替换或耗尽则标 `[ERROR]`
- [ ] 5.3 单测：替换与失败子任务 jaccard≥阈值 -> 拒绝；< 阈值 -> 接受并执行

## 6. Replan 预算集成

- [ ] 6.1 replanner API 调用 + 替换子任务重执行从 `BudgetAllocation` 的 executor pool 扣除
- [ ] 6.2 整体消耗不超 `delegate` 的 `token_budget_k`；剩余 executor pool 预算不足以发 replanner 调用时不 replan（标 `[ERROR]`）
- [ ] 6.3 未用 replan 预算经 `rollover_unused` 滚入 aggregator（复用现有语义）
- [ ] 6.4 单测：replan 扣 executor pool；预算不足时不 replan；剩余滚入 aggregator

## 7. P2-2 验证 + 文档

- [ ] 7.1 补测试覆盖：RLM 子任务执行时 `max_concurrent` permit 生效（reserve_child 获取 semaphore）；structural fallback 仅 permit 耗尽时触发
- [ ] 7.2 更新 `WGENTY.md`：标注 P2-2（统一并行编排路径）已由现有 `reserve_child`/`finish_child` permit 设计满足，structural fallback 为设计内降级

## 8. 不变量与回归

- [ ] 8.1 确认 `src/exec_session/` 代码无 "comet" 字符串（除枚举变体与注释）
- [ ] 8.2 确认 RLM 不写 `session.json`，replan 不引入新持久化崩溃面
- [ ] 8.3 `cargo test --all` + `cargo clippy --all-targets -- -D warnings` + `cargo fmt --check` 全绿
- [ ] 8.4 非 git 项目降级路径：config 默认值向后兼容，无新配置项
```

## openspec/changes/rlm-replan-ability/specs/rlm-budget-control/spec.md

- Source: openspec/changes/rlm-replan-ability/specs/rlm-budget-control/spec.md
- Lines: 1-18
- SHA256: 65cea7ef5e810972c88dd2d56f26776f87bb26c855771ba227042d9637a78013

```md
## ADDED Requirements

### Requirement: Replan phase budget allocation
When replan is enabled and a replan is triggered, the replanner API call and the re-execution of replacement sub-tasks SHALL consume budget from the executor phase pool of the `BudgetAllocation`. The total replan consumption (replanner call + replacement sub-task executions) SHALL NOT cause the pipeline to exceed the overall `token_budget_k` limit of the invoking `delegate` call. Unused replan budget SHALL roll forward to the aggregator phase using the same `rollover_unused` semantics as the executor phase.

#### Scenario: Replan consumes executor pool budget
- **WHEN** a replan is triggered and the replanner API call plus replacement sub-task executions consume X tokens
- **THEN** X tokens SHALL be deducted from the executor phase pool of the `BudgetAllocation`
- **AND** the overall pipeline token consumption SHALL NOT exceed the `delegate` call's `token_budget_k` limit

#### Scenario: Unused replan budget rolls to aggregator
- **WHEN** a replan consumes less than its allocated portion of the executor pool
- **THEN** the unused remainder SHALL roll forward to the aggregator phase via `rollover_unused`

#### Scenario: Budget exhaustion prevents replan
- **WHEN** a sub-task fails and the remaining executor pool budget is insufficient for a replanner API call
- **THEN** the Executor SHALL NOT attempt a replan
- **AND** the failed sub-task SHALL be recorded as `[ERROR]`
```

## openspec/changes/rlm-replan-ability/specs/rlm-replan/spec.md

- Source: openspec/changes/rlm-replan-ability/specs/rlm-replan/spec.md
- Lines: 1-102
- SHA256: 482227f996e9f2bc9fca5e433ed37aa3d564072bfa968050f1714e43694d4ac5

[TRUNCATED]

```md
## ADDED Requirements

### Requirement: Replan gated by configuration
The RLM Executor SHALL attempt a task-level replan when a sub-task fails, but only when `RlmSettings.retry_enabled` is true. When replan is disabled, a failed sub-task SHALL be recorded as `[ERROR]` and the pipeline SHALL proceed exactly as before this change (backward compatible).

#### Scenario: Replan disabled preserves legacy behavior
- **WHEN** a sub-task fails during executor phase
- **AND** `settings.rlm.retry_enabled` is false
- **THEN** the Executor SHALL record `results[idx] = "[ERROR] ..."` and proceed to the next dependency level without invoking the replanner
- **AND** the final `RlmResult.failed` count SHALL include the failed sub-task

#### Scenario: Replan enabled attempts recovery
- **WHEN** a sub-task fails during executor phase
- **AND** `settings.rlm.retry_enabled` is true
- **AND** the global replan quota is not exhausted
- **THEN** the Executor SHALL invoke the incremental Planner to produce a replacement sub-task set for the failed sub-task and its downstream dependents

#### Scenario: Join errors do not trigger replan
- **WHEN** a spawned sub-task fails with a join error (panic or cancellation, i.e. the `tokio::spawn` handle returns `Err`)
- **THEN** the Executor SHALL treat it as an infrastructure failure and SHALL NOT invoke the replanner
- **AND** the global replan quota SHALL NOT be consumed

### Requirement: Local replan scope
A replan SHALL re-decompose only the failed sub-task and its downstream dependents (sub-tasks that transitively `depends_on` the failed one). Sub-tasks that already completed successfully SHALL be preserved and their partial results SHALL be passed to the incremental Planner as context. The replanner SHALL NOT re-decompose the entire original plan from scratch.

#### Scenario: Failed sub-task with downstream dependents
- **WHEN** sub-task 2 fails and sub-tasks 4 and 5 transitively depend on sub-task 2
- **THEN** the replanner input SHALL include the original plan, sub-task 2's id and failure reason, and the partial results of completed sub-tasks
- **AND** the replanner output SHALL be a replacement set covering sub-task 2 and its downstream dependents 4 and 5
- **AND** the replacement set SHALL NOT include already-completed sub-tasks

#### Scenario: Failed sub-task with no downstream dependents
- **WHEN** a leaf sub-task (no dependents) fails
- **THEN** the replanner SHALL produce a replacement for only that single failed sub-task

#### Scenario: Executor determines downstream dependents via graph analysis
- **WHEN** a sub-task fails and the Executor identifies its downstream dependents
- **THEN** the Executor SHALL compute the transitive closure of `depends_on` (deterministic graph analysis) to determine which sub-task ids to replace
- **AND** the Planner SHALL NOT be responsible for identifying dependents; it SHALL only decompose the replacement set for the ids the Executor supplies

### Requirement: Global replan quota
The total number of replans across a single `run_rlm_pipeline` invocation SHALL NOT exceed `RlmSettings.max_replan_cycles` (default 2). The quota is a global shared counter across all failed sub-tasks in the run, not a per-sub-task limit. When the quota is exhausted, subsequent failures SHALL be recorded as `[ERROR]` without replan.

#### Scenario: Quota shared across failures
- **WHEN** two different sub-tasks fail in the same pipeline run
- **AND** `max_replan_cycles` is 2
- **THEN** the first failure SHALL consume one replan and the second failure SHALL consume the second replan
- **AND** a third failure in the same run SHALL be recorded as `[ERROR]` without replan

#### Scenario: Quota exhausted falls back to legacy marking
- **WHEN** a sub-task fails and the replan quota is already exhausted
- **THEN** the Executor SHALL record `results[idx] = "[ERROR] ..."` and proceed without invoking the replanner

### Requirement: Replan output deduplication
The Executor SHALL reject a replacement sub-task whose prompt is semantically identical to the failed sub-task it replaces, using Jaccard similarity on the prompt text with threshold `RlmSettings.jaccard_threshold` (default 0.8). A rejected replacement SHALL NOT be executed; the Executor SHALL either request another replacement from the Planner or, if none is available, record the sub-task as `[ERROR]`.

#### Scenario: Replacement identical to failed sub-task is rejected
- **WHEN** the replanner produces a replacement sub-task whose prompt has Jaccard similarity >= `jaccard_threshold` to the failed sub-task's prompt
- **THEN** the Executor SHALL reject that replacement and SHALL NOT execute it
- **AND** the Executor SHALL either request a distinct replacement or record the sub-task as `[ERROR]`

#### Scenario: Distinct replacement is accepted
- **WHEN** the replanner produces a replacement sub-task whose prompt has Jaccard similarity < `jaccard_threshold` to the failed sub-task's prompt
- **THEN** the Executor SHALL execute the replacement through the same coordinator permit and structural-fallback path as a normal sub-task

### Requirement: Incremental Planner mode
The Planner SHALL expose an incremental replan mode distinct from the initial decomposition mode. The incremental mode input SHALL be: the original plan, the failed sub-task id(s), the failure reason(s), and the partial results of completed sub-tasks. The output SHALL be a replacement sub-task set with `depends_on` references valid against the preserved (non-replaced) sub-task ids, so the replacement set integrates into the existing dependency graph without invalidating completed work.

#### Scenario: Incremental replan preserves completed work
- **WHEN** the incremental Planner is invoked after sub-task 2 fails
- **AND** sub-tasks 0 and 1 have completed successfully
- **THEN** the replacement sub-tasks SHALL reference preserved sub-task ids (e.g. depend on 0 or 1) where dependencies exist
- **AND** the replacement SHALL NOT redefine or invalidate sub-tasks 0 and 1

#### Scenario: Replacement integrates into dependency graph
- **WHEN** the replacement set is produced for failed sub-task 2 and its dependent sub-task 4
- **THEN** the replacement sub-tasks SHALL be executable against the existing `depth`/dependency structure without requiring the entire plan to be re-leveled from scratch

### Requirement: Replacement sub-task execution path
Replacement sub-tasks produced by a replan SHALL execute through the same `AgentCoordinator::reserve_child` permit model and structural-fallback path as normal sub-tasks, so global concurrency limits and the existing fallback semantics apply uniformly. A replacement SHALL NOT bypass the coordinator permit except via the same structural-fallback ghost path used for normal sub-tasks.
```

Full source: openspec/changes/rlm-replan-ability/specs/rlm-replan/spec.md

