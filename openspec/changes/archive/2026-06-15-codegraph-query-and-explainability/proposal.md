## Why

#1 codegraph-agent-adoption 已通过 prompt + tool description + error message 三层修复，让 Agent 在代码导航任务中主动选用 codegraph 替代 grep。但 Agent 即使选择 codegraph，当前的查询结果仍缺乏可信度和可追溯性：没有置信度标注（Agent 无法判断结果可靠性）、没有调用路径证据（Agent 只能看到一跳 callers/callees）、没有审计日志（无法复现和调试查询）。同时查询入口仅 codegraph_node 和 codegraph_explore 两种，缺少批量查询、路径搜索、模块概要等常用模式。

本 change 从查询能力增强和可解释性两个维度改进 codegraph，让 Agent 不仅能找到符号、还能理解为什么找到、结果有多可靠、以及如何复现。

## What Changes

- **可解释性 — 调用路径树**：`codegraph_explore` 输出格式从扁平列表改为多跳调用路径树（深度 ≤5），每跳显示证据（file:line + RelKind），让 Agent 能追踪完整调用链
- **可解释性 — 审计日志**：新增 `.codegraph/audit.log`（结构化 JSONL，append-only），记录每次查询的时间/类型/参数/结果摘要/audit_id。Agent 和用户可通过 jq 复现查询
- **可解释性 — 置信度与来源**：`codegraph_node` 和 `codegraph_explore` 输出增加 `confidence` 字段（High=treesitter-AST 直接解析，Medium=文本匹配，Low=推断，Unresolved=找不到）和 `source` 字段，利用已有 Confidence 枚举
- **查询增强 — 模糊匹配**：`codegraph_node` 在精确匹配无结果时自动 Levenshtein ≤3 模糊补全（与现有 spec「Symbol not found → suggestions」场景一致）
- **查询增强 — 过滤排序**：输出支持按 confidence/relevance 排序和按 name/kind 过滤
- **新查询模式 — call_path**：新增两点间最短调用路径查询（符号 A → B 路径，Dijkstra 图搜索）
- **新查询模式 — symbol_batch**：一次查询多个符号名，聚合结果
- **新查询模式 — module_summary**：模块概要（符号列表、导出函数、依赖关系）
- **现有能力修改**：`symbol-query`（增加 confidence/source/fuzzy）、`call-graph`（增加 call_path/path_tree）

## Capabilities

### New Capabilities

- `codegraph-explainability`：codegraph 查询结果的可解释性保障，包括结构化审计日志（JSONL，可复现）、调用路径树的逐跳证据标注、输出中 confidence/source 字段。该 capability 规定审计日志的格式约束、路径树深度和证据要求、confidence 的分级来源策略。
- `codegraph-advanced-query`：codegraph 的增强查询模式，包括 call_path（两点间最短路径）、symbol_batch（批量查询）、module_summary（模块概要），以及模糊匹配、过滤排序。该 capability 规定新查询模式的输入输出格式、模糊匹配的算法约束（Levenshtein ≤3）、过滤排序的支持字段。

### Modified Capabilities

- `symbol-query`：`codegraph_node` 输出增加 `confidence` 和 `source` 字段；增加 fuzzy 匹配行为（精确未命中 → Levenshtein ≤3 建议）；增加 `sort_by` 和 `filter` 参数
- `call-graph`：`codegraph_explore` 输出格式从扁平列表改为多跳路径树（深度 ≤5）；增加 `call_path` 查询模式（两点间最短路径）；增加逐跳证据标注（file:line + RelKind）

## Impact

- **修改文件**：
  - `src/tools/codegraph/query.rs`：调用路径树算法 + 模糊匹配 + 过滤排序 + 新查询模式
  - `src/tools/codegraph/types.rs`：可能新增 CallPath / ModuleSummary 类型；现有 Confidence 枚举增强
  - `src/tools/codegraph/store.rs`：可能新增图遍历查询（Dijkstra）所需的 SQL 查询
  - `src/tools/codegraph/tools.rs`：新增 call_path / symbol_batch / module_summary 三个工具；修改 codegraph_node / codegraph_explore 的 input/output schema
  - `src/tools/codegraph/indexer.rs`：可能需要增强索引时的置信度标记（tree-sitter 解析源记录）
  - 新增 `.codegraph/audit.log`：结构化审计日志
  - `openspec/specs/symbol-query/spec.md`、`openspec/specs/call-graph/spec.md`：修改场景
- **新增文件**：
  - `src/tools/codegraph/audit.rs`：审计日志写入模块
  - `src/tools/codegraph/call_path.rs`：两点间路径搜索
- **不修改**：`src/prompts/`（#1 已完成）；TUI 显示；MCP 协议层；索引引擎核心
- **新增依赖**：可能引入 `levenshtein` crate（模糊匹配）；审计日志无外部依赖（纯 `std::fs` append）
- **运行影响**：审计日志自然增长（每次查询追加 ~200 字节）；图搜索（call_path）首次查询可能较慢（需全量加载关系），后续缓存
- **验收数据来源**：bench-agent-replay.sh（#1 产物）重跑验证 codegraph 调用率不降；#0 基线报告中性能数据作为对比基线
- **下游依赖**：`codegraph-multilang-and-deep-graph` (#3) 在扩展多语言索引时，会依赖本 change 的调用路径树和审计日志基础；新查询模式也会在多语言场景中直接获益
- **风险**：中
  - 调用路径树图搜索可能引入 O(n²) 复杂度（缓解：深度限制 ≤5、首次缓存、最大节点限制）
  - 审计日志文件大小自然增长（缓解：JSONL 压缩性高、可设置 rotate 上限，归档旧 log）
  - 新增 3 个 tool 可能让 Agent 工具选择更复杂（缓解：#1 的 playbook 已提供优先级指导）
