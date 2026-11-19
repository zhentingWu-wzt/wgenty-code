# model-mode-input-chips Specification

## Purpose
web 端将模型与权限模式的选择入口从底部 StatusBar 迁移到输入框上方，以两级（厂商 → 模型）chip 呈现，使入口位置与 session 级作用域一致，并在发送前呈现当前会话将使用的模型与权限。

## Requirements

### Requirement: 输入框上方的模型 chip

web 输入区上方 SHALL 展示当前会话生效的模型 chip（厂商 + 模型名，未覆盖时展示全局默认）。点击 chip SHALL 打开两级选择器：第一级列厂商，第二级列该厂商下模型（合并动态目录、内置目录与手工配置，见 model-catalog-fetch）。选择器 SHALL 提供「跟随默认」入口以清除会话覆盖。

#### Scenario: 两级选择

- **WHEN** 用户点击模型 chip，先选厂商 zhipu 再选模型 glm-5.3
- **THEN** 当前会话生效模型变为 zhipu/glm-5.3，chip 更新展示，其他会话不受影响

#### Scenario: 未覆盖会话的 chip 展示

- **WHEN** 会话无模型覆盖
- **THEN** chip 展示全局默认模型，无覆盖标记

### Requirement: 输入框上方的权限模式 chip

web 输入区上方 SHALL 展示当前会话生效的权限模式 chip（覆盖时为会话覆盖值，未覆盖时为 project 级值）。点击 SHALL 打开现有 normal/accept_edits/yolo 选择，作用于当前会话。

#### Scenario: 会话级模式切换

- **WHEN** 用户在会话 A 的模式 chip 选择 yolo
- **THEN** A 的权限模式变为 yolo；同 project 其他会话保持原值

### Requirement: StatusBar 瘦身

底部 StatusBar SHALL 移除模型名与权限模式入口，保留连接状态、待审批数、workspace、context 用量等会话无关或既有展示；权限模式待审批徽标等既有告警行为不变。

#### Scenario: 迁移后底部栏

- **WHEN** 页面渲染完成
- **THEN** 底部 StatusBar 不含模型名与权限模式控件，输入框上方可见模型与模式 chip

### Requirement: 会话切换时状态同步

切换活跃会话时，模型与权限模式 chip SHALL 立即反映目标会话的解析值（覆盖或默认层），订阅的变更事件仅更新对应会话的缓存值。

#### Scenario: 并行会话各自显示

- **WHEN** 会话 A 覆盖 zhipu/glm-5.3、会话 B 使用全局默认，用户在 A、B 间切换
- **THEN** chip 依次展示 glm-5.3 与全局默认模型，无串扰
