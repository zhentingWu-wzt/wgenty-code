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
