# Spec: gui-app-shell

GUI 桌面应用骨架：窗口生命周期、导航与多面板布局、daemon 连接管理、UI 无关的会话编排客户端、应用级状态与错误兜底。

## ADDED Requirements

### Requirement: 应用启动与窗口生命周期

系统 SHALL 提供纯 Rust GUI 桌面应用入口，启动后展示主窗口，并在窗口关闭时完成资源清理（包括内嵌 daemon 的关停）。GUI 代码 MUST 通过 Cargo feature flag 隔离编译，默认构建不包含 GUI。

#### Scenario: 启动应用

- **WHEN** 用户通过 GUI 入口启动应用
- **THEN** 主窗口打开，进入默认布局，并自动建立 daemon 连接

#### Scenario: 默认构建不含 GUI

- **WHEN** 使用默认 feature 集构建项目
- **THEN** 产物不包含 GUI 依赖与 GUI 入口，启动时间与二进制大小满足 AGENTS.md 性能约束

#### Scenario: 关闭窗口

- **WHEN** 用户关闭主窗口
- **THEN** 应用退出前断开事件订阅，若 daemon 为本进程内嵌拉起则一并关停

### Requirement: daemon 连接管理

系统 SHALL 优先按发现机制连接已常驻的 daemon 实例（与其他 UI 共享同一会话真相）；发现失败时 MUST 回退为进程内拉起 daemon。连接失败时 MUST 向用户展示可操作的错误信息并允许重试。

#### Scenario: 连接常驻 daemon

- **WHEN** 发现机制找到存活的常驻 daemon
- **THEN** 应用直接连接该实例，可看到其他 UI 正在进行的会话

#### Scenario: 内嵌拉起兜底

- **WHEN** 未发现可用常驻实例
- **THEN** 应用进程内拉起 daemon 并连接，用户无需手动启动服务

#### Scenario: 连接失败

- **WHEN** daemon 连接建立失败（token 无效、进程崩溃等）
- **THEN** 界面展示失败原因与重试入口，应用不崩溃、不静默卡死

### Requirement: 导航与多面板布局

系统 SHALL 提供桌面级导航与多面板布局框架，支持主内容区与可切换的侧边导航，布局与交互参考 orca / paseo，为后续 change（会话管理、配置、高级面板）预留面板挂载点。

#### Scenario: 默认布局展示

- **WHEN** 应用启动完成
- **THEN** 展示侧边导航 + 主内容区（默认为对话界面）的布局

#### Scenario: 面板扩展

- **WHEN** 后续 change 注册新面板（如会话列表、配置页）
- **THEN** 导航中出现对应入口且主内容区可切换到该面板，无需改动骨架代码结构

### Requirement: UI 无关的会话编排客户端

系统 SHALL 提供不依赖任何 UI 框架的 daemon 会话编排客户端模块：命令通道（发起 turn、中断、审批应答）与事件通道（SSE 订阅、seq 跟踪、断线自动重连续传、失步后全量恢复回退）。GUI 的本地状态 MUST 只是事件流的投影，不各自维护会话真相副本。

#### Scenario: 断线自动恢复

- **WHEN** 事件流连接中断后恢复
- **THEN** 客户端按最后 seq 自动续传；失步时从会话存储全量恢复后重新订阅，界面向用户提示同步状态

#### Scenario: 多 UI 一致性

- **WHEN** GUI 与 TUI 同时连接同一 daemon 的同一会话
- **THEN** 双方呈现相同的对话内容与 turn 状态

### Requirement: 应用状态与错误兜底

系统 SHALL 将连接状态、同步状态（实时/重连中/失步恢复中）呈现给用户；事件处理异常 MUST NOT 导致应用崩溃。

#### Scenario: 同步状态可见

- **WHEN** 事件流处于重连或恢复中
- **THEN** 界面明确展示同步状态，恢复后自动回到实时模式
