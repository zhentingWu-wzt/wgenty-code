# code-indexing Specification

## Purpose
TBD - created by archiving change code-graph-tool. Update Purpose after archive.
## Requirements
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

### Requirement: Multi-language parser pool

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

