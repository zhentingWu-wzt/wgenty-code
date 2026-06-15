## ADDED Requirements

### Requirement: call_path 两点间路径查询

系统 SHALL 提供 `call_path` 工具，查询从符号 A 到符号 B 的最短调用路径。

#### Scenario: 路径存在

- **WHEN** 查询 `call_path("main", "run_async")` 且调用路径存在
- **THEN** 返回一条 hops[] 数组，每跳包含 from/to/RelKind/file:line，加上 `depth` 总跳数

#### Scenario: 路径不存在

- **WHEN** 查询两个符号之间没有连接路径
- **THEN** 返回 `{"path_found": false, "reason": "no_connecting_path"}` 而非空数组

### Requirement: symbol_batch 批量查询

系统 SHALL 提供 `symbol_batch` 工具，一次查询多个符号名并聚合结果。

#### Scenario: 批量查询

- **WHEN** 查询 `symbol_batch(["ToolRegistry", "StreamEvent", "run_async"])`（最多 10 个）
- **THEN** 返回一个数组，每个元素等同于该符号的 `codegraph_node` 查询结果（定义位置、签名、callers/callees、confidence/source）

#### Scenario: 超量限制

- **WHEN** 查询的 symbols 数组超过 10 个
- **THEN** 返回错误 "Batch size exceeds maximum (10)"

### Requirement: module_summary 模块概要

系统 SHALL 提供 `module_summary` 工具，输出指定模块路径下的符号列表、导出函数和依赖关系。

#### Scenario: 模块概要

- **WHEN** 查询 `module_summary("src/tools/codegraph")`
- **THEN** 返回该目录下所有 Rust 文件中定义的符号列表（按 SymbolKind 分组）、公开导出函数清单、以及该模块依赖的其他模块列表

#### Scenario: 模块不存在

- **WHEN** 指定的模块路径在索引中没有文件
- **THEN** 返回 `{"found": false, "reason": "no indexed files under module_path"}`

### Requirement: 模糊匹配

系统 SHALL 在 `codegraph_node` 精确匹配无结果时提供 Levenshtein 距离 ≤3 的候选建议。

#### Scenario: 精确未命中 → 模糊补全

- **WHEN** 查询的符号名在索引中不存在（精确匹配）
- **THEN** 系统自动进行模糊匹配，按 Levenshtein 距离排序返回 top 5 候选（距离 ≤3 且长度差 ≤50%）

#### Scenario: 无任何候选

- **WHEN** 精确匹配和模糊匹配均无结果
- **THEN** 返回 `not_found` 结果（与现有 spec 保持一致）

### Requirement: 过滤与排序

系统 SHALL 支持 codegraph_node / codegraph_explore 输出结果的过滤和排序。

#### Scenario: 按置信度排序

- **WHEN** `codegraph_node` 调用指定 `sort_by: "confidence"`
- **THEN** 返回结果按 confidence 降序排列（high > medium > low > unresolved）

#### Scenario: 按名称过滤

- **WHEN** `codegraph_node` 调用指定 `filter: {"name_prefix": "run_"}`
- **THEN** 仅返回名称以 "run_" 开头的符号
