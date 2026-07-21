# Brainstorm Summary

- Change: rlm-replan-ability
- Date: 2026-07-20

## 确认的技术方案

**模块结构**：从单体 `run_rlm_pipeline`（`src/tools/meta/rlm/pipeline.rs`，~630 行）提取 `Planner`/`Executor`/`Aggregator` 到 `planner.rs`/`executor.rs`（`aggregator.rs`）；pipeline 成薄编排器。行为保持的重构。

**Replan 数据流**（5 决议）：
1. **触发（Q1-A）**：仅 `Ok(Err)`（子代理 loop 失败，含 structural-fallback-then-fail）触发 replan。Join error（panic/cancel）视为基础设施故障，标 `[ERROR]` 不 replan。
2. **范围+契约（Q2-A）**：Executor 做图分析（transitive `depends_on`）确定「失败子任务 + 下游依赖」id 集；Planner 增量模式输入 = 原 plan + 失败 id + 原因 + 已完成子任务 partial result，输出 = 显式替换映射 `{replacements:[{replaces_id, prompt, use_small_model, depends_on}]}`，`depends_on` 仅引用保留 id。
3. **时机+depth（Q3-A）**：每层完成后检查失败，早 replan；下游依赖（未跑）从后续层移除；替换子任务按 `depends_on`（对保留 id 已知 depth）重新 level，在 replan mini-loop 按依赖序跑。防止 doomed 依赖白跑。
4. **预算（Q4-B）**：replanner 调用 + 替换执行按需从 executor pool 扣；不足发 replanner 时不 replan（标 `[ERROR]`）；未用经 `rollover_unused` 滚入 aggregator。
5. **去重（Q5-A）**：replan 去重复用 `formats.rs` 的 `jaccard_similarity`，阈值用 `settings.rlm.jaccard_threshold`（首次活用）；`formats.rs` 死代码 `Aggregator` 原样不动（out of scope）。

**配置接入**：`RlmSettings`（retry_enabled 门控 / max_replan_cycles 全局配额 / jaccard_threshold）+ `SubagentRlmOverride`（subagent 覆盖）。无新配置项，向后兼容。

**P2-2 验证**：确认 `reserve_child`/`finish_child` permit 路径已覆盖 RLM + 补测试 + 更新 WGENTY.md 标注 P2-2 已完成。

## 关键取舍与风险

- **取舍**：Executor 拥有依赖图（不信任 LLM 算依赖）vs Planner 自决--选前者，确定性强；早 replan 防白跑 vs 晚 replan 简单--选早 replan，正确性优先；按需扣预算 vs 预留子池--选按需，不惩罚常见无 replan 情况。
- **风险**：replan mini-loop 的 re-level 逻辑复杂度（需严格测试依赖序）；replanner prompt 增量模式可能产出无效 `depends_on`（需校验引用合法性，拒绝非法引用）；max_replan_cycles 全局配额在高失败率场景下快速耗尽（可接受，符合「防止无限 replan」）。
- **不变式延续**：`src/exec_session/` 无 "comet"；RLM 不写 session.json，replan 不引入新持久化崩溃面。

## 测试策略

- 单测覆盖：模块提取后行为不变（无 replan，失败仍 `[ERROR]`）；config 接入（top-level/override/回退）；incremental Planner 输出 `depends_on` 合法性 + 替换集不含已完成子任务；replan 核心成功（1 失败 -> 替换成功 -> failed=0）/ 配额耗尽 / 关闭 replan；jaccard 去重接受/拒绝；预算扣除/不足不 replan/rollover；P2-2 permit 生效 + structural fallback 仅 permit 耗尽时触发。
- 不变量回归：`cargo test --all` + `cargo clippy --all-targets -- -D warnings` + `cargo fmt --check` 全绿；`exec_session/` 无 "comet"。

## Spec Patch

- `specs/rlm-replan/spec.md`：
  - 「Replan gated by configuration」加 scenario：join error（panic/cancel）不触发 replan，标 `[ERROR]`。
  - 「Local replan scope」加 scenario：Executor（非 Planner）通过 transitive `depends_on` 图分析确定下游依赖集。
- `specs/rlm-budget-control/spec.md`：无 patch（Q4-B 已与现有 ADDED requirement 一致）。
