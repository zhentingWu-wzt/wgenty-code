# org-graph-contract-viewer Specification

## Purpose
TBD - created by archiving change org-graph-contract-viewer. Update Purpose after archive.
## Requirements
### Requirement: 默认表格视图渲染全部内置契约

系统 SHALL 提供一个 CLI 命令，默认以终端表格形式列出所有内置 `NodeContract` 的五维约束，至少包含：node type / can_spawn / can_mutate_fs / can_exec / input→output IO shape / budget / allowed_tools。

#### Scenario: 默认调用列出五个内置契约

- **WHEN** 用户运行契约渲染命令且不带 `--format`
- **THEN** 输出包含 Explore / Plan / GeneralPurpose / Verification / WgentyCodeGuide 五个节点类型
- **AND** 每个节点显示 can_spawn / can_mutate_fs / can_exec 三个权限位

#### Scenario: 表格涵盖五维约束

- **WHEN** 用户运行契约渲染命令（默认格式）
- **THEN** 输出为每个节点呈现 capability（allowed_tools）、permission、budget、IO shape、identity 五个维度的可读字段

### Requirement: Graphviz DOT 格式导出

系统 SHALL 支持 `--format dot`，输出合法的 Graphviz DOT 文本，每个内置契约渲染为一个节点，可被 `dot -Tsvg` 等工具渲染。

#### Scenario: dot 输出可被 Graphviz 解析

- **WHEN** 用户运行命令带 `--format dot`
- **THEN** 输出以 `digraph` 声明开头的合法 DOT 文本
- **AND** 输出可被 `dot` 命令成功解析（无语法错误）

### Requirement: Mermaid 格式导出

系统 SHALL 支持 `--format mermaid`，输出可嵌入 Markdown 并被 GitHub / mermaid 渲染的图定义。

#### Scenario: mermaid 输出合法图定义

- **WHEN** 用户运行命令带 `--format mermaid`
- **THEN** 输出以合法 mermaid 图类型声明开头（`graph` 或 `flowchart`）
- **AND** 每个内置契约渲染为图中的一个节点

### Requirement: JSON 格式导出

系统 SHALL 支持 `--format json`，输出可被 `NodeContract` 的 `Deserialize` 反序列化的 JSON 数组。

#### Scenario: json 输出可 serde 反序列化且逐字段相等

- **WHEN** 用户运行命令带 `--format json`
- **THEN** 输出为 JSON 数组，每个元素是一个完整 `NodeContract`
- **AND** 反序列化结果与 `NodeRegistry::builtin()` 中的契约逐字段相等

### Requirement: 权限维度反映 explore_readonly 配置

系统 SHALL 在渲染时如实反映 `explore_readonly` 配置驱动的 `can_mutate_fs` 维度：`explore_readonly=true` 时 Explore 与 Plan 的 `can_mutate_fs=false`；反之 `true`。

#### Scenario: explore_readonly=true 时 explore/plan 为只读

- **WHEN** `explore_readonly` 配置为 true 时渲染契约
- **THEN** Explore 与 Plan 节点的 can_mutate_fs 显示为 false

#### Scenario: explore_readonly=false 时 explore/plan 可写

- **WHEN** `explore_readonly` 配置为 false 时渲染契约
- **THEN** Explore 与 Plan 节点的 can_mutate_fs 显示为 true

