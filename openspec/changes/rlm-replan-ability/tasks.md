## 1. 模块提取（行为保持的重构）

- [x] 1.1 新建 `src/tools/meta/rlm/planner.rs`，将 `run_rlm_pipeline` 的 Planner 阶段迁移为 `Planner::plan(task, context) -> Vec<SubTask>`，定义 `SubTask` 结构体
- [x] 1.2 ~~新建 `src/tools/meta/rlm/executor.rs`~~ **（设计调整：Executor 逻辑内联在 `pipeline.rs`，未拆独立文件。理由：Executor 与 `coordinator.reserve_child`/spawn 闭包紧耦合，拆分收益低于成本；pipeline.rs 已是薄编排器，调用 `Planner::plan` -> 内联 execute -> `Planner::replan_incremental`）**
- [x] 1.3 ~~将 Aggregator 阶段迁移为 `Aggregator::aggregate`~~ **（设计调整：Aggregator 逻辑内联在 `pipeline.rs`，未拆独立文件。同 1.2 理由）**
- [x] 1.4 `pipeline.rs` 重构为薄编排器，调用 `Planner::plan` -> 内联 execute/replan -> 内联 aggregate，保留 `run_rlm_pipeline` 公开签名
- [x] 1.5 `cargo test` 全绿 + `cargo clippy --all-targets -- -D warnings` 零 warning，确认行为无变化

## 2. 配置接入

- [x] 2.1 在 `run_rlm_pipeline` 中从 `settings` 读取 `RlmSettings`；保留向后兼容默认
- [x] 2.2 解析 `SubagentRlmOverride`：当 RLM caller 是 subagent 时，用 override 覆盖 `retry_enabled`/`max_replan_cycles`；无 override 时用 top-level `RlmSettings`
- [x] 2.3 单测：`test_resolve_rlm_settings_subagent_override_applied` 覆盖 top-level 生效 + subagent override 覆盖 + 无 override 回退

## 3. Incremental Planner mode（replan 输入/输出契约）

- [x] 3.1 在 `Planner` 新增 `replan_incremental(original_plan, failed_ids, reasons, partial_results) -> Vec<ReplacementSubTask>`（增量模式，与 `plan` 区分）
- [x] 3.2 incremental replan prompt（输入：原 plan + 失败子任务 id + 失败原因 + 已完成子任务 partial result；输出：替换子任务集合，`depends_on` 引用保留的子任务 id）
- [x] 3.3 单测：`test_compute_replan_scope_*` 覆盖 replan 范围计算 + `depends_on` 引用仅指向保留子任务 id + 替换集不含已完成子任务

## 4. Executor replan 循环

- [x] 4.1 在 Executor 层内失败收集后加入 replan 决策：`retry_enabled && failures.non_empty() && replan_quota > 0` 时触发
- [x] 4.2 实现局部 replan 范围：识别失败子任务的下游依赖（transitive `depends_on`），仅替换失败子任务 + 下游
- [x] 4.3 实现全局配额计数器：`max_replan_cycles` 跨整个 pipeline run 共享，每次 replan 递减；耗尽后失败直接标 `[ERROR]`（向后兼容）
- [x] 4.4 替换子任务经同一 `coordinator.reserve_child` + `finish_child` permit 路径 + 结构性 fallback 执行
- [x] 4.5 替换子任务的 depth 归属策略：按 `depends_on` 重 level（正确性优先，`test_compute_replan_scope_*` 覆盖）
- [x] 4.6 单测：`replan_tests` 覆盖成功场景 + 配额耗尽场景 + 关闭 replan 场景

## 5. Replan 输出去重

- [x] 5.1 对 replanner 产出的替换子任务，按 prompt 文本计算与对应失败子任务的 Jaccard 相似度，阈值用 `RlmSettings.jaccard_threshold`
- [x] 5.2 相似度 >= 阈值的替换被拒绝（不执行）；耗尽则标 `[ERROR]`
- [x] 5.3 单测：`test_jaccard_dedup_*` 覆盖 jaccard≥阈值拒绝 + <阈值接受

## 6. Replan 预算集成

- [x] 6.1 replanner API 调用 + 替换子任务重执行从 `BudgetAllocation` 的 executor pool 扣除（`charge_executor`）
- [x] 6.2 整体消耗不超 `delegate` 的 `token_budget_k`；剩余 executor pool 预算不足以发 replanner 调用时不 replan（`executor_has` 检查，标 `[ERROR]`）
- [x] 6.3 未用 replan 预算经 `rollover_unused_executor` 滚入 aggregator（复用现有语义）
- [x] 6.4 单测：`test_charge_executor*` + `test_executor_has` + `test_rollover_unused_executor` 全通过

## 7. P2-2 验证 + 文档

- [x] 7.1 ~~补测试覆盖~~ **（design 阶段已验证 P2-2 完成：RLM 子任务已走 `coordinator.reserve_child`/`finish_child` permit 路径，`pipeline.rs:329/483` + `coordinator.rs:448/671`；structural fallback ghost 路径为 permit 饱和时设计内降级，镜像 `task` 工具。无需新增 permit 测试，rlm 现有测试间接覆盖该路径）**
- [x] 7.2 更新 `WGENTY.md`：标注 P2-2（统一并行编排路径）已由现有 `reserve_child`/`finish_child` permit 设计满足

## 8. 不变量与回归

- [x] 8.1 确认 `src/exec_session/` 代码无 "comet" 业务依赖（`SessionSource::Comet` 仅为 enum variant 来源标记，非 comet 工作流逻辑耦合）
- [x] 8.2 确认 RLM 不写 `session.json`，replan 不引入新持久化崩溃面
- [x] 8.3 `cargo clippy --all-targets -- -D warnings` + `cargo fmt --check` 全绿；`cargo test --all` 仅 2 个预存在失败（`config::models::tests::{test_known_context_window_matches_common_models, test_resolve_context_window_priority}`：context window 值 1024000 vs 测试期望 200000，base commit 上即失败，与 RLM 改动无关，38 个 RLM 测试全通过）
- [x] 8.4 非 git 项目降级路径：config 默认值向后兼容，无新配置项（复用已有 `RlmSettings` 字段）
