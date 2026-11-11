# session-scoped-model-mode

## Purpose

模型与权限模式支持会话级覆盖：同一 daemon 下并行会话可各自使用不同厂商/模型与权限档位；默认层（全局模型默认、project 级权限模式）保持原有语义，覆盖可显式重置。

## ADDED Requirements

### Requirement: 模型 session 覆盖

会话记录 SHALL 支持可选的模型覆盖（厂商 + 模型）。运行时会话解析模型时 SHALL 按链路解析：session 覆盖 > 全局默认（当前 active 模型）。未覆盖的会话行为与现状一致。session 覆盖 SHALL NOT 写回全局设置文件。

#### Scenario: 覆盖仅影响本会话

- **WHEN** 会话 A 设置厂商/模型覆盖为 zhipu/glm-5.3，会话 B 无覆盖
- **THEN** A 的后续轮次使用 glm-5.3，B 与全局默认模型均不变，全局 `settings.json` 未被修改

#### Scenario: 轮次边界生效

- **WHEN** 会话运行中发生模型覆盖切换
- **THEN** 当前轮次不被打断，下一轮次起使用新模型

### Requirement: 权限模式 session 覆盖

会话记录 SHALL 支持可选的权限模式覆盖。权限模式解析链 SHALL 为：session 覆盖 > 现有 project/worktree 级条目 > Normal。`Plan`（root Normal + effective Plan）SHALL NOT 持久化到会话覆盖（与会话重启后需用户重新确认的现有语义一致）。

#### Scenario: 会话间权限隔离

- **WHEN** 同一 project 下会话 A 覆盖为 yolo，会话 B 无覆盖且 project 级为 accept_edits
- **THEN** A 以 yolo 执行，B 以 accept_edits 执行

#### Scenario: Plan 不持久化

- **WHEN** 会话处于 Plan 模式时进程重启后恢复会话
- **THEN** 会话权限模式回到非 Plan 的持久值，不自动恢复 Plan

### Requirement: 覆盖重置

每个会话 SHALL 提供将模型覆盖与权限模式覆盖分别清除的操作；清除后会话立即回退到对应默认层，且该操作不影响其他会话与默认层设置。

#### Scenario: 重置模型覆盖

- **WHEN** 会话 A 存在模型覆盖，用户执行重置
- **THEN** A 后续轮次使用全局默认模型，会话记录中覆盖字段清空

### Requirement: Subagent 继承会话覆盖

subagent 在 spawn 时 SHALL 快照其所属会话解析后的模型（含 tier 路由基线）与权限模式；会话覆盖变化不影响已 spawn 的 subagent。

#### Scenario: spawn 快照

- **WHEN** 会话 A 覆盖模型为 X 时 spawn subagent，随后 A 切换覆盖为 Y
- **THEN** 该 subagent 全生命周期使用 X（tier 路由在 X 厂商范围内解析），A 的后续轮次使用 Y

### Requirement: 事件广播携带会话标识

模型变更与权限模式变更的全局事件广播 SHALL 携带 `session_id`（覆盖会话时）与生效的模型/模式值；客户端据此只更新对应会话的展示。

#### Scenario: 客户端按会话过滤

- **WHEN** 会话 A 切换模型触发广播
- **THEN** 仅订阅/激活会话 A 的界面更新其模型显示，其他会话显示不变
