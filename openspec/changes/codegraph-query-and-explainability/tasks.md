# Tasks — codegraph-query-and-explainability

> 每完成一个 task 必须立即勾选并 git commit；message 体现设计意图。

## 1. 审计日志基础设施

- [ ] 1.1 新增 `src/tools/codegraph/audit.rs`：实现 AuditLogger 结构体 + `log_query()` 方法（JSONL append + audit_id 生成）
- [ ] 1.2 在 QueryEngine 中注入 AuditLogger，所有查询路径（node/explore/call_path/symbol_batch/module_summary）调用后写入日志
- [ ] 1.3 在 codegraph_node/explore 输出 JSON 中追加 `audit_id` 字段
- [ ] 1.4 commit：`feat(codegraph): audit logger — JSONL append-only with UUID audit_id`

## 2. Confidence + Source 标注

- [ ] 2.1 修改 `src/tools/codegraph/types.rs`：Confidence 枚举增加 `source` 字段和映射函数 `from_parse_source()`
- [ ] 2.2 修改 QueryEngine 的 node/explore 方法：填充返回结果中每个 symbol 的 `confidence` 和 `source`
- [ ] 2.3 修改 codegraph_node/explore 的 output JSON schema：增加 `confidence`（"high"/"medium"/"low"/"unresolved"）和 `source` 字段
- [ ] 2.4 commit：`feat(codegraph): confidence + source annotation on query results`

## 3. 调用路径树

- [ ] 3.1 新增 `src/tools/codegraph/call_path.rs`：实现 graph 构建（从 IndexStore 加载全量 calls）+ BFS 遍历（深度 ≤5，记录 file:line + RelKind）
- [ ] 3.2 修改 codegraph_explore 输出格式：当查询涉及 calls 关系时，附加 `call_paths` 字段（路径树 JSON）
- [ ] 3.3 commit：`feat(codegraph): multi-hop call path tree with evidence (depth ≤5)`

## 4. 模糊匹配

- [ ] 4.1 实现 `levenshtein_distance()` 函数（≤ `src/tools/codegraph/query.rs` 或独立 `fuzzy.rs`）
- [ ] 4.2 修改 codegraph_node：精确匹配无结果时，从索引中取候选，按距离排序返回 top 5（≤3 跳 + length_diff ≤50%）
- [ ] 4.3 编写单元测试覆盖 Unicode/空字符串/极长符号
- [ ] 4.4 commit：`feat(codegraph): fuzzy symbol matching with Levenshtein ≤3`

## 5. 过滤与排序

- [ ] 5.1 修改 codegraph_node/explore 参数 schema：增加 `sort_by`（confidence/name）和 `filter`（kind/name_prefix）
- [ ] 5.2 实现排序逻辑（confidence 降序 + name 升序）和过滤逻辑
- [ ] 5.3 commit：`feat(codegraph): filter and sort support for query results`

## 6. 新查询模式 — call_path

- [ ] 6.1 新增 `CallPathTool` struct（实现 Tool trait）：`from` + `to` symbol 输入，输出最短路径 hops[]
- [ ] 6.2 实现 Dijkstra 最短路径搜索（基于 memory graph from call_path.rs）
- [ ] 6.3 无路径场景输出 `{"path_found": false, "reason": "..."}`
- [ ] 6.4 commit：`feat(codegraph): call_path tool — shortest path between two symbols`

## 7. 新查询模式 — symbol_batch

- [ ] 7.1 新增 `SymbolBatchTool` struct：输入 `symbols[]`（max 10），输出每个 symbol 的 node 结果数组
- [ ] 7.2 聚合逻辑复用 QueryEngine::codegraph_node()
- [ ] 7.3 commit：`feat(codegraph): symbol_batch tool — batch symbol lookup`

## 8. 新查询模式 — module_summary

- [ ] 8.1 新增 `ModuleSummaryTool` struct：输入 `module_path`，输出该模块下所有符号列表 + 导出函数 + 依赖关系
- [ ] 8.2 实现基于 IndexStore 的模块过滤查询（SQLite `WHERE file_path LIKE 'module_path%'`）
- [ ] 8.3 commit：`feat(codegraph): module_summary tool — module-level overview`

## 9. Tool 注册与集��

- [ ] 9.1 在 tool registry 中注册 call_path / symbol_batch / module_summary 三个新 tool
- [ ] 9.2 修改所有 5 个 codegraph tool 的 `description()` 为新格式（PREFER FOR/AVOID WHEN，与 #1 保持一致）
- [ ] 9.3 commit：`feat(codegraph): register call_path/symbol_batch/module_summary tools`

## 10. Spec 同步

- [ ] 10.1 创建 delta specs：`specs/codegraph-explainability/spec.md`（审计日志 + confidence + 调用路径树证据）
- [ ] 10.2 创建 delta specs：`specs/codegraph-advanced-query/spec.md`（call_path + symbol_batch + module_summary + fuzzy + filter-sort）
- [ ] 10.3 修改 delta specs：`specs/symbol-query/spec.md`（confidence/source + fuzzy）
- [ ] 10.4 修改 delta specs：`specs/call-graph/spec.md`（call_path + 路径树）

## 11. 验收与回归

- [ ] 11.1 跑 cargo build + cargo test 无回归
- [ ] 11.2 跑 bench-agent-replay.sh 验证 codegraph 调用率不降（≥ #1 存档水平）
- [ ] 11.3 手动验证 audit.log 文件正常写入（jq 可读）
- [ ] 11.4 手动验证 call_path 两点间路径（选取 3 对已知调用关系的符号）
- [ ] 11.5 手动验证 fuzzy 匹配（输入近似符号名观察 top 5 建议）
- [ ] 11.6 手动验证 symbol_batch 批量和 module_summary 输出正确

## 12. 验证与归档

- [ ] 12.1 运行 `openspec validate codegraph-query-and-explainability` 校验
- [ ] 12.2 进入 `/comet-verify`，按 spec scenarios 逐项核对
- [ ] 12.3 verify 通过后进入 `/comet-archive`
