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
      # Q2: Executor 图分析确定替换集
      replace_ids = transitive_dependents(failures.idx, deps)  # 失败子任务 + 下游
      # Q3: 从后续层移除 replace_ids（防白跑）
      pending_remove |= replace_ids
      # Q2: 调用 incremental Planner
      repl = Planner::replan_incremental(
        original_plan, replace_ids, failure_reasons, partial_results_of_completed)
      # Q5: jaccard 去重
      repl.replacements = repl.replacements.filter(|r| 
        jaccard_similarity(r.prompt, failed_prompt[r.replaces_id]) < jaccard_threshold)
      # 校验 depends_on 引用合法性（仅保留 id）
      repl.replacements = repl.replacements.filter(|r| 
        r.depends_on.all(|d| preserved_ids.contains(d)))
      # Q3: 重 level 替换子任务（基于保留 id 已知 depth）
      relevel(repl.replacements, preserved_depths)
      # Q4: 预算检查
      if budget.executor_remaining < replanner_call_cost: mark [ERROR], continue
      执行 repl.replacements (reserve_child + fallback，Q2 同路径)
      quota.remaining -= 1
    # 已完成的 partial result 存入 partial_results 供下次 replan
```

**替换映射 schema**（Q2）：
```json
{
  "replacements": [
    {"replaces_id": 2, "prompt": "...", "use_small_model": false, "depends_on": [0, 1]},
    {"replaces_id": 4, "prompt": "...", "use_small_model": true,  "depends_on": [2]}
  ]
}
```
`replaces_id` 指向被替换的原 sub-task id；`depends_on` 仅引用 preserved（未替换）id。

**Incremental Planner prompt**（Q2）：输入 = 原 plan JSON + replace_ids + 各自 failure
reason + 已完成子任务 partial result；指令「仅重新分解指定 id 的子任务，产出的替换
`depends_on` 只能引用未替换的 id，不得重新分解已完成子任务」。

### 4.3 配置接入

`run_rlm_pipeline` 从 `settings.rlm` 读 `RlmSettings`：
- `retry_enabled`（Q1）：门控整个 replan 路径。false 时失败直接标 `[ERROR]`（现状）。
- `max_replan_cycles`（Q3/全局配额）：`ReplanQuota` 计数器，每次 replan 递减，跨整个
  run 共享。耗尽后失败标 `[ERROR]`。
- `jaccard_threshold`（Q5）：replan 去重阈值。

Subagent 覆盖：当 RLM caller 是 subagent（`caller.depth > 0`），用
`SubagentRlmOverride`（若配置）覆盖 `retry_enabled`/`max_replan_cycles`；无 override
用 top-level。复用现有 `SubagentRlmOverride` 机制。

### 4.4 预算（Q4）

replanner API 调用 + 替换子任务执行从 `BudgetAllocation` 的 executor pool 按需扣除
（复用 `distribute_to_tasks` / `rollover_unused`）。检查点：发 replanner 调用前，
若 executor_remaining 不足以覆盖一次 replanner 调用（估算成本），不 replan（标
`[ERROR]`）。未用 replan 预算经 `rollover_unused("executor", ...)` 滚入 aggregator
（现有语义）。

### 4.5 jaccard 去重（Q5）

`executor.rs` import `crate::tools::meta::rlm::formats::jaccard_similarity`（现有
pub fn）。对每个替换子任务，计算其 prompt 与对应 `replaces_id` 失败子任务 prompt 的
Jaccard 相似度；>= `jaccard_threshold` 则拒绝该替换（请求新替换或耗尽标 `[ERROR]`）。
`formats.rs` 死代码 `Aggregator`/`deduplicate_claims` 原样不动。

### 4.6 P2-2 验证

补测试：RLM 子任务执行时 `max_concurrent` permit 生效（`reserve_child` 获取 semaphore
`coordinator.rs:448`）；structural fallback 仅 permit 耗尽时触发。更新 `WGENTY.md`
标注 P2-2（统一并行编排路径）已由现有 permit 设计满足。

## 5. 不变量（延续 spec §9）

- `src/exec_session/` 代码无 "comet" 字符串（RLM 在 `tools/meta/rlm/`，不碰 exec_session）。
- 崩溃一致性：RLM 不写 `session.json`，replan 不引入新持久化崩溃面。
- 契约由 agent 声明、runtime 执行（replan 重新分解由 Planner API 产出，Executor 执行）。
- 向后兼容：`retry_enabled=false` 时行为与现状完全一致；无新配置项。

## 6. 测试策略

- **模块提取回归**：提取后 `cargo test` 全绿，无 replan 时失败仍标 `[ERROR]`。
- **配置接入**：top-level `RlmSettings` 生效；subagent override 覆盖；无 override 回退。
- **incremental Planner**：输出 `depends_on` 仅引用 preserved id；替换集不含已完成子任务；非法引用被拒。
- **replan 核心**：1 失败 -> replan -> 替换成功 -> `failed=0`；配额耗尽 -> `[ERROR]` 聚合继续；`retry_enabled=false` -> 不 replan；join error -> 不 replan 不耗配额。
- **jaccard 去重**：替换与失败子任务 jaccard≥阈值 -> 拒绝；< 阈值 -> 接受执行。
- **预算**：replan 扣 executor pool；预算不足不 replan；剩余 rollover aggregator。
- **P2-2**：permit 生效；fallback 仅 permit 耗尽时触发。
- **不变量**：`exec_session/` 无 "comet"；`cargo test --all` + `cargo clippy --all-targets -- -D warnings` + `cargo fmt --check` 全绿。

## 7. Spec Patch（已回写）

- `specs/rlm-replan/spec.md`：「Replan gated by configuration」加 join-error 不触发
  scenario；「Local replan scope」加 Executor 图分析确定依赖集 scenario。
- `specs/rlm-budget-control/spec.md`：无 patch（Q4-B 与现有 ADDED requirement 一致）。
