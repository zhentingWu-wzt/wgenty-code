## Context

#0 和 #1 已完成。当前 codegraph 的查询能力仅限于两种模式：
- `codegraph_node(symbol)`：精确匹配单符号，返回定义/签名/引用/callers-calers
- `codegraph_explore(query)`：模糊探索，返回相关符号和关系

两者输出均缺乏 confidence/source 字段、无审计追踪、无可解释的调用路径证据。Agent 即使选择了 codegraph，也无法判断结果可靠性或追溯查询过程。用户原始需求中的"可解释性 = 调用路径树 + 审计日志"在本 change 中实现。

现状代码关键事实：
- `query.rs` 已有 QueryEngine（node/exlore 方法）
- `types.rs` 已有 Confidence 枚举（High/Medium/Low/Unresolved）但实际产出薄弱
- `store.rs` 已有 IndexStore（SQLite 读写）
- 关系仅 4 种 RelKind：calls/implements/contains/imports
- 索引持久化在 `.codegraph/index.db`（由 indexer.rs 生成）

约束：不修改索引引擎核心（tree-sitter 解析链）——归 #3；不修改 MCP 层、不修改 TUI。

## Goals / Non-Goals

**Goals:**

- 实现调用路径树输出（深度 ≤5，每跳带 file:line + RelKind 证据）
- 新增 `.codegraph/audit.log`（结构化 JSONL，append-only）
- 在 codegraph_node 和 codegraph_explore 输出中填充 confidence（基于解析源分级）和 source 字段
- 实现模糊匹配（Levenshtein ≤3）
- 实现输出过滤排序（按 confidence/relevance + name/kind）
- 新增 3 种查询模式：call_path、symbol_batch、module_summary
- 在 #1 的 bench-agent-replay.sh 上重新验证 codegraph 调用率不降

**Non-Goals:**

- 不修改 tree-sitter 解析链（索引如何产生 confidence——只需要标记"此符号来自 tree-sitter AST 解析"标志位，归 #3）
- 不引入 NLP/LLM 语义检索（超出当前范围）
- 不修改 codegraph index 的数据 schema（只读不改写）
- 不在 TUI 中显示审计日志（仅文件级）
- 不扩展现有关系类型（归 #3）

## Decisions

### D1：调用路径树算法 —— BFS/DFS + 深度限制

**决策**：从目标符号出发做 BFS（宽度优先）遍历 calls 关系，深度 ≤5 截断。每跳记录 {from_symbol, to_symbol, rel_kind, file:line}。

**数据结构**：
```json
{
  "symbol": "run_async",
  "call_paths": [
    {
      "depth": 3,
      "hops": [
        {"from": "main", "to": "init", "rel": "calls", "location": "src/cli/mod.rs:45"},
        {"from": "init", "to": "setup_agent", "rel": "calls", "location": "src/agent/core.rs:120"},
        {"from": "setup_agent", "to": "run_async", "rel": "calls", "location": "src/agent/core.rs:201"}
      ]
    }
  ]
}
```

**理由**：
- BFS 天然支持最短路径，与 spec「最多 5 跳」且「优先展示短路径」
- 现有 calls 关系存储在 SQLite，可通过递归 CTE 或内存 graph 遍历
- 深度限制防止指数爆炸

**替代方案**：
- DFS —— 拒绝。单路径可能过长，跳过更短的备选路径
- 不缓存图（每次全量 SQL 查）—— 拒绝。小项目 OK，大项目 O(n²)

### D2：审计日志格式 —— JSONL append + audit_id

**格式**（每行一个 JSON 对象）：
```json
{"ts":"2026-06-15T10:30:00Z","audit_id":"550e8400-e29b-41d4-a716-446655440000","query_type":"codegraph_node","params":{"symbol":"ToolRegistry"},"result_count":1,"elapsed_ms":12,"source_files":["src/tools/mod.rs"]}
```

**audit_id 生成**：UUID v4，每次查询生成，返回给 Agent 的 output 中包含 `audit_id` 字段。Agent 可引用 audit_id 报告异常。

**写入方式**：
- `std::fs::OpenOptions::append(true).open(log_path)` 每次查询追加
- 文件路径：`.codegraph/audit.log`
- 不预设 rotate 机制（后续 change 可引入 `.audit.log.N` rollover）

### D3：Confidence 分级策略 —— 基于解析源

**决策**：在索引阶段标记每个符号的解析源，查询时映射：

| 解析源 | Confidence | 说明 |
|--------|-----------|------|
| tree-sitter AST 直接解析 | High | 语法树精确产出 |
| 文本正则匹配 / AST 辅助推断 | Medium | 匹配到但非直接 AST 节点 |
| 名称推断 / 默认值 | Low | 推测性关系 |
| 索引中不存在 | Unresolved | 找不到 |

**本期实现**：由于不修改索引器（归 #3），当前所有现有符号的 confidence 暂时标记为 High（因为现有索引器 100% 走 tree-sitter AST）。新查询模式（如 call_path 的 Dijkstra 路径可能经多跳关系链，不直接对应单个 tree-sitter 节点）标记为 Medium。

### D4：查询模式接口 —— 独立 tool 实现

**决策**：3 个新模式作为**独立 tool** 注册（`call_path`、`symbol_batch`、`module_summary`），复用 QueryEngine 方法。这样 Agent 通过 tool selection 即可选择查询模式，不需要给 codegraph_node/explore 加模式参数。

**Tool 定义概览**：

| Tool | Input | Output |
|------|-------|--------|
| `call_path` | from_symbol, to_symbol | 最短路径 JSON（hops[] + depth） |
| `symbol_batch` | symbols[] (最多 10) | 每个 symbol 的 codegraph_node 结果聚合 |
| `module_summary` | module_path (e.g. "src/tools") | 符号列表、导出函数、依赖关系 |

### D5：模糊匹配实现 —— 轻量 Levenshtein

**决策**：在 codegraph_node 精确匹配无结果时，从索引中取所有权重 ≥0.6 的候选（Levenshtein 距离 ≤3 且长度差 ≤50%），按距离排序返回 top 5 建议。参考现有 spec「Symbol not found → suggestions」scenario。

**不引入 crate**：Levenshtein ≤3 对短符号名（平均长度 15）只需 O(n×m) 内循环，手写 15 行即可。

### D6：审计日志 + confidence 对 AI 可见性

**决策**：
- `audit_id` 出现在 codegraph_node/explore/call_path 等所有 tool 的返回 JSON 中
- `confidence` 字段作为 `symbols[].confidence` 返回
- `source` 字段作为 `symbols[].source` 返回（值为 `"treesitter-ast" | "regex-match" | "inferred" | "none"`）
- 审计日志文件位于 `.codegraph/audit.log`，AI Agent 可通过 file_read 直接读取（JSONL 一行一个查询，jq 友好）

## Risks / Trade-offs

| 风险 | 缓解 |
|------|------|
| 图搜索 O(n²) 大项目慢 | 深度 ≤5 限制；最大路径数阈值（100）；首次查询缓存图 |
| 审计日志无限增长 | JSONL 压缩率高；后续可加 rotate；本期不加（YAGNI） |
| 3 个新 tool 让 Agent 选择困惑 | #1 playbook 已提供优先级；tool description 含 PREFER FOR |
| Levenshtein 自实现有边缘 bug | 单元测试覆盖 Unicode/空字符串/极长符号/大小写 |
| confidence 全固定 High 短期内缺乏区分度 | #3 多语言索引器引入后自然分化为 High/Medium/Low |
| call_path 两点无路径时 Agent 怎么处理 | 输出 `{"path_found": false, "reason": "no_connecting_path"}` |

## Migration Plan

不适用 — 纯增强，不修改现有行为。codegraph_node 和 codegraph_explore 原有输出格式保持兼容（在原 JSON 上追加 `confidence`/`source`/`audit_id` 字段）。新增的 `.codegraph/audit.log` 自动创建，首次查询前不存在不影响正常使用。

## Open Questions

以下问题需在 build 阶段 brainstorming 解决：

1. **图缓存的粒度**：每次查询加载全量 calls → 内存 graph，还是用递归 SQL CTE？小项目内存 OK；大项目（10K+ calls）CTE 可能更快但实现更复杂
2. **audit.log 的竞态处理**：多线程并发写同一个 log 文件——用 `Mutex<BufWriter>` 还是每次 `OpenOptions::append` + `write_all`？（后者天然原子性由 OS 保证行级写入）
3. **symbol_batch 的 max batch size**：10 还是 20？批量过大可能让 Agent 一次 query 消耗过多 token
4. **module_summary 的模块边界**：用文件系统目录边界还是 Rust `mod` 声明？目录边界简单（`src/tools/codegraph/` → 统计该目录下所有 `.rs` 中定义的符号）；mod 声明精确但需解析源码
5. **call_path 多路径展示策略**：返回所有最短路径（≤3 条）还是仅 1 条？全返回可能路径数爆炸（如 `main → println`）；仅 1 条可能漏掉重要的备选路径
