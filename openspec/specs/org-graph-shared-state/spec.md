# org-graph-shared-state Specification

## Purpose
TBD - created by archiving change org-graph-shared-state. Update Purpose after archive.
## Requirements
### Requirement: 强类型共享工作状态

系统 SHALL 为每个任务实例维护一个结构化的共享工作状态（`WorkState`），承载节点之间传递的工作产物。状态 SHALL 至少包含以下结构化字段：任务原始需求、生成的代码变更、编译结果（含成功标志与错误输出）、测试结果（含是否通过与失败用例清单）、人工审核结论、预算消耗、以及步骤审计日志。

#### Scenario: 工作产物以结构化字段流转

- **WHEN** 一个节点完成其工作并把结果写入共享工作状态的结构化字段
- **THEN** 后续节点能从该状态读取对应的结构化字段（如 verify 结果的成功标志与失败枚举分支），而不是解析自然语言文本

#### Scenario: 完整 schema 与 deferred-write-point

- **WHEN** 系统定义共享工作状态的完整字段集（任务需求、生成 diff、编译结果、测试结果、人工审核、verify 结果、预算、步骤日志）
- **THEN** 所有字段 SHALL 具备强类型与字段级权限强制（越权写一律拒绝）
- **AND** 本期 pilot 锚定唯一真实闭环 verify_result（二次查证确认 compile/test 闭环当前在代码中不存在，故不建虚构 pilot）
- **AND** 其余字段（compile_result / test_result / human_review / budget / generated_diff）类型与权限 SHALL 就绪，但生产写入点 SHALL NOT 在本期接入（权限强制通过单测合成写入验证，生产接入留待将来新增节点的 change）

#### Scenario: 状态字段强类型可序列化

- **WHEN** 共享工作状态被序列化（用于持久化或检查点）
- **THEN** 序列化结果可被反序列化回等价的结构化状态，字段类型不丢失

### Requirement: 节点对状态字段的访问受权限约束

系统 SHALL 为每种节点类型声明其可读与可写的共享工作状态字段子集。一个节点 SHALL NOT 写入其未声明可写的状态字段。

#### Scenario: 节点越权写字段被拒绝

- **WHEN** 一个节点尝试写入其节点类型未被授权写的状态字段
- **THEN** 系统拒绝该写入并报契约违规（与节点权限边界强制同款机制），状态保持写入前的值

#### Scenario: 节点正常读写授权字段

- **WHEN** 一个节点读写其节点类型声明可读/可写的状态字段
- **THEN** 读写成功完成，且该写操作被记入步骤审计日志（读操作不记入，避免高频读爆日志）

#### Scenario: 预留字段对所有节点强制为空

- **WHEN** 任何节点类型尝试写入当前无生产写入点的预留字段（如 compile_result / test_result / human_review）
- **THEN** 系统拒绝该写入并报契约违规（`ContractDimension::State`），状态保持写入前的值——这是全字段权限真强制的落地形式

### Requirement: 工作状态与既有状态层分层不吞并

系统 SHALL 把共享工作状态与既有的会话状态（会话骨架、turn 链、节点链）和全局应用配置三者职责分层。共享工作状态 SHALL NOT 取代或吞并会话状态或应用配置的既有职责。

#### Scenario: 三层状态互不吞并

- **WHEN** 共享工作状态被引入系统
- **THEN** 会话状态仍承担会话骨架职责（turn 链 / 节点链 / 会话状态），全局应用配置仍承担全局配置职责，共享工作状态只承载 per-task 工作产物
- **AND** 既有的会话状态与应用配置字段语义不发生改变

### Requirement: 工作状态随 turn 检查点持久化可续跑

系统 SHALL 把共享工作状态的生命周期锚定在会话 turn 上，并随既有 turn 检查点机制一并持久化。任务中途崩溃后，系统 SHALL 能从最近 turn 的共享工作状态快照恢复结构化工作产物，而非丢失在自由文本消息中。

#### Scenario: 崩溃后从检查点恢复结构化工作产物

- **WHEN** 一个任务在写入结构化工作状态后崩溃，随后从最近 turn 检查点恢复
- **THEN** 恢复后的共享工作状态包含崩溃前写入的结构化字段（如 verify 结果的强类型枚举）
- **AND** 已捕获的普通文件状态恢复行为不受影响（零回归）

### Requirement: 路由判定读取结构化字段而非解析文本

系统 SHALL 让至少一个真实存在的路由判定（本期 pilot）从「解析节点自然语言输出」改为「读取共享工作状态的结构化字段」。该 pilot 路由点 SHALL 包含一个真实的结构化字段写入场景，使字段级权限强制能被实际验证。

#### Scenario: pilot 路由点读结构化字段做判定

- **WHEN** pilot 路由点（verify_node 出口）需要判定 verify 结果以决定 retry / escalate
- **THEN** 判定读取共享工作状态的 verify_result 强类型字段（VerifyFailureKind 枚举分支：CommandFailed / BoundaryViolation），而不是解析 `format!("{f:?}")` 降级后的自然语言字符串

#### Scenario: pilot 字段写入受权限强制验证

- **WHEN** pilot 路由点涉及的结构化字段被对应节点写入
- **THEN** 该写入受字段级访问权限约束（节点须声明可写该字段），使本期字段级真强制存在可被验证的真实场景

### Requirement: 零回归与正交性

系统 SHALL NOT 改变既有节点契约的五维语义、节点派发行为、契约强制逻辑，也 SHALL NOT 改变既有 transcript store 的 schema。共享工作状态与在途的运行时派发遥测能力 SHALL 保持正交、可并行推进。

#### Scenario: 既有节点契约与派发强制零回归

- **WHEN** 引入共享工作状态后运行既有节点契约、派发与契约强制测试套件
- **THEN** 全部通过，无回归

#### Scenario: 与派发遥测能力正交

- **WHEN** 共享工作状态与运行时派发遥测同时存在于系统
- **THEN** 两者互不依赖、互不修改对方的字段或 schema，可独立交付

