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
