# Comet Design Handoff

- Change: exec-session-node-state-machine
- Phase: design
- Mode: compact
- Context hash: a83f7661249e2ba2030d1dfd45f7095b5bc6019458b707a614b0b56ba7de8a03

Generated-by: comet-handoff.sh

OpenSpec remains the canonical capability spec. This handoff is a deterministic, source-traceable context pack, not an agent-authored summary.

## openspec/changes/exec-session-node-state-machine/proposal.md

- Source: openspec/changes/exec-session-node-state-machine/proposal.md
- Lines: 1-49
- SHA256: f7751f0f36f68a2425f6a21cb25d489534676e0303e834058a350871cff1b63c

```md
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
```

## openspec/changes/exec-session-node-state-machine/design.md

- Source: openspec/changes/exec-session-node-state-machine/design.md
- Lines: 1-342
- SHA256: 6e1e86220e14950bae83a788afed3a03dc365273346af006cb95f8fabd5b7f05

[TRUNCATED]

```md
---
comet_change: exec-session-node-state-machine
role: technical-design
canonical_spec: openspec
---

# Design Doc: ExecutionSession 外层 -- Node 状态机

**日期**: 2026-07-20
**状态**: design
**关联**: `docs/superpowers/specs/2026-07-19-exec-session-inner-layer-design.md`（内层 L1）
**change**: `openspec/changes/exec-session-node-state-machine`

## 1. 定位

外层是 ExecutionSession 的 node 级运行时，在内层 L1（SessionCoordinator + turn 级 verify-gate）之上增加 **node 聚合层**：把"为了一个可验证目标跑的多个 turn"打包成可验证、可回退、可重试的单元。

```
comet (流程编排层, WHAT, 可选插件) ── 不动
        │ build 阶段通过 skill 指令引导 agent 使用 begin_node
        ▼
ExecutionSession 外层 (HOW, 本次) ── node 级运行时
  node 契约 + 状态机 + AutoRetry + 回退
        │ 复用
        ▼
ExecutionSession 内层 (L1, 已完成) ── turn 级运行时
  SessionCoordinator + verify-gate
        │ 复用
        ▼
CheckpointStore / sandbox / guardian (机制层, 现有) ── 不动
```

### 1.1 核心概念

- **turn** = agent 的一轮对话（一次 LLM 调用 + 工具执行 + 回复），内层 L1 管
- **node** = 为了一个可验证目标进行的一组 turn，外层管

node 给了 turn 没有的能力：

| 能力 | 只有 turn | 有 node 后 |
|------|----------|-----------|
| 验证范围 | 单 turn | node 跨的多个 turn 的改动一起验 |
| 回退粒度 | 单 turn（undo） | 回退到上一个安全点（verified node） |
| 失败重试 | 无 | 同 node 内 retry，工作不丢 |
| 进度可见性 | 一堆离散 turn | "node-1 完成, node-2 进行中" |
| 越界检测 | 单 turn 改动 | node 范围改动 ⊆ expected_files |

### 1.2 解耦原则（延续内层不变式）

- runtime 零 comet 知识：`src/exec_session/` 代码无 "comet" 字符串（除 `SessionSource::Comet` 枚举变体与注释）
- node 契约由 agent 声明，runtime 信任声明并亲自执行 verify（不信任 agent 贴的结果）
- verify 失败返回给 agent，不通知特定调用方（agent 根据自身流程决定升级）
- 无 comet-adapter 代码模块：comet 通过 skill 指令引导 agent 使用 begin_node

## 2. 数据结构

### 2.1 NodeContract

```rust
/// agent 声明的 node 契约。runtime 信任声明并亲自执行 verify。
pub struct NodeContract {
    /// 人类可读目标描述
    pub goal: String,
    /// runtime 亲自执行的验证命令（经 guardian 审查 + sandbox 执行）
    pub verify_commands: Vec<String>,
    /// 越界检测边界。空列表 = 不检测越界
    pub expected_files: Vec<PathBuf>,
}
```

### 2.2 NodeStatus

```rust
/// node 状态机
pub enum NodeStatus {
    /// 已创建未开始（瞬态，begin_node 后立即转 Running）
    Pending,
    /// 进行中，agent 在此 node 内工作
    Running,
    /// 正在执行 verify_node
```

Full source: openspec/changes/exec-session-node-state-machine/design.md

## openspec/changes/exec-session-node-state-machine/tasks.md

- Source: openspec/changes/exec-session-node-state-machine/tasks.md
- Lines: 1-3
- SHA256: cb56b18bce4aa292e20242a8ef3be39d4360c8509bec407ec727b5da4b016744

```md
# Tasks

Tasks will be defined in build phase based on the design doc.
```

## openspec/changes/exec-session-node-state-machine/specs/exec-session-node-runtime/spec.md

- Source: openspec/changes/exec-session-node-state-machine/specs/exec-session-node-runtime/spec.md
- Lines: 1-140
- SHA256: 828a8792dd9920e8c86b2b0f823ba14f03d9ebcda4115ff9844bb978ce353aef

[TRUNCATED]

```md
## ADDED Requirements

### Requirement: Node contract schema

The exec-session outer layer SHALL define a node contract that an agent declares when starting a verifiable work unit. A node contract consists of a human-readable goal, a list of verify commands (executed by the runtime, never trusted from agent-asserted results), and a list of expected changed files (for out-of-bounds detection). The runtime SHALL store the contract in `session.json` under a `node_states` field so node state survives across turns within a session.

#### Scenario: Agent declares a node with full contract

- **WHEN** the agent invokes `begin_node` with `{goal: "add memory clear command", verify_commands: ["cargo test --test integration memory", "cargo clippy -- -D warnings"], expected_files: ["src/cli.rs", "src/memory/list.rs"]}`
- **THEN** the runtime SHALL create a node record with status `running`
- **AND** the node contract (goal, verify_commands, expected_files) SHALL be persisted to `session.json` under `node_states`
- **AND** the node SHALL be linked to the current turn chain in the session

#### Scenario: Node contract without expected_files

- **WHEN** the agent invokes `begin_node` with `{goal: "...", verify_commands: ["cargo test"], expected_files: []}`
- **THEN** the runtime SHALL create the node with an empty expected_files list
- **AND** out-of-bounds detection SHALL be skipped (empty expected means no boundary constraint)

#### Scenario: Node contract persisted across turns

- **WHEN** a node is created in turn N and the agent continues to turn N+1 without completing the node
- **THEN** the node SHALL remain in `running` status across turns
- **AND** `session.json` SHALL retain the node record so it is available for verify or rollback in any subsequent turn

### Requirement: Node state machine

The runtime SHALL manage a node state machine with states `pending`, `running`, `verifying`, `verified`, and `failed`. A node transitions `pending -> running` on creation, `running -> verifying` when `verify_node` is invoked, `verifying -> verified` on verify success, and `verifying -> failed` on verify failure. A `failed` node MAY transition back to `running` for self-correction (AutoRetry) up to a configured maximum. Transitions SHALL be persisted atomically to `session.json`.

#### Scenario: Node transitions to verified on successful verify

- **WHEN** the agent invokes `verify_node` on a `running` node and all verify commands exit 0 and no out-of-bounds files are detected
- **THEN** the node status SHALL transition `running -> verifying -> verified`
- **AND** `session.json` SHALL be atomically updated with the new status
- **AND** the verify_log SHALL record the successful attempt

#### Scenario: Node transitions to failed on verify failure

- **WHEN** the agent invokes `verify_node` on a `running` node and a verify command exits non-zero OR out-of-bounds files are detected
- **THEN** the node status SHALL transition `running -> verifying -> failed`
- **AND** the workspace changes SHALL be preserved (no automatic rollback)
- **AND** the failure reason (which command failed, or the out-of-bounds file list) SHALL be returned to the agent

#### Scenario: Failed node self-correction within AutoRetry limit

- **WHEN** a node is `failed` and the retry count is below `auto_retry_max` (default 2)
- **THEN** the agent MAY invoke `begin_node` again or continue working and re-invoke `verify_node`
- **AND** the node SHALL transition `failed -> running` (self-correction path)
- **AND** the workspace changes from the failed attempt SHALL be preserved so the agent can inspect and fix them

#### Scenario: Failed node exceeds AutoRetry limit

- **WHEN** a node has failed more than `auto_retry_max` times
- **THEN** the session status SHALL become `failed`
- **AND** the runtime SHALL return the failure to the agent (not to any specific orchestration skill)
- **AND** the agent SHALL decide the escalation action based on its current flow (comet verify-failure handling, or user report in agent-self mode)

### Requirement: Node-level verify-gate reuses inner VerifyGate

The `verify_node` tool SHALL delegate command execution and out-of-bounds detection to the existing inner-layer `VerifyGate`. The runtime SHALL NOT re-implement command execution, guardian review, or sandbox execution. Out-of-bounds detection SHALL combine the inner layer's CheckpointStore manifest + git diff + untracked sources, scoped to the node's turn span.

#### Scenario: verify_node delegates to inner VerifyGate

- **WHEN** the agent invokes `verify_node`
- **THEN** the runtime SHALL call the inner `VerifyGate` with the node's `verify_commands` and `expected_files`
- **AND** each command SHALL pass through guardian review and sandbox execution (same as `exec_command`)
- **AND** the verify result (pass/fail, failure reason, verify_log) SHALL be produced by the inner `VerifyGate`

#### Scenario: Out-of-bounds detection scoped to node turn span

- **WHEN** `verify_node` checks for out-of-bounds changes
- **THEN** `actual_changed_files` SHALL be computed from turns belonging to the current node (node start turn to current turn)
- **AND** the check SHALL be `actual_changed_files ⊆ expected_files`
- **AND** out-of-bounds files (actual not in expected) SHALL cause verify failure with the out-of-bounds list returned to the agent

### Requirement: Node rollback to last verified node

The `rollback_node` tool SHALL roll back to the most recent `verified` node by delegating to the inner `SessionCoordinator::rollback_to` with the verified node's starting turn. The rollback algorithm (git reset --hard if head changed + CheckpointStore::rewind + delete new untracked) SHALL be reused from the inner layer without modification. Rollback SHALL only be triggered by explicit agent invocation, never automatically on verify failure.

#### Scenario: Rollback to last verified node
```

Full source: openspec/changes/exec-session-node-state-machine/specs/exec-session-node-runtime/spec.md

