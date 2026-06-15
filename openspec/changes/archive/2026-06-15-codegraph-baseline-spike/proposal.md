## Why

wgenty-code 已经具备一套基于 tree-sitter 的 Rust codegraph（索引、符号查询、调用图、MCP 暴露），但用户反馈三个核心痛点：Agent 不主动用 codegraph、索引覆盖/速度不足、查询结果不够好用、缺乏可解释性。后续计划通过三个独立 change（`codegraph-agent-adoption`、`codegraph-query-and-explainability`、`codegraph-multilang-and-deep-graph`）逐层改进，但当前**没有任何量化基线**——既不知道现在的索引耗时/体积/查询延迟，也不知道 Agent 在代码导航任务中实际有多少比例用了 codegraph，更不知道"不用"的真实根因是什么。在没有基线的前提下立项三个 change，会让"提升速度"、"提升使用率"沦为模糊目标，验收阈值无据可依，归档时无法回看 ROI。

本 change 是后续 3 个 change 的**前置 spike**：用最小成本（不动业务源码、不改 prompt、不改 spec）拿到一组可信的量化数据 + 一份"Agent 不用 codegraph 的根因 top 3"，作为后续 change 的输入和验收阈值来源。

## What Changes

- 新增基线测量套件 `scripts/codegraph-bench/`：性能、覆盖率、Agent 使用率三组测量脚本（shell 或最小 Rust 二进制，独立目录与业务源码隔离）
- 新增基线报告 `docs/superpowers/specs/<date>-codegraph-baseline-report.md`，至少包含：性能基线、覆盖率基线、Agent 使用率基线、"不用 codegraph"根因 top 3、后续 3 个 change 的"基线值 vs 建议目标值"表
- 在 wgenty-code 自身仓库 + 至少 1 个外部 Rust 项目（如 ripgrep 或同等规模 crate）上跑通测量脚本，验证脚本不绑死本仓库
- 测量脚本可重复执行（`bash scripts/codegraph-bench/run-all.sh` 或等价入口），后续 change 验收时可重跑对比

## Capabilities

### New Capabilities

- `codegraph-baseline-bench`：可重复执行的 codegraph 基线测量套件 + 量化报告。该 capability 规定测量脚本的入口、覆盖范围（性能、覆盖率、Agent 使用率）、测量产物（基线报告必含字段）、可重复性约束（同一仓库重跑结果可比对）以及外部仓库适配性。

### Modified Capabilities

无。本 change 不修改任何现有 codegraph 相关 capability 的 spec 行为；`code-indexing`、`symbol-query`、`call-graph`、`codegraph-mcp`、`codegraph-lazy-init` 在测量过程中只作为"被观测对象"，requirements 不变 — 它们的修改属于后续 3 个 change。

## Impact

- **新增文件**：`scripts/codegraph-bench/`（测量脚本）、`docs/superpowers/specs/<date>-codegraph-baseline-report.md`（报告）
- **不修改的代码**：`src/` 下所有源码、`src/prompts/` 下所有 prompt、`openspec/specs/` 下所有 capability spec
- **可能修改的依赖**：仅当测量必需时引入轻量依赖（如 hyperfine、jq、time），优先用系统自带工具
- **运行影响**：测量过程会在 `.codegraph/` 下生成临时索引；测量完成后清理或保留视脚本设计而定（在 design 阶段决定）
- **下游 change 依赖**：`codegraph-agent-adoption`、`codegraph-query-and-explainability`、`codegraph-multilang-and-deep-graph` 三个 change 在各自 design 阶段会引用本 spike 报告作为基线和目标值来源
- **风险**：低 — 不修改业务代码；最坏情况下报告产出延迟，但不阻塞当前 codegraph 的使用
