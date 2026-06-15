# Comet Design Handoff

- Change: codegraph-multilang-and-deep-graph
- Phase: design
- Mode: compact
- Context hash: bee87cddc48991d45a9a316c650ed8f11f1c12897be2332dad914e97408dc75f

Generated-by: comet-handoff.sh

OpenSpec remains the canonical capability spec. This handoff is a deterministic, source-traceable context pack, not an agent-authored summary.

## openspec/changes/codegraph-multilang-and-deep-graph/proposal.md

- Source: openspec/changes/codegraph-multilang-and-deep-graph/proposal.md
- Lines: 1-39
- SHA256: 87779cb88045f03f9c7b34fec5b90347069be7ca1c17900efe004082e78485c7

```md
## Why

#0/#1/#2 已完成 codegraph 的基线测量、Agent 采纳提升、查询增强和可解释性。但 codegraph 当前仅支持 Rust 单语言，且关系类型限于 4 种（calls/implements/contains/imports），无法捕获继承链、类型归属、返回值类型、参数关系等深层语义。用户原始需求中的「多语言广度 = Rust+Java+Python 都做深」和「Symbol Graph 深化」在本 change 中实现。

## What Changes

- **多语言 parser**：新增 tree-sitter-java 和 tree-sitter-python 依赖；抽离 `LanguageAdapter` trait（含 `parse()`、`extract_symbols()`、`extract_relationships()`）；实现 RustAdapter / JavaAdapter / PythonAdapter 三种适配器
- **深层关系**：新增 `RelKind` 变种 — `Inherits`（继承/实现）、`TypeOf`（类型归属）、`Returns`（返回值类型）、`Parameter`（参数类型）。@≥4 种
- **索引器重构**：parser pool 多语言分发（根据文件扩展名选适配器）；索引 schema 演进（新关系类型的列/表迁移）
- **types 模型扩展**：现有 `Symbol` / `Relationship` / `Confidence` 模型对多语言通用化，增加 language 字段
- **覆盖率验收**：在 Java/Python 样例项目上验证覆盖率 ≥70%（来自 #0 基线报告目标）

## Capabilities

### New Capabilities

- `multilang-indexing`：多语言 tree-sitter 索引能力，支持 Rust/Java/Python 三语言，通过 LanguageAdapter trait 实现语言无关的符号提取和关系构建。索引 schema 支持 language 字段。
- `symbol-graph-deep`：深层 Symbol Graph 关系，包括继承链（Inherits）、类型归属（TypeOf）、返回值类型（Returns）、参数类型（Parameter），扩展 RelKind 枚举并支持跨语言关系查询。

### Modified Capabilities

- `code-indexing`：索引器从单语言 Rust 重构为多语言 adapter 模式；索引 schema 增加 language 字段和新关系类型的存储列

## Impact

- **修改文件**：
  - `Cargo.toml`：新增 `tree-sitter-java`、`tree-sitter-python` 依赖
  - `src/tools/codegraph/parser.rs`：重构为 trait + adapter 模式
  - `src/tools/codegraph/indexer.rs`：多语言分发 + schema 迁移
  - `src/tools/codegraph/types.rs`：新增 RelKind 变种 + language 字段
  - `src/tools/codegraph/store.rs`：新关系类型 SQL 存储 + schema migration
  - `src/tools/codegraph/query.rs`：新关系类型的查询支持
- **新增文件**：
  - `src/tools/codegraph/adapters/`：RustAdapter / JavaAdapter / PythonAdapter
  - `src/tools/codegraph/migration.rs`：schema 版本管理
- **不修改**：prompts；MCP 协议；TUI；audit log
- **新增依赖**：`tree-sitter-java`、`tree-sitter-python`
- **运行影响**：首次启动时触发 schema 迁移（新增 language 列和新关系类型表）；全量索引时间增加（三语言 parser 初始化）；对仅 Rust 项目的用户无影响（向后兼容）
- **风险**：中高。Schema 迁移不可逆（缓解：migration.rs 版本化 + 保留 `--dry-run` 模式）；多语言 parser 复杂性高（缓解：adapter trait 隔离每种语言实现）
```

## openspec/changes/codegraph-multilang-and-deep-graph/design.md

- Source: openspec/changes/codegraph-multilang-and-deep-graph/design.md
- Lines: 1-66
- SHA256: d28ac4672b7ce1729ee2448bb9343fc859ece0469acaf40086af6adb59df90ae

```md
## Context

codegraph 当前仅支持 Rust 单语言解析（`tree-sitter-rust 0.24`），关系类型限于 4 种。parser.rs 和 indexer.rs 紧密耦合 Rust-specific 逻辑。Types 模型已通用化（Symbol/Relationship/Confidence 不绑语言），为新语言扩展奠定基础。

## Goals / Non-Goals

**Goals:**
- 新增 tree-sitter-java 和 tree-sitter-python，实现三语言深度解析
- 抽离 LanguageAdapter trait + 3 种语言适配器
- 新增 ≥4 种 RelKind（Inherits/TypeOf/Returns/Parameter）
- 索引 schema 演进（language 字段 + 新关系表）
- Java/Python 样例项目覆盖率 ≥70%

**Non-Goals:**
- 不新增第 4+ 种语言
- 不实现跨语言语义检索（如 "Java class A → Python class B"）
- 不修改 TUI / MCP / prompts

## Decisions

### D1: LanguageAdapter trait 设计

```rust
pub trait LanguageAdapter: Send + Sync {
    fn language(&self) -> &'static str; // "rust" | "java" | "python"
    fn parse(&self, source: &str) -> Result<Tree, ParseError>;
    fn extract_symbols(&self, tree: &Tree, source: &str, file_path: &str) -> Vec<Symbol>;
    fn extract_relationships(&self, tree: &Tree, source: &str, symbols: &[Symbol]) -> Vec<Relationship>;
    fn file_extensions(&self) -> &[&str]; // ["rs"] | ["java"] | ["py"]
}
```

### D2: 文件扩展名路由

`indexer.rs` 根据文件扩展名选择 adapter：`.rs`→RustAdapter, `.java`→JavaAdapter, `.py`→PythonAdapter。其余文件跳过。

### D3: 新 RelKind

在 `RelKind` 枚举中增加：`Inherits`（extends/implements）、`TypeOf`（变量:类型）、`Returns`（函数→返回类型）、`Parameter`（函数→参数类型）。

### D4: Schema 迁移策略

`migration.rs` 版本化管理：version=1 为当前 schema；version=2 增加 language 列 + 新关系表。IndexStore::open() 时检测并自动迁移。

### D5: parser pool

Parser 按语言缓存（HashMap<&str, Arc<Mutex<CodeParser>>>），避免重复初始化 tree-sitter grammar。

## Risks

| 风险 | 缓解 |
|------|------|
| Schema 迁移不可逆 | migration.rs 版本化 + --dry-run |
| 多语言编译时间长 | feature flags 可选 language |
| Java/Python tree-sitter 质量 | 样例项目验证 ≥70% coverage |
| 新 RelKind 查询复杂度 | 复用现有 call_path/explore 逻辑 |

## Migration Plan

首次启动自动检测 schema version 并迁移。降级不支持（需重新 `codegraph index --full`）。

## Open Questions (build 阶段 brainstorming)
1. Java lambda/匿名类如何表示？(tree-sitter-java 的支持程度)
2. Python 动态类型如何映射到 TypeOf？（仅标注静态可推导的类型）
3. 索引性能退化多少？（bench-perf.sh 重跑对比）
4. LanguageAdapter 是否需要 async？（tree-sitter parse 是 CPU-bound，sync 即可）
```

## openspec/changes/codegraph-multilang-and-deep-graph/tasks.md

- Source: openspec/changes/codegraph-multilang-and-deep-graph/tasks.md
- Lines: 1-56
- SHA256: 7649a366f8b2b71a9f6b236bc2ac4c9fa5d069029e6bdb67c5111cda86eea1c9

```md
# Tasks — codegraph-multilang-and-deep-graph

## 1. 依赖 + LanguageAdapter trait

- [ ] 1.1 Cargo.toml: 新增 tree-sitter-java, tree-sitter-python
- [ ] 1.2 创建 src/tools/codegraph/adapters/mod.rs: LanguageAdapter trait 定义
- [ ] 1.3 实现 RustAdapter (src/tools/codegraph/adapters/rust.rs)
- [ ] 1.4 实现 JavaAdapter (src/tools/codegraph/adapters/java.rs)
- [ ] 1.5 实现 PythonAdapter (src/tools/codegraph/adapters/python.rs)
- [ ] 1.6 单元测试：每种 adapter 解析简单代码片段

## 2. Types + RelKind 扩展

- [ ] 2.1 types.rs: Symbol 增加 language 字段
- [ ] 2.2 types.rs: RelKind 新增 Inherits/TypeOf/Returns/Parameter
- [ ] 2.3 单元测试：新 RelKind 的 as_str/parse

## 3. Parser Pool + 路由

- [ ] 3.1 parser.rs: 重构为多语言 parser pool (HashMap<&str, Arc<Mutex<CodeParser>>>)
- [ ] 3.2 parser.rs: 文件扩展名→language 路由
- [ ] 3.3 单元测试：pool 缓存、语言路由正确性

## 4. Schema 迁移

- [ ] 4.1 创建 src/tools/codegraph/migration.rs: version 检测 + 自动迁移
- [ ] 4.2 store.rs: IndexStore::open() 时调用 migration
- [ ] 4.3 store.rs: 新增新 RelKind 的 insert/query 方法
- [ ] 4.4 单元测试：迁移前后数据完整性

## 5. Indexer 适配

- [ ] 5.1 indexer.rs: 注入 adapter map
- [ ] 5.2 indexer.rs: 按文件扩展名选择 adapter → extract_symbols/extract_relationships
- [ ] 5.3 indexer.rs: language 字段写入
- [ ] 5.4 单元测试：三语言各索引一个 fixture 文件

## 6. Query 适配

- [ ] 6.1 query.rs: 新 RelKind 查询（按 Inherits/TypeOf 过滤）
- [ ] 6.2 tools.rs: 现有 tool description 更新（mention Java/Python support）

## 7. 验收

- [ ] 7.1 Java 样例项目（≥100 文件）索引验证 coverage ≥70%
- [ ] 7.2 Python 样例项目（≥100 文件）索引验证 coverage ≥70%
- [ ] 7.3 bench-perf.sh 重跑对比 #0 基线（全量索引耗时 ≤ baseline × 1.5）
- [ ] 7.4 cargo build + cargo test 全绿
- [ ] 7.5 现有 Rust 索引数据向后兼容（不丢数据）

## 8. Spec 同步 + 归档

- [ ] 8.1 创建 delta specs: multilang-indexing/spec.md (ADDED)
- [ ] 8.2 创建 delta specs: symbol-graph-deep/spec.md (ADDED)
- [ ] 8.3 修改 delta specs: code-indexing/spec.md (MODIFIED)
- [ ] 8.4 openspec validate → comet-verify → comet-archive
```

## openspec/changes/codegraph-multilang-and-deep-graph/specs/code-indexing/spec.md

- Source: openspec/changes/codegraph-multilang-and-deep-graph/specs/code-indexing/spec.md
- Lines: 1-24
- SHA256: a9e8d1a9111c943e42192892b0e7396ce518c70158887a8fa6eff3c216937558

```md
## MODIFIED Requirements

### Requirement: Tree-sitter based code indexing

系统 SHALL 使用 tree-sitter 解析器进行代码索引，支持多语言 parser pool 和 LanguageAdapter trait 模式。索引 schema SHALL 包含 language 字段。

#### Scenario: Full index on multi-language project

- **WHEN** 在包含 .rs/.java/.py 文件的项目上运行 `wgenty-code codegraph index`
- **THEN** 每个文件根据扩展名选择正确的 LanguageAdapter，提取符号和关系，写入索引

#### Scenario: Incremental index preserves language info

- **WHEN** 仅修改 `.java` 文件后增量索引
- **THEN** 仅重新索引变更的 Java 文件，其他语言数据保留

### Requirement: Schema migration

系统 SHALL 支持索引 schema 的版本化自动迁移。

#### Scenario: Version 1 → Version 2 migration

- **WHEN** 项目在升级 codegraph 后首次打开旧版本索引
- **THEN** schema 自动迁移：新增 language 列、新关系类型表；原有数据保留
```

## openspec/changes/codegraph-multilang-and-deep-graph/specs/multilang-indexing/spec.md

- Source: openspec/changes/codegraph-multilang-and-deep-graph/specs/multilang-indexing/spec.md
- Lines: 1-33
- SHA256: cbfe12bdc2ae94c753810bebc679b45d4631202b8c771e7e2f1fe89659aedc09

```md
## ADDED Requirements

### Requirement: LanguageAdapter trait

系统 SHALL 提供 LanguageAdapter trait 作为语言无关的解析接口。

#### Scenario: Adapter 注册
- **WHEN** Indexer 初始化
- **THEN** RustAdapter / JavaAdapter / PythonAdapter 按文件扩展名注册到 adapter map

#### Scenario: 文件扩展名路由
- **WHEN** 索引 `.rs` / `.java` / `.py` 文件
- **THEN** 自动选择对应的 LanguageAdapter 进行解析

### Requirement: Multi-language parsing

系统 SHALL 支持 Rust/Java/Python 三种语言的 tree-sitter 解析。

#### Scenario: Java 解析
- **WHEN** 索引 `.java` 文件
- **THEN** tree-sitter-java 提取类/方法/字段等符号

#### Scenario: Python 解析
- **WHEN** 索引 `.py` 文件
- **THEN** tree-sitter-python 提取函数/类/模块等符号

### Requirement: Language field in symbol

系统 SHALL 在 Symbol 模型中包含 language 字段。

#### Scenario: Symbol 含 language
- **WHEN** 从多语言项目中查询 symbol
- **THEN** 每个 symbol 返回 language 字段 ("rust"/"java"/"python")
```

## openspec/changes/codegraph-multilang-and-deep-graph/specs/symbol-graph-deep/spec.md

- Source: openspec/changes/codegraph-multilang-and-deep-graph/specs/symbol-graph-deep/spec.md
- Lines: 1-37
- SHA256: d583590b8167988b91b9b355a4dd37d13a4cbbd1706ce1a79843537c86e3a775

```md
## ADDED Requirements

### Requirement: Inherits relationship

系统 SHALL 支持继承/实现关系。

#### Scenario: Java extends
- **WHEN** 解析 `class Dog extends Animal`
- **THEN** 产生 RelKind::Inherits 关系 (Dog → Animal)

#### Scenario: Rust impl
- **WHEN** 解析 `impl Display for Foo`
- **THEN** 产生 RelKind::Inherits 关系 (Foo → Display)

### Requirement: TypeOf relationship

系统 SHALL 支持变量类型归属关系。

#### Scenario: Variable type
- **WHEN** 解析 `let x: String` 或 `String name`
- **THEN** 产生 RelKind::TypeOf 关系 (x/name → String)

### Requirement: Returns relationship

系统 SHALL 支持函数返回值类型关系。

#### Scenario: Return type
- **WHEN** 解析 `fn foo() -> Bar`
- **THEN** 产生 RelKind::Returns 关系 (foo → Bar)

### Requirement: Parameter relationship

系统 SHALL 支持函数参数类型关系。

#### Scenario: Parameter type
- **WHEN** 解析 `fn foo(x: i32, y: &str)`
- **THEN** 产生 RelKind::Parameter 关系 (foo → i32, foo → &str)
```

