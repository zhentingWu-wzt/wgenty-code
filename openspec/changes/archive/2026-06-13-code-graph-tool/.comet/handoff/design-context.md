# Comet Design Handoff

- Change: code-graph-tool
- Phase: design
- Mode: compact
- Context hash: d84064d051c7feb3864efe1f59a4b55c63ec77d0f24939826208106901eb5ae4

Generated-by: comet-handoff.sh

OpenSpec remains the canonical capability spec. This handoff is a deterministic, source-traceable context pack, not an agent-authored summary.

## openspec/changes/code-graph-tool/proposal.md

- Source: openspec/changes/code-graph-tool/proposal.md
- Lines: 1-36
- SHA256: e604f08062aa32068a757fa0f0bc26009ba28b5ceb3848e2fa1775333e6080db

```md
## Why

当前 `tools/meta/lsp.rs` 基于正则表达式做符号查找，无法提供精确的 AST 级符号索引和调用图分析能力。AI agent 在处理复杂代码任务时需要精确理解代码结构（"谁调用了这个函数？""这个 trait 有哪些实现？"），正则方案误匹配率高且完全缺失关系图谱。引入 tree-sitter 静态索引 + LSP 实时查询的混合方案，为 agent 提供生产级的代码理解基础设施。

## What Changes

- **新增** tree-sitter 驱动的代码索引引擎，支持全量和增量索引
- **新增** 基于 SQLite 的符号存储：定义、引用、调用关系
- **新增** `codegraph_explore` 工具：按查询返回相关符号及其调用路径
- **新增** `codegraph_node` 工具：返回单个符号的定义、签名、callers/callees
- **新增** MCP Server 形态的 codegraph 接口，供外部客户端使用
- **新增** CLI 子命令 `codegraph index|query|clean`
- **升级** 现有 `lsp.rs` 的符号查找能力，与 codegraph 索引互通
- `.codegraph/` 目录新增为项目级索引存储

## Capabilities

### New Capabilities

- `code-indexing`: tree-sitter 驱动的 Rust 源码 AST 解析与符号索引构建，支持全量和增量更新
- `symbol-query`: 精确的符号定义查找与引用追踪，按名称/类型/位置检索
- `call-graph`: 函数级调用图分析，支持 caller（谁调用我）和 callee（我调用谁）双向查询
- `codegraph-mcp`: MCP 协议接口，将 codegraph 查询能力暴露为 MCP tools，供外部 AI 客户端使用

### Modified Capabilities

<!-- 当前 openspec/specs/ 下无已有 spec，无需修改现有能力 -->

## Impact

- **依赖新增**: `tree-sitter`、`tree-sitter-rust`、`rusqlite`（或等效 SQLite binding）
- **新增模块**: `src/tools/codegraph/`（索引引擎、查询引擎、MCP 适配层）
- **影响模块**: `src/tools/meta/lsp.rs`（与 codegraph 索引整合）、`src/tools/mod.rs`（注册新工具）
- **CLI 新增**: `src/cli/args.rs` 新增 `Codegraph` 子命令
- **存储新增**: 项目根目录 `.codegraph/` 目录（可加入 `.gitignore`）
- **二进制体积**: 预计增加 ~2-3 MB（tree-sitter 运行时 + Rust grammar）
```

## openspec/changes/code-graph-tool/design.md

- Source: openspec/changes/code-graph-tool/design.md
- Lines: 1-143
- SHA256: 1ef418798a70a7616c6f7c42e24693f0425e64aa2abb44b738311a168c9ddfdd

[TRUNCATED]

```md
## Context

`wgenty-code` 当前通过 `tools/meta/lsp.rs` 提供基于正则表达式的符号查找能力。该方案约覆盖 80% 的 LSP 功能但存在根本性局限：无法构建真正的 AST、无调用图分析、正则误匹配率高。AI agent 在执行复杂代码任务时需要精确的代码理解基础设施。

本设计基于 tree-sitter 实现离线静态索引 + 可选 LSP 实时补充的混合架构，为 agent 提供符号索引、引用追踪和调用图分析能力。

## Goals / Non-Goals

**Goals:**
- tree-sitter 驱动的 Rust 源码 AST 解析与符号提取
- SQLite 持久化索引：符号定义、引用、调用关系
- 符号查询 API：定义查找、引用追踪（`codegraph_node`）
- 符号探索 API：按查询返回相关符号及调用路径（`codegraph_explore`）
- 调用图分析：caller/callee，支持可配置深度的传递闭包
- 增量索引：文件变更检测，仅重索引变更文件
- 内置 Tool + MCP Server 双接口
- CLI 子命令：`codegraph index|query|clean`

**Non-Goals:**
- Rust 以外的语言支持（架构预留扩展点但不实现）
- 数据流分析、控制流图
- 宏展开追踪（tree-sitter 处理的是展开前的语法树）
- 跨仓库/workspace 级分析
- 图形化可视化
- 类型推断、泛型实例化追踪
- trait 方法实现的跨文件自动发现

## Decisions

### Decision 1: tree-sitter 作为解析引擎（而非 rust-analyzer / syn）

| 维度 | tree-sitter | rust-analyzer (LSP) | syn (Rust proc macro) |
|------|-------------|---------------------|------------------------|
| 解析速度 | 极快（增量解析 ms 级） | 慢（需要编译级分析） | 中等 |
| 容错性 | 优秀（错误恢复） | 依赖完整项目 | 差（要求合法语法） |
| 部署复杂度 | 低（单库 + grammar） | 高（需要 rust-analyzer 二进制） | 低 |
| 多语言扩展 | 原生支持 | 每语言一个 server | 仅 Rust |
| 语义精确度 | 语法级（非语义级） | 编译级（类型解析） | 语法级 |

**选择 tree-sitter**，理由：
- 语法级精确度满足符号索引 + 调用图需求
- 容错性强，无需完整编译环境
- 可作为通用索引引擎，后续扩展其他语言
- 与 `.codegraph/` 离线索引模式天然匹配

### Decision 2: SQLite 作为索引存储（而非 LMDB / 自定义二进制格式）

**选择 SQLite (rusqlite)**：
- WAL 模式支持读多写少并发，AI agent 查询场景匹配
- schema 灵活，后续增加关系类型无需迁移格式
- 与项目已有的 `rusqlite` 依赖一致（若不存在则新增）
- SQL 查询能力让复杂图遍历（传递闭包）直接用递归 CTE
- 单文件存储，易于分发和清理

### Decision 3: 双形态架构（内置 Tool + MCP Server）

```
┌─────────────────────────────────────────────────────┐
│                 codegraph 核心引擎                     │
│                                                       │
│  ┌──────────┐  ┌────────────┐  ┌──────────────────┐  │
│  │ Indexer  │  │QueryEngine │  │   IndexStore      │  │
│  │          │  │            │  │   (SQLite)        │  │
│  │ • parse  │  │ • symbol   │  │                   │  │
│  │ • extract│  │ • refs     │  │ symbols | refs    │  │
│  │ • store  │  │ • callgraph│  │ relationships     │  │
│  └────┬─────┘  └─────┬──────┘  └────────┬──────────┘  │
│       │              │                  │              │
│       └──────────────┼──────────────────┘              │
│                      │                                 │
├──────────────────────┼─────────────────────────────────┤
│              接口层   │                                 │
│       ┌──────────────┼──────────────┐                  │
│       ▼              ▼              ▼                  │
│  ┌─────────┐  ┌──────────┐  ┌──────────┐              │
│  │ CLI     │  │  Tool    │  │   MCP    │              │
│  │ codegraph│ │  impl    │  │  Server  │              │
│  │ index/   │  │ codegraph│  │ codegraph│              │
│  │ query/   │  │ _explore │  │ _explore │              │
│  │ clean    │  │ _node    │  │ _node    │              │
```

Full source: openspec/changes/code-graph-tool/design.md

## openspec/changes/code-graph-tool/tasks.md

- Source: openspec/changes/code-graph-tool/tasks.md
- Lines: 1-70
- SHA256: eb90ad84785fb43988acf4005df0ee2777d8e3d7c609f92891968f3919f9bcee

```md
## 1. 项目基础设施

- [ ] 1.1 添加 tree-sitter、tree-sitter-rust、rusqlite 依赖到 Cargo.toml
- [ ] 1.2 创建 `src/tools/codegraph/` 模块目录结构和 `mod.rs` 入口
- [ ] 1.3 在 `src/tools/mod.rs` 中注册 codegraph 模块

## 2. 数据模型与存储层

- [ ] 2.1 定义核心类型：`Symbol`, `SymbolKind`, `Reference`, `Relationship`, `Visibility` 在 `types.rs`
- [ ] 2.2 实现 `IndexStore`：SQLite schema 创建（symbols, references, relationships, files 表）在 `store.rs`
- [ ] 2.3 实现 `IndexStore::upsert_symbol` / `delete_symbol` / `get_symbol` CRUD 方法
- [ ] 2.4 实现 `IndexStore::insert_reference` / `get_references` 引用管理
- [ ] 2.5 实现 `IndexStore::insert_relationship` / `get_callers` / `get_callees` 关系查询
- [ ] 2.6 实现 `IndexStore` 的文件哈希变更检测方法

## 3. 索引引擎

- [ ] 3.1 实现 tree-sitter Rust parser 初始化和语言注入
- [ ] 3.2 实现 AST 遍历器：提取函数、结构体、枚举、trait、impl、类型别名、常量、静态变量、模块定义
- [ ] 3.3 实现符号引用提取：函数调用、类型引用、use 声明中的符号引用
- [ ] 3.4 实现调用关系提取：call_expression → callee 映射，含方法调用解析
- [ ] 3.5 实现全量索引流程：扫描所有 `.rs` 文件 → 解析 → 存储，含进度输出
- [ ] 3.6 实现增量索引流程：文件哈希比对 → 仅重索引变更文件

## 4. 查询引擎

- [ ] 4.1 实现 `codegraph_node` 查询：按名称查找符号定义、引用、callers/callees
- [ ] 4.2 实现 `codegraph_explore` 查询：关键词匹配符号 + 返回相关调用路径
- [ ] 4.3 实现传递闭包查询：`get_callers`/`get_callees` 带 depth 限制（默认2，最大5）
- [ ] 4.4 实现模糊匹配：未找到符号时，按 Levenshtein 距离 ≤ 3 提供相似名称建议

## 5. 内置 Tool 实现

- [ ] 5.1 实现 `CodegraphNodeTool`：Tool trait，调用 query 引擎的 `codegraph_node`
- [ ] 5.2 实现 `CodegraphExploreTool`：Tool trait，调用 query 引擎的 `codegraph_explore`
- [ ] 5.3 在 `ToolRegistry` 中注册两个 codegraph 工具为 read-only

## 6. CLI 命令

- [ ] 6.1 在 `src/cli/args.rs` 中添加 `Codegraph` 子命令（index/query/clean）
- [ ] 6.2 实现 `codegraph index` 命令：调用索引引擎的全量/增量索引
- [ ] 6.3 实现 `codegraph query <symbol>` 命令：CLI 下行 `codegraph_node` 查询
- [ ] 6.4 实现 `codegraph clean` 命令：删除 `.codegraph/` 目录

## 7. MCP Server 集成

- [ ] 7.1 实现 MCP 工具适配层：将 `codegraph_explore`/`codegraph_node` 包装为 MCP tools
- [ ] 7.2 在 MCP 服务注册表中注册 codegraph MCP tools
- [ ] 7.3 实现 MCP 查询时的索引存在性检查和错误消息

## 8. 与现有 lsp.rs 整合

- [ ] 8.1 在 `lsp.rs` 中添加 codegraph 索引作为首选查询源（index-first, regex-fallback）
- [ ] 8.2 确保 `lsp.rs` goToDefinition/findReferences 在 codegraph 可用时优先使用索引结果

## 9. 测试

- [ ] 9.1 单元测试：types 序列化/反序列化
- [ ] 9.2 单元测试：IndexStore CRUD 操作
- [ ] 9.3 单元测试：tree-sitter 解析器符号提取（用 wgenty-code 自身源码作为测试输入）
- [ ] 9.4 集成测试：全量索引 + codegraph_node 查询端到端
- [ ] 9.5 集成测试：增量索引（修改文件 → 重新索引 → 验证仅有变更文件被更新）
- [ ] 9.6 集成测试：调用图查询（callers/callees/depth）
- [ ] 9.7 集成测试：MCP tools/list 和 tools/call
- [ ] 9.8 验证：`cargo clippy --all-targets -- -D warnings` 和 `cargo fmt -- --check` 通过

## 10. 收尾

- [ ] 10.1 将 `.codegraph/` 添加到项目 `.gitignore`
- [ ] 10.2 更新 CLAUDE.md 中关于 CodeGraph 的说明，标注 codegraph 工具已可用
```

## openspec/changes/code-graph-tool/specs/call-graph/spec.md

- Source: openspec/changes/code-graph-tool/specs/call-graph/spec.md
- Lines: 1-41
- SHA256: 5836fd6ecbb7e2d8d11d2944132b2df5e9e515873af1ece797db79e99d0cb6b1

```md
## ADDED Requirements

### Requirement: Caller analysis
The system SHALL return the list of all functions that call a given function.

#### Scenario: Direct callers
- **WHEN** querying `codegraph_node("execute")` with `callers` option
- **THEN** the system returns every function that directly invokes `execute()`, with call site location

#### Scenario: No callers (entry point)
- **WHEN** querying callers for `main()`
- **THEN** the system returns an empty caller list with `is_entry_point: true` indication

### Requirement: Callee analysis
The system SHALL return the list of all functions called by a given function.

#### Scenario: Direct callees
- **WHEN** querying `codegraph_node("run_async")` with `callees` option
- **THEN** the system returns every function directly called within `run_async()`, with call site locations

#### Scenario: Leaf function
- **WHEN** a function makes no calls to other user-defined functions
- **THEN** the system returns an empty callee list with `is_leaf: true` indication

### Requirement: Transitive call graph
The system SHALL support querying call relationships up to a configurable depth (default depth=2, max depth=5).

#### Scenario: Callers with depth=2
- **WHEN** querying `codegraph_node("checkpoint_file")` with `callers` and `depth=2`
- **THEN** the system returns direct callers AND their callers (transitive callers up to depth 2)

#### Scenario: Callees with depth=3
- **WHEN** querying `codegraph_node("process_request")` with `callees` and `depth=3`
- **THEN** the system returns the full call tree up to 3 levels deep

### Requirement: Method resolution for impl blocks
The system SHALL correctly resolve method calls on struct/enum types through their `impl` blocks.

#### Scenario: Method call on struct
- **WHEN** a function calls `registry.register(...)` and `register` is defined in `impl ToolRegistry`
- **THEN** the call graph resolves the call target to `ToolRegistry::register` in the corresponding impl block
```

## openspec/changes/code-graph-tool/specs/code-indexing/spec.md

- Source: openspec/changes/code-graph-tool/specs/code-indexing/spec.md
- Lines: 1-53
- SHA256: aa99d0ed674632084c881f011afb76d84ec21407dd3417cbfca90a9bd426bb29

```md
## ADDED Requirements

### Requirement: Full project indexing
The system SHALL parse all Rust source files in a project directory using tree-sitter and extract symbol definitions into a persistent index.

#### Scenario: First-time indexing
- **WHEN** user runs `wgenty-code codegraph index` in a Rust project for the first time
- **THEN** the system scans all `.rs` files, extracts symbols (functions, structs, enums, traits, impls, type aliases, consts, statics, modules), and stores them in `.codegraph/index.db`
- **THEN** the system outputs a summary: file count, symbol count, and elapsed time

#### Scenario: Indexing empty project
- **WHEN** user runs index on a directory with no `.rs` files
- **THEN** the system creates an empty `.codegraph/index.db` and reports "0 files, 0 symbols"

#### Scenario: Indexing a file with parse errors
- **WHEN** a `.rs` file has syntax errors
- **THEN** the system skips the malformed portions and indexes all valid symbols it can extract, reporting a warning count

### Requirement: Incremental indexing
The system SHALL detect file changes since last index and only re-index modified files.

#### Scenario: Single file change
- **WHEN** one `.rs` file has been modified (different hash from stored record)
- **THEN** the system re-indexes only that file and updates its symbols, removing stale entries

#### Scenario: File added
- **WHEN** a new `.rs` file is created since last index
- **THEN** the system indexes the new file without re-indexing unchanged files

#### Scenario: File removed
- **WHEN** a `.rs` file tracked in the index has been deleted
- **THEN** the system removes all symbols belonging to that file from the index

### Requirement: Index persistence
The system SHALL store the index in SQLite format under the `.codegraph/` directory with a defined schema for symbols, references, and relationships.

#### Scenario: Index survives process restart
- **WHEN** the index has been built and the process exits
- **THEN** a subsequent `codegraph query` can read the existing index without re-indexing

### Requirement: Parallel indexing
The system SHALL use a parser pool of size (num_cpus - 1) to parse multiple files concurrently during full indexing.

#### Scenario: Multi-file full index
- **WHEN** full indexing is triggered on a project with more than 10 `.rs` files
- **THEN** the system distributes files across parser pool workers for concurrent parsing and reports the parallelism level in the summary

### Requirement: Supported symbol kinds
The system SHALL recognize and classify at minimum: `function`, `struct`, `enum`, `trait`, `impl`, `type_alias`, `const`, `static`, `mod`, `macro`.

#### Scenario: Rust symbol classification
- **WHEN** indexing a file containing `pub fn foo()`, `struct Bar`, `enum Baz`, `trait Qux`
- **THEN** the index records symbols with kinds `function`, `struct`, `enum`, `trait` respectively, including their visibility modifiers
```

## openspec/changes/code-graph-tool/specs/codegraph-mcp/spec.md

- Source: openspec/changes/code-graph-tool/specs/codegraph-mcp/spec.md
- Lines: 1-33
- SHA256: e1c94db94f8be61f082852e4ea2ad8fde08e18d65738b8a7f411dcd8e2c372e2

```md
## ADDED Requirements

### Requirement: MCP tool listing
The system SHALL expose `codegraph_explore` and `codegraph_node` as MCP tools via the MCP protocol (JSON-RPC 2.0).

#### Scenario: MCP tools/list includes codegraph tools
- **WHEN** an MCP client calls `tools/list`
- **THEN** the response includes `codegraph_explore` and `codegraph_node` with their input schemas

### Requirement: MCP tool invocation
The system SHALL handle MCP `tools/call` requests for codegraph tools and return results in MCP-compliant format.

#### Scenario: codegraph_explore via MCP
- **WHEN** an MCP client calls `tools/call` with `name: "codegraph_explore"` and `arguments: {"query": "Tool trait implementors"}`
- **THEN** the system queries the codegraph index and returns results as MCP `text` content

#### Scenario: codegraph_node via MCP
- **WHEN** an MCP client calls `tools/call` with `name: "codegraph_node"` and `arguments: {"symbol": "ToolRegistry"}`
- **THEN** the system returns the symbol definition, references, and call graph in MCP `text` content

### Requirement: Index freshness check
The system SHALL verify the codegraph index exists before serving MCP requests and return a clear error if no index is found.

#### Scenario: MCP query without index
- **WHEN** an MCP client calls a codegraph tool but `.codegraph/index.db` does not exist
- **THEN** the system returns an error: "No codegraph index found. Run `wgenty-code codegraph index` first."

### Requirement: Built-in tool parity
The MCP codegraph tools SHALL have identical behavior and output format to the built-in `codegraph_explore` and `codegraph_node` tools.

#### Scenario: Same query, same result
- **WHEN** the same `codegraph_node("ToolRegistry")` query is made via built-in tool and via MCP
- **THEN** both return identical structured output
```

## openspec/changes/code-graph-tool/specs/symbol-query/spec.md

- Source: openspec/changes/code-graph-tool/specs/symbol-query/spec.md
- Lines: 1-53
- SHA256: f0764d93684994984ca682d728e300956582740307897c3742396394f2b13427

```md
## ADDED Requirements

### Requirement: Symbol definition lookup
The system SHALL return the exact file path, line number, column, signature, and visibility of a symbol given its name.

#### Scenario: Find a function definition
- **WHEN** querying `codegraph_node("ToolRegistry")`  
- **THEN** the system returns `src/tools/mod.rs:75` with the full signature and visibility

#### Scenario: Find a struct definition
- **WHEN** querying `codegraph_node("StreamEvent")`
- **THEN** the system returns the file path, line, column, and all fields of the struct

#### Scenario: Symbol not found
- **WHEN** querying a symbol name that does not exist in the index
- **THEN** the system returns a `not_found` result with suggestions for similarly-named symbols (Levenshtein distance ≤ 3)

#### Scenario: Ambiguous symbol name
- **WHEN** multiple symbols share the same name (e.g., `Config` in different modules)
- **THEN** the system returns all matches with their fully-qualified paths, letting the caller disambiguate

### Requirement: Symbol reference lookup
The system SHALL return all locations where a given symbol is referenced (called, imported, type-referenced).

#### Scenario: Find all references to a function
- **WHEN** querying references for a function `execute`
- **THEN** the system returns a list of {file, line, column, context_line} for every call site and import of `execute`

#### Scenario: No references found
- **WHEN** a symbol has only a definition and no references
- **THEN** the system returns an empty reference list with a clear indication

### Requirement: Index-first query strategy
The system SHALL query the codegraph index first before falling back to regex-based LSP search.

#### Scenario: Index available
- **WHEN** `codegraph_node` or `codegraph_explore` is called and the codegraph index exists
- **THEN** the system returns indexed results without invoking the regex-based lsp tool, and marks the result source as `[codegraph]`

#### Scenario: Index unavailable fallback
- **WHEN** `codegraph_node` or `codegraph_explore` is called and the codegraph index does not exist and cannot be auto-built
- **THEN** the system falls back to regex-based LSP search and marks the result source as `[regex fallback]`

### Requirement: Symbol exploration by query
The system SHALL accept a natural query string and return relevant symbols along with their relationships.

#### Scenario: Explore trait implementors
- **WHEN** querying `codegraph_explore("Tool implementations")`
- **THEN** the system finds the `Tool` trait and returns all `impl Tool for Xxx` blocks with their locations

#### Scenario: Explore module structure
- **WHEN** querying `codegraph_explore("tools module structure")`
- **THEN** the system returns the module hierarchy under `src/tools/` with key symbols in each submodule
```

