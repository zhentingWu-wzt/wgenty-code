## Why

#0 codegraph-baseline-spike 已建立量化基线：codegraph 工具调用率仅 0.05%（1/1959），session 采纳率 1.4%（1/71），codegraph_explore 从未被使用。根因分析（`scripts/codegraph-bench/root-cause-analysis.md`）确定 top 3 根因：

1. **System Prompt 中 grep 被列为代码搜索首选**（`src/prompts/base.md:117-119, 141-153`），codegraph 工具完全不在工具列表和「When to use each tool」对比表中。Agent 不知道 codegraph 存在
2. **工具描述缺乏场景引导**（`src/tools/codegraph/tools.rs:61-63, 175-177`），description 是功能导向（"what it does"），没有"何时优先用"的对比性引导
3. **Lazy-init 反馈缺失**（`src/tools/codegraph/tools.rs:10-34`），初始化无成功信号、错误信息无操作建议

本 change 针对这三个根因从 prompt 层、工具描述层、错误反馈层同步修复，让 Agent 在合适场景主动使用 codegraph 替代 grep。

## What Changes

- 调整 `src/prompts/base.md`：
  - 在「Search」工具段落加入 `codegraph_node` 和 `codegraph_explore`，排在 `grep` 之前
  - 在「When to use each tool」对比表中将「Find where a function is defined」、「Find callers of a function」、「Find implementations of a trait」等场景的推荐工具改为 codegraph，grep 降为兜底
- 调整 `src/prompts/`（位置由 design 阶段决定）：新增「代码导航 playbook」段落，明确 codegraph→grep→file_read 的标准工作流
- 修改 `src/tools/codegraph/tools.rs`：
  - 强化 `codegraph_node` description，加入 "PREFER FOR symbol definitions, callers, references"
  - 强化 `codegraph_explore` description，加入场景化引导（模块结构、调用图浏览）
  - 优化 lazy-init 错误文案：明确告知如何通过 `wgenty-code codegraph index` 修复
- 新增 `scripts/codegraph-bench/bench-agent-replay.sh`：在新 prompt 上回放 14 条标准导航任务，输出工具调用分布 + 分层统计 JSON 报告
- 在 wgenty-code 自身仓库验证分层阈值（强项类 ≥60%、其他类 ≥25%）

## Capabilities

### New Capabilities

无。本次 change 修改既有 capability 的 spec 行为，不引入新 capability。

### Modified Capabilities

- `symbol-query`：`codegraph_node` 工具的 description 字段从纯功能描述变更为含场景引导（"PREFER FOR..."）；spec 验收场景需补充「工具描述包含场景引导」要求。
- `call-graph`：`codegraph_explore` 工具的 description 字段同上；spec 需补充场景引导验收。
- `codegraph-lazy-init`：lazy-init 错误信息从泛化提示变更为明确的可操作建议（包含具体修复命令）；spec 需补充错误反馈格式要求。

## Impact

- **修改文件**：
  - `src/prompts/base.md`（codegraph 加入工具列表 + 对比表）
  - `src/prompts/` 下其他 prompt 文件（位置 design 决定，新增代码导航 playbook）
  - `src/tools/codegraph/tools.rs`（强化 description + 错误文案）
  - `openspec/specs/symbol-query/spec.md`、`openspec/specs/call-graph/spec.md`、`openspec/specs/codegraph-lazy-init/spec.md`（修改场景描述）
- **新增文件**：`scripts/codegraph-bench/bench-agent-replay.sh`（回归测试脚本）
- **不修改**：codegraph 索引引擎、查询逻辑（`src/tools/codegraph/{indexer,query,store,parser}.rs`）；TUI 显示；MCP 协议层
- **不引入新依赖**
- **运行影响**：Agent 行为发生显式变化（更多调用 codegraph_node/codegraph_explore）；对仅使用现有 grep/file_read 工作流的 session 无影响（这些工具仍可用，只是优先级降低）
- **验收数据来源**：#0 baseline 报告（`openspec/changes/archive/2026-06-15-codegraph-baseline-spike/`）和 14 条标准任务集（`scripts/codegraph-bench/agent-tasks/nav-001~014.yaml`）
- **下游 change 依赖**：`codegraph-query-and-explainability` (#2)、`codegraph-multilang-and-deep-graph` (#3) 在各自 design 阶段会引用本 change 的采纳率提升结果作为查询能力 / 多语言改进的需求驱动证据
- **风险**：中
  - prompt 修改可能影响其他类型任务的工具选择行为（缓解：S2 验收场景要求不破坏现有功能）
  - codegraph 索引未建时新错误文案需精准（缓解：错误文案修改仅文案，不改架构）
  - 14 条任务集代表性有限（缓解：分层阈值已宽松到可达，本 change 验收以任务集为准；真实使用监控留给后续 change）
