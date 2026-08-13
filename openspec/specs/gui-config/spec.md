# gui-config Specification

## Purpose
TBD - created by archiving change gui-config-and-models. Update Purpose after archive.
## Requirements
### Requirement: 模型切换

系统 SHALL 在 GUI 中展示可用模型列表，并允许用户切换当前模型，切换 MUST 对后续新对话生效且不中断进行中的对话。

#### Scenario: 查看模型列表

- **WHEN** 用户打开模型界面
- **THEN** 展示当前模型与全部可用模型（含 provider 信息）

#### Scenario: 切换模型

- **WHEN** 用户选择另一个模型并确认切换
- **THEN** 当前模型更新，之后发起的对话使用新模型

### Requirement: 基础配置读写

系统 SHALL 提供常用配置项的图形化查看与修改，配置写入 MUST 通过 daemon API 完成；API key 等敏感值 MUST 脱敏展示，编辑时不回显明文。

#### Scenario: 查看配置

- **WHEN** 用户打开配置界面
- **THEN** 展示常用配置项当前值，敏感字段以掩码显示

#### Scenario: 修改配置

- **WHEN** 用户修改某配置项并保存
- **THEN** 通过 daemon API 写入，界面反馈保存成功或失败原因

#### Scenario: 编辑敏感值

- **WHEN** 用户编辑 API key 类字段
- **THEN** 输入框不回显原值，保存后仍以掩码展示

### Requirement: MCP servers 管理

系统 SHALL 展示已配置的 MCP servers 列表，支持启用/禁用与添加/移除。

#### Scenario: 查看与切换状态

- **WHEN** 用户打开 MCP 管理界面
- **THEN** 展示各 server 名称与连接/启用状态，用户可切换启用状态

#### Scenario: 添加与移除

- **WHEN** 用户添加新 server（填写必要参数）或移除现有 server
- **THEN** 变更通过 daemon API 生效并反映在列表中

### Requirement: skills 与 memory 管理

系统 SHALL 展示已安装 skills 列表，并提供 memory 条目的浏览、搜索与删除能力。

#### Scenario: 查看 skills

- **WHEN** 用户打开 skills 界面
- **THEN** 展示已安装 skills 的名称、来源与描述

#### Scenario: 管理 memory

- **WHEN** 用户在 memory 界面浏览、搜索或删除条目
- **THEN** 列表正确过滤，删除后条目不再出现

