## MODIFIED Requirements

### Requirement: Role-enforced tool visibility for explore and plan

`explore` 和 `plan` 节点类型的工具可见性 SHALL 由其 `NodeContract` 的 `permissions`（`can_mutate_fs`）和 `capabilities` 驱动，而非硬编码 `is_leaf`/`explore_readonly` 逻辑。当 `explore_readonly` 全局配置启用（默认 true）且节点契约的 `permissions.can_mutate_fs = false` 时，`explore` 和 `plan` 子 agent SHALL NOT 拥有变更文件系统工具（`file_write`、`file_edit`、`apply_patch`）。`general-purpose` 子 agent MAY 保留完整工具集，受 depth 限制和统一权限管线约束。`filter_allowed_tools` SHALL 读取 `NodeContract.permissions` + `NodeContract.capabilities` 决定工具可见性，`explore_readonly` 配置作为 `permissions.can_mutate_fs` 的全局默认源，契约字段 `Option` 覆盖它。

#### Scenario: Explore cannot call file_write

- **WHEN** an `explore` subagent attempts to call `file_write` with `explore_readonly=true`（其 `NodeContract.permissions.can_mutate_fs = false`）
- **THEN** the call SHALL fail as not allowed for that agent type before execution

#### Scenario: Explore can call file_read

- **WHEN** an `explore` subagent calls `file_read` on a path inside the workspace
- **THEN** the tool SHALL be visible and proceed through the unified permission pipeline
