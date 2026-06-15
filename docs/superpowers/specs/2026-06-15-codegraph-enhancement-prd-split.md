# PRD 拆分清单 — 增强项目代码理解能力

> 用户原始请求：**「如何增强这个项目的代码理解能力」**
> PRD 拆分确认日期：2026-06-15
> 拆分确认人：用户（通过 `/comet-open` 阶段 1a PRD 拆分预检）

## 已确认的拆分方案：4 个 change

> 4 个 change 全部走 `full` workflow（必经 brainstorming）。
> 拆分目的：避免 1 个超大 change 跨越 3-5 周难以归档；按风险从低到高分阶段交付。

| 编号 | change name                            | 类型   | 工期估算 | 依赖           | 状态                           |
| ---- | -------------------------------------- | ------ | -------- | -------------- | ------------------------------ |
| #0   | `codegraph-baseline-spike`             | spike  | 2-3 天   | 无             | **active** — 2026-06-15 创建   |
| #1   | `codegraph-agent-adoption`             | full   | 2-4 天   | #0 报告        | 待创建（#0 archive 后启动）    |
| #2   | `codegraph-query-and-explainability`   | full   | ~1 周    | #0 报告        | 待创建（#1 archive 后启动）    |
| #3   | `codegraph-multilang-and-deep-graph`   | full   | 2-3 周   | #0 报告 + 部分 #2 输出格式 | 待创建（#2 archive 后启动） |

## 各 change 摘要

### #0 codegraph-baseline-spike（active）

**类型**：spike（新增 capability `codegraph-baseline-bench`）
**目的**：建立量化基线 + 验证 Agent 不用 codegraph 的根因，作为后续 3 个 change 的输入。
**详情**：参见 `openspec/changes/codegraph-baseline-spike/`

### #1 codegraph-agent-adoption

**目的**：让 Agent 在合适场景主动用 codegraph，而非默认 grep。
**预计范围**：
- 修改 `src/prompts/base.md`：调整代码搜索章节顺序与措辞
- 修改 `src/tools/codegraph/tools.rs`：强化 description（"PREFER FOR symbol/call relationships"）
- 可能新增 collaboration prompt 中的"代码导航 playbook"
- 可能调整 lazy-init 失败信息

**预计 capability 影响**：
- 修改 `codegraph-mcp`、`codegraph-lazy-init`（视具体修改而定）
- 可能新增 `codegraph-agent-guidance` capability

**验收阈值**：codegraph 调用率比 #0 报告记录的基线提升 ≥ X%（X 由 #0 报告确定）

### #2 codegraph-query-and-explainability

**目的**：增强查询能力 + 加可解释性（调用路径树 + 审计日志）。
**预计范围**：
- 修改 `src/tools/codegraph/query.rs`：调用路径树、模糊匹配/语义检索、过滤排序
- 新增 `.codegraph/audit.log`
- 修改 `src/tools/codegraph/tools.rs`：输出格式增加 source/confidence/audit_id

**预计 capability 影响**：
- 修改 `symbol-query`、`call-graph`
- 新增 `codegraph-explainability` capability
- 可能新增 `codegraph-advanced-query` capability

**验收阈值**：调用路径树支持深度 ≤ 5、每跳显示证据；审计日志可复现查询；至少 3 种新查询模式

### #3 codegraph-multilang-and-deep-graph

**目的**：tree-sitter 多语言（Rust + Java + Python，每种深做） + 深 Symbol Graph。
**预计范围**：
- `Cargo.toml`：新增 `tree-sitter-java`、`tree-sitter-python`
- `src/tools/codegraph/parser.rs`：多语言 parser pool
- `src/tools/codegraph/indexer.rs`：抽离 LanguageAdapter trait + 三种语言适配器
- `src/tools/codegraph/types.rs`：新增关系类型（Inherits、TypeOf、Returns、Parameter）
- `src/tools/codegraph/store.rs`：schema 演进 + 迁移

**预计 capability 影响**：
- 修改 `code-indexing`
- 新增 `multilang-indexing`、`symbol-graph-deep` capability

**验收阈值**：在 Rust/Java/Python 样例项目上索引时间 ≤ 基线 × N（N 由 #0 报告确定），新关系类型至少 4 种

## 未拆入本批次（明确 out-of-scope）

- 跨仓库 / monorepo 多 root 索引
- LLM 嵌入向量语义检索
- LSP / 真正的类型检查器集成
- call graph 可视化 GUI
- Rust/Java/Python 之外的语言

## 拆分原则与红线

1. **逐个 active**：同一时间只让 1 个 change 处于 active 状态（design/build/verify）。当前 active 是 #0；#0 archive 后才创建 #1。
2. **4 个都走 full workflow**：包括 spike，brainstorming 不可跳过。
3. **每个 change 独立闭环**：proposal → design → build → verify → archive 全走完才开下一个。
4. **#0 报告是后续 change 的"目标值之源"**：#1/#2/#3 的 design 必须引用 #0 报告中的具体数字。

## 恢复指引

如果对话上下文丢失：

1. 检查 `openspec list --json` 看哪个 change 是 active
2. 读本文件确认整体拆分结构
3. 通过 `/comet` 进入 active change 继续
4. 当前 active change archive 后，按本表第二列顺序创建下一个：`/comet 创建 <next-change-name>`

## 修订记录

- 2026-06-15：初始版本，4 个 change 拆分确认；#0 codegraph-baseline-spike 创建并完成 open 阶段
