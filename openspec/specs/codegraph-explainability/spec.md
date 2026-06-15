# codegraph-explainability Specification

## Purpose
TBD - created by archiving change codegraph-query-and-explainability. Update Purpose after archive.
## Requirements
### Requirement: 审计日志记录

系统 SHALL 在每次 codegraph 查询（codegraph_node / codegraph_explore / call_path / symbol_batch / module_summary）执行后，向 `.codegraph/audit.log` 追加一条结构化记录。

#### Scenario: 查询后写入日志

- **WHEN** 任一 codegraph 查询完成（成功或失败）
- **THEN** `.codegraph/audit.log` 中追加一条 JSONL 记录，包含：`ts`（ISO8601 UTC）、`audit_id`（UUID v4）、`query_type`、`params`、`result_count`、`elapsed_ms`、`source_files[]`

#### Scenario: 日志文件首次创建

- **WHEN** `.codegraph/audit.log` 不存在且首次查询开始
- **THEN** 系统自动创建文件并以 append-only 模式写入

#### Scenario: audit_id 可追溯

- **WHEN** Agent 收到 codegraph 查询结果
- **THEN** 结果中包含 `audit_id` 字段，Agent 可通过 jq 或 file_read 在 `.codegraph/audit.log` 中按 audit_id 检索该条查询

#### Scenario: 并发写入安全

- **WHEN** 多个查询并发执行
- **THEN** 每条查询的日志记录完整写入（不交叉、不截断），OS 行级写入原子性保证

### Requirement: 调用路径树证据

系统 SHALL 在 call graph 查询结果中提供多跳调用路径，每跳标注来源文件和行号。

#### Scenario: 多跳路径展示

- **WHEN** `codegraph_explore` 返回涉及 calls 关系的结果
- **THEN** 结果包含 `call_paths` 字段，每一跳包含：`from`（符号名）、`to`（符号名）、`rel`（RelKind）、`location`（file:line）

#### Scenario: 深度限制

- **WHEN** 调用路径深度超过 5 跳
- **THEN** 路径在第 5 跳截断，并在结果中标注 `truncated: true`

### Requirement: 置信度与来源标注

系统 SHALL 在所有 codegraph 查询结果中为每个 symbol 标注 confidence 和 source 字段。

#### Scenario: tree-sitter 直接解析

- **WHEN** symbol 由 tree-sitter AST 直接解析产生（现有索引的默认来源）
- **THEN** `confidence` = "high"，`source` = "treesitter-ast"

#### Scenario: 间接推断

- **WHEN** symbol 由非直接 AST 节点的推断产生（如 call_path 的多跳关系链）
- **THEN** `confidence` = "medium"，`source` = "inferred"

#### Scenario: 模糊匹配

- **WHEN** symbol 由模糊匹配（Levenshtein ≤3）补全提供
- **THEN** `confidence` = "low"，`source` = "fuzzy-match"

