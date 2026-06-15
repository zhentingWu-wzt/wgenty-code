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
