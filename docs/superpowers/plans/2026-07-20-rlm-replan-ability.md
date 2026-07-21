---
change: rlm-replan-ability
design-doc: openspec/changes/rlm-replan-ability/design.md
base-ref: 21637d09c45264b696785cdbb3b590e2d5430a5d
---

# Implementation Plan: RLM Task-Level Replan Ability

**Design Doc**: `openspec/changes/rlm-replan-ability/design.md`
**Tasks (boundaries)**: `openspec/changes/rlm-replan-ability/tasks.md`
**Change**: `rlm-replan-ability`

## 执行原则

- **顺序约束**：Phase 1（模块提取，行为保持）必须先完成且全绿，作为安全检查点；Phase 2-4（replan 逻辑）建立在干净抽象上。
- **每个 task 一个提交**，commit message 体现设计意图（Conventional Commits）。
- **验证门槛**：每个 Phase 结束跑 `cargo test --all` + `cargo clippy --all-targets -- -D warnings` + `cargo fmt --check` 全绿才进入下一 Phase。
- **不变量**：`src/exec_session/` 无 "comet" 字符串；RLM 不写 `session.json`；向后兼容（`retry_enabled=false` 行为同现状）。

---

## Phase 1：模块提取（行为保持重构）

### Task 1.1 — 定义 `SubTask` 结构体 + 新建模块骨架
**文件**: `src/tools/meta/rlm/mod.rs`（新增 `pub mod planner/executor/aggregator`）、`src/tools/meta/rlm/planner.rs`
**做**:
- 在 `planner.rs` 定义 `pub struct SubTask { pub prompt: String, pub use_small_model: bool, pub depends_on: Vec<usize> }`（从当前 pipeline.rs 内联的 `serde_json::Value` 解析升级为强类型）
- 定义 `pub struct Planner { client: ApiClient }` + `Planner::new(client)`
**验收**: `cargo check` 通过；`SubTask` 可从现有 JSON 解析
**依赖**: 无

### Task 1.2 — 迁移 Planner 阶段到 `Planner::plan`
**文件**: `src/tools/meta/rlm/planner.rs`
**做**:
- 将 `run_rlm_pipeline` L113-185（planner prompt 构造、API 调用、`extract_json`、`serde_json::from_str`、`take(8)`、解析失败回退单子任务）迁移为 `Planner::plan(&self, task: &str, context: &str) -> Result<Vec<SubTask>, String>`
- 返回 `Vec<SubTask>` 而非 `Vec<serde_json::Value>`
**验收**: `plan()` 产出与原逻辑等价；`extract_json` 保留为 pub(crate) util
**依赖**: 1.1

### Task 1.3 — 迁移 Aggregator 阶段到 `Aggregator::aggregate`
**文件**: `src/tools/meta/rlm/aggregator.rs`（新建）
**做**:
- 将 L532-578（results_section 构造、aggregator prompt、API 调用、返回 aggregated 字符串）迁移为 `pub struct Aggregator { client: ApiClient }` + `Aggregator::aggregate(&self, task, context, results: &[Option<String>], task_errors: &[Option<String>]) -> Result<String, String>`
**验收**: `aggregate()` 输出与原逻辑等价
**依赖**: 1.1

### Task 1.4 — 迁移 Executor 阶段到 `Executor::execute`（无 replan）
**文件**: `src/tools/meta/rlm/executor.rs`（新建）
**做**:
- 将 L200-506（depth 计算、分层并行、`reserve_child` + 结构性 fallback、失败收集 L489-505）迁移为 `pub struct Executor { ... }` + `Executor::execute(...) -> Result<ExecutorOutcome, String>`
- `ExecutorOutcome { results: Vec<Option<String>>, task_errors: Vec<Option<String>>, completed: usize, failed: usize }`
- **此阶段不加 replan**，纯迁移，行为与现状完全一致
- 依赖项（settings/tool_registry/coordinator/caller/allowed_tools/timeout/budget/workdir/progress）通过构造函数或方法参数传入
**验收**: Executor 单跑行为与原 pipeline 等价
**依赖**: 1.1

### Task 1.5 — `pipeline.rs` 重构为薄编排器
**文件**: `src/tools/meta/rlm/pipeline.rs`
**做**:
- `run_rlm_pipeline` 改为：构造 `Planner`/`Executor`/`Aggregator` -> `Planner::plan` -> `Executor::execute` -> `Aggregator::aggregate` -> 组装 `RlmResult`
- **公开签名不变**（向后兼容）
- budget 分配逻辑（L187-198, L519-530 rollover）保留在编排器层（传给 Executor/Aggregator）
**验收**: `cargo test --all` 全绿（现有测试不受影响）；`cargo clippy --all-targets -- -D warnings` 零 warning；`cargo fmt --check` 通过
**依赖**: 1.2, 1.3, 1.4

### Task 1.6 — Phase 1 回归验证
**做**:
- 跑 `cargo test --all` + clippy + fmt 全绿
- 确认无 replan 时失败子任务仍标 `[ERROR]`（行为不变）
- 提交检查点 commit `refactor(rlm): extract Planner/Executor/Aggregator from monolithic pipeline`
**验收**: 全绿 + 行为不变
**依赖**: 1.5

---

## Phase 2：配置接入

### Task 2.1 — `RlmSettings` 传入 pipeline
**文件**: `src/tools/meta/rlm/pipeline.rs`、`src/config/guardian.rs`（确认 `RlmSettings` 字段）
**做**:
- `run_rlm_pipeline` 内从 `settings.rlm` 读取 `RlmSettings`（retry_enabled / max_replan_cycles / jaccard_threshold），传入 `Executor`
- 保留向后兼容默认（config 已有默认值）
**验收**: `settings.rlm.retry_enabled` 可被 Executor 访问
**依赖**: 1.6

### Task 2.2 — Subagent 覆盖解析
**文件**: `src/tools/meta/rlm/executor.rs`、`src/config/agent.rs`（确认 `SubagentRlmOverride`）
**做**:
- 当 RLM caller 是 subagent（`caller.depth > 0`），用 `SubagentRlmOverride`（若配置）覆盖 `retry_enabled`/`max_replan_cycles`；无 override 用 top-level
- 封装为 `resolve_rlm_settings(settings, caller) -> EffectiveRlmSettings`
**验收**: top-level / override / 回退三路径正确
**依赖**: 2.1

### Task 2.3 — 配置接入单测
**文件**: `src/tools/meta/rlm/executor.rs`（`#[cfg(test)]`）
**做**:
- 单测：top-level `RlmSettings` 生效；subagent override 覆盖；无 override 回退 top-level
**验收**: 3 个单测通过
**依赖**: 2.2

---

## Phase 3：Incremental Planner mode

### Task 3.1 — `Planner::replan_incremental` 方法
**文件**: `src/tools/meta/rlm/planner.rs`
**做**:
- 新增 `Planner::replan_incremental(&self, original_plan: &[SubTask], replace_ids: &[usize], failure_reasons: &[(usize, String)], partial_results: &[(usize, String)]) -> Result<Vec<ReplacementSubTask>, String>`
- `ReplacementSubTask { replaces_id: usize, sub_task: SubTask }`
- incremental prompt：输入原 plan JSON + replace_ids + 各自 failure reason + 已完成子任务 partial result；指令「仅重新分解指定 id 的子任务，产出的替换 `depends_on` 只能引用未替换（preserved）id，不得重新分解已完成子任务」
- 输出解析为 `Vec<ReplacementSubTask>`，`replaces_id` 必须在 `replace_ids` 内
**验收**: 方法可调用并返回结构化替换集
**依赖**: 1.6, 2.1

### Task 3.2 — `depends_on` 合法性校验 + 替换集排除已完成
**文件**: `src/tools/meta/rlm/planner.rs`
**做**:
- 校验：每个 replacement 的 `depends_on` 仅引用 preserved（未替换）id；非法引用的 replacement 被拒绝（过滤掉）
- 校验：替换集不含已完成子任务的 id
- 返回校验后的合法替换集；若全部非法则返回空集（Executor 据此标 `[ERROR]`）
**验收**: 非法引用被过滤；已完成子任务不被替换
**依赖**: 3.1

### Task 3.3 — Incremental Planner 单测
**做**:
- 单测：replan 输出 `depends_on` 仅引用 preserved id；替换集不含已完成子任务；非法引用被拒
**验收**: 单测通过（用 mock client 或 JSON 解析测试）
**依赖**: 3.2

---

## Phase 4：Executor replan 循环

### Task 4.1 — `ReplanQuota` + replan 决策门
**文件**: `src/tools/meta/rlm/executor.rs`
**做**:
- `struct ReplanQuota { remaining: usize }`（从 `max_replan_cycles` 初始化）
- Executor 层内失败收集后：`if retry_enabled && failures.non_empty() && quota.remaining > 0` 触发 replan
- **仅 `Ok(Err)` 计入 failures**（Q1）；Join error（`Err`）标 `[ERROR]` 但不触发 replan、不耗配额
**验收**: 决策门正确触发/不触发
**依赖**: 2.3, 3.3

### Task 4.2 — 局部 replan 范围（Executor 图分析）
**文件**: `src/tools/meta/rlm/executor.rs`
**做**:
- 实现 `transitive_dependents(failed_ids, deps) -> HashSet<usize>`：失败子任务 + 其下游依赖（transitive `depends_on` 闭包）
- `replace_ids = transitive_dependents(failures)`
- `preserved_ids = all_ids - replace_ids`
- 调用 `Planner::replan_incremental(original_plan, &replace_ids, failure_reasons, partial_results)`
**验收**: 图分析确定性；下游依赖正确识别
**依赖**: 4.1

### Task 4.3 — 早 replan + 后续层移除（Q3）
**文件**: `src/tools/meta/rlm/executor.rs`
**做**:
- 每层完成后检查失败 -> 触发 replan
- `replace_ids` 从后续层执行队列移除（`pending_remove` 集合），防止 doomed 依赖白跑
- 替换子任务按 `depends_on`（对 preserved id 已知 depth）重新 level，在 replan mini-loop 按依赖序执行
- relevel 逻辑：replacement depth = max(dep.depth for dep in depends_on) + 1，复用现有 depth 计算
**验收**: 下游依赖不白跑；替换按依赖序执行
**依赖**: 4.2

### Task 4.4 — 替换子任务执行路径
**文件**: `src/tools/meta/rlm/executor.rs`
**做**:
- 替换子任务经同一 `coordinator.reserve_child` + `finish_child` permit 路径 + 结构性 fallback 执行（复用 Task 1.4 的执行逻辑，抽为内部方法 `execute_subtask(...)`）
- 替换成功 -> `results[replaces_id] = Some(result)`，清除 `task_errors[replaces_id]`
- 替换失败 -> 保持 `[ERROR]`
- `quota.remaining -= 1`（每次 replan 递减，全局共享）
**验收**: 替换走 permit + fallback；配额递减
**依赖**: 4.3

### Task 4.5 — 配额耗尽 + 关闭 replan 路径
**做**:
- 配额耗尽：失败直接标 `[ERROR]`，不调 replanner（向后兼容）
- `retry_enabled=false`：完全跳过 replan 路径，行为同现状
**验收**: 两条路径行为正确
**依赖**: 4.4

### Task 4.6 — Replan 核心单测
**做**:
- 单测（用 mock executor/fake subtask results）：
  - 核心成功：1 失败 -> replan -> 替换成功 -> `failed=0`
  - 配额耗尽：2 次 replan 都失败 -> `[ERROR]` 聚合继续
  - 关闭 replan：`retry_enabled=false` -> 直接 `[ERROR]`
  - join error：不触发 replan、不耗配额
**验收**: 4 个场景单测通过
**依赖**: 4.5

---

## Phase 5：Replan 输出去重（Q5）

### Task 5.1 — jaccard 去重过滤
**文件**: `src/tools/meta/rlm/executor.rs`
**做**:
- import `crate::tools::meta::rlm::formats::jaccard_similarity`（确认是 pub）
- 对每个 replacement，计算其 prompt 与对应 `replaces_id` 失败子任务 prompt 的 Jaccard 相似度
- `>= settings.rlm.jaccard_threshold` 则拒绝该 replacement（过滤）
- 全部被拒 -> 该 replan 无有效替换 -> 标 `[ERROR]`，但**仍消耗配额**（防止无限重生相同失败）
**验收**: 相似替换被拒；不相似被接受
**依赖**: 4.6

### Task 5.2 — 去重单测
**做**:
- 单测：替换与失败子任务 jaccard≥阈值 -> 拒绝；< 阈值 -> 接受执行
**验收**: 单测通过
**依赖**: 5.1

---

## Phase 6：Replan 预算集成（Q4）

### Task 6.1 — 预算按需扣除 + 不足检查
**文件**: `src/tools/meta/rlm/executor.rs`、`src/tools/meta/rlm/budget.rs`
**做**:
- replanner API 调用 + 替换子任务执行从 `BudgetAllocation` executor pool 按需扣除
- 发 replanner 调用前：若 executor_remaining 不足以覆盖一次 replanner 调用（估算成本），不 replan，标 `[ERROR]`
- 替换子任务执行消耗计入 executor pool
**验收**: 预算扣除正确；不足时不 replan
**依赖**: 5.2

### Task 6.2 — 未用预算 rollover
**做**:
- 未用 replan 预算经 `rollover_unused("executor", ...)` 滚入 aggregator（复用现有语义，编排器层处理）
**验收**: rollover 行为与现有 executor rollover 一致
**依赖**: 6.1

### Task 6.3 — 预算单测
**做**:
- 单测：replan 扣 executor pool；预算不足时不 replan；剩余滚入 aggregator
**验收**: 单测通过
**依赖**: 6.2

---

## Phase 7：P2-2 验证 + 文档

### Task 7.1 — P2-2 permit 路径测试覆盖
**文件**: `src/agent/coordinator.rs`（测试）或 `src/tools/meta/rlm/executor.rs`（测试）
**做**:
- 补测试：RLM 子任务执行时 `max_concurrent` permit 生效（`reserve_child` 获取 semaphore）
- structural fallback 仅 permit 耗尽时触发
**验收**: 测试覆盖 permit 路径 + fallback 触发条件
**依赖**: 1.6

### Task 7.2 — 更新 `WGENTY.md`
**文件**: `WGENTY.md`
**做**:
- 标注 P2-2（统一并行编排路径）已由现有 `reserve_child`/`finish_child` permit 设计满足
- structural fallback 为 permit 饱和时设计内降级（镜像 `task` 工具）
- 更新 RLM 相关架构描述（如有 replan 配置项说明）
**验收**: 文档准确反映现状
**依赖**: 7.1

---

## Phase 8：不变量与回归

### Task 8.1 — 不变量检查
**做**:
- `rg "comet" src/exec_session/` 仅匹配 `SessionSource::Comet` 枚举变体与注释
- 确认 RLM 不写 `session.json`（replan 不引入新持久化）
**验收**: 不变量保持
**依赖**: 6.3, 7.2

### Task 8.2 — 全量回归
**做**:
- `cargo test --all` 全绿
- `cargo clippy --all-targets -- -D warnings` 零 warning
- `cargo fmt --check` 通过
- 非 git 项目降级：config 默认值向后兼容，无新配置项
**验收**: 三命令全绿 + 向后兼容
**依赖**: 8.1

### Task 8.3 — 最终提交
**做**:
- 提交 `feat(rlm): add task-level replan ability with incremental Planner`
- 确认 tasks.md 全部勾选
**验收**: 提交完成 + tasks.md 同步
**依赖**: 8.2

---

## 风险与缓解

| 风险 | 缓解 |
|------|------|
| replan mini-loop re-level 逻辑复杂 | Phase 4 单测覆盖依赖序；先实现简单串行再优化并行 |
| replanner 产出非法 `depends_on` | Task 3.2 强制校验 + 拒绝非法引用 |
| max_replan_cycles 高失败率快速耗尽 | 可接受（符合"防止无限 replan"）；文档说明 |
| 模块提取引入行为回归 | Phase 1 独立验证全绿作为安全检查点；公开签名不变 |
| jaccard_similarity 非 pub | Task 5.1 确认/调整为 pub(crate) |
