## Why

ExecutionSession 内层（L1, SessionCoordinator + turn 级 verify-gate）已完成并接入主 agent loop，提供单 turn 的可中断/可回滚/可验证能力。但长程自主任务缺乏 **node 级**的执行运行时：agent 能在一个可验证工作单元内跨多个 turn 试错、验证、失败自修正、必要时回退到上一个已验证节点。

现状缺口：
- 内层 verify-gate 是 **turn 级**（单 turn 完成是否可证明），没有 node 级聚合验证
- comet task 是 markdown 描述，**没有结构化 verify 契约**，verify 依赖人工/agent 判断
- task 失败靠 `systematic-debugging` skill + 人工，**没有 runtime 层的 AutoRetry + 状态机 + 回退**
- agent 没有办法声明"我要开始一个带验证契约的工作单元"并在失败时回退到上一个安全点

本 change 定义外层的 node 级运行时：node 契约 + 状态机 + 工具，作为长程自主性闭环（C 方案）的运行时地基。这是外层三块中的第一块（后续 #2 跨会话 resume、#3 已消解为 comet skill 指令维护）。

## What Changes

- 新增 **node 契约 schema**：`{goal, verify_commands, expected_files}`，由 agent 调 `begin_node` 时声明，runtime 信任声明并执行 verify（延续内层 verify_and_complete 的"agent 传 commands, runtime 亲自跑"模式）
- 新增 **node 状态机**：`pending -> running -> verifying -> verified / failed`，状态持久化到 session.json 的 `node_states` 字段（内层已预留）
- 新增三个**外层工具**：
  - `begin_node(goal, verify_commands, expected_files)` -> 创建 node，状态 pending->running
  - `verify_node()` -> 对当前 running node 执行 verify-gate（复用内层 VerifyGate），通过则 verified，失败则 failed + AutoRetry
  - `rollback_node()` -> 回退到上一个 verified node（复用内层 rollback 算法：git reset + CheckpointStore::rewind + 删 untracked）
- 新增 **AutoRetry 机制**：node verify 失败不自动回退，把失败原因返回给 agent 允许自修正（max N 次），超限则 session.status=failed 升级给调用方
- **失败结果返回给 agent**（不通知特定调用方）：外层不关心是 comet 还是 agent-self，只把 verify 结果返回给 agent，升级决策在 agent 侧
- 复用内层 `SessionCoordinator` + `VerifyGate` + `CheckpointStore`，不重做快照/回退机制
- `SessionHooks` trait 预留 `pre_node` / `post_node` 钩子（内层已预留），外层默认 `NoHooks` 实现

## Capabilities

### New Capabilities

- `exec-session-node-runtime`: node 级执行运行时--agent 可声明带验证契约的工作单元（node），runtime 管理 node 状态机（pending->running->verifying->verified/failed），提供失败自修正（AutoRetry）和回退到上一个 verified node 的能力。node 契约由 agent 声明，runtime 与任何流程编排 skill（含 comet）解耦。

### Modified Capabilities

- `agent-runtime-engine`: agent loop 在 turn 边界已接入内层 SessionCoordinator；本 change 在其上增加 node 级工具（begin_node/verify_node/rollback_node），node 状态机驱动内层 turn 链的分组与验证聚合。

## Impact

- `src/exec_session/node.rs` (新): `Node` 结构、`NodeContract`（goal + verify_commands + expected_files）、`NodeStatus` 状态机、`NodeStates` 持久化到 session.json
- `src/exec_session/node_runtime.rs` (新): `NodeRuntime` 协调 node 生命周期，调用内层 `SessionCoordinator`（begin_turn/end_turn/rollback_to）和 `VerifyGate`
- `src/exec_session/coordinator.rs` (改): `SessionCoordinator` 增加 node 状态管理（`node_states` 字段读写，`current_node` 游标），复用现有 turn 链与 git refs 保护
- `src/exec_session/hooks.rs` (改): `SessionHooks` trait 增加 `pre_node` / `post_node` 默认实现（NoHooks 不变）
- `src/exec_session/mod.rs` (改): 导出新类型
- `src/tools/meta/begin_node.rs` (新): `BeginNodeTool` 实现 `Tool` trait
- `src/tools/meta/verify_node.rs` (新): `VerifyNodeTool` 实现 `Tool` trait，调用 `VerifyGate`
- `src/tools/meta/rollback_node.rs` (新): `RollbackNodeTool` 实现 `Tool` trait，调用 `SessionCoordinator::rollback_to`
- `src/tools/mod.rs` (改): 注册三个新工具
- `src/config/agent.rs` (改): `ExecSessionSettings` 增加 node 相关配置（`auto_retry_max`）
- 不破坏内层 L1 现有行为（CheckpointStore / turn 级 verify_and_complete / undo 工具不变）
- 解耦不变式延续：`src/exec_session/` 代码无 "comet" 字符串（除 SessionSource::Comet 枚举变体与注释）
