## ADDED Requirements

### Requirement: 持久化每次分发的 NodeType

系统 SHALL 在保存 subagent transcript 时，把该 run 实际分发的 `NodeType` 一并持久化到 transcript store。

#### Scenario: 新 run 持久化正确的 node_type

- **WHEN** coordinator 以 `NodeType::Explore` 分发一个 subagent 并保存其 transcript
- **THEN** 该 transcript 的 `node_type` 字段为 `Explore`

#### Scenario: 所有内置 NodeType 均可持久化

- **WHEN** 分别以 Explore / Plan / GeneralPurpose / Verification / WgentyCodeGuide 分发并保存 transcript
- **THEN** 每个 transcript 的 `node_type` 字段记录其对应的 `NodeType`

### Requirement: 向后兼容的 schema 迁移

系统 SHALL 以幂等 `ALTER` 方式为 `subagent_transcripts` 表增加 `node_type` 列；旧库升级不崩溃，迁移前已存在的 legacy row 的 `node_type` 为 `None`。

#### Scenario: 旧库打开不崩溃

- **WHEN** 一个不含 `node_type` 列的旧 transcript DB 被打开
- **THEN** 迁移自动添加 `node_type` 列且不报错
- **AND** legacy row 的 `node_type` 读取为 `None`

#### Scenario: 迁移幂等

- **WHEN** 迁移对一个已有 `node_type` 列的库重复执行
- **THEN** 不报错（幂等，不重复加列）

### Requirement: Subagent list 按 NodeType 展示

系统 SHALL 在 `subagent list` 输出中为每条 run 显示其 `node_type`。

#### Scenario: list 输出含 node_type 列

- **WHEN** 用户运行 `subagent list`
- **THEN** 输出包含每个 run 的 `node_type` 列
- **AND** legacy run（`node_type=None`）以占位符（如 `-` 或 `unknown`）显示

### Requirement: Subagent trace 显示该 run 的 NodeType

系统 SHALL 在 `subagent trace <id>` 输出中显示该 run 的 `node_type`。

#### Scenario: trace 输出含 node_type

- **WHEN** 用户运行 `subagent trace <id>` 指向一个已持久化 `node_type` 的 run
- **THEN** 输出显示该 run 的 `node_type`

### Requirement: 零回归

系统 SHALL 不改变已有 transcript 字段语义、dispatch 行为与契约强制逻辑。

#### Scenario: 已有 list/trace/health 测试零回归

- **WHEN** 运行已有 subagent transcript / CLI 测试套件
- **THEN** 全部通过，无回归
