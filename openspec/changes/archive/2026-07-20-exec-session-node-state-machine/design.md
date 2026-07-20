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
    Verifying,
    /// verify 通过，安全点
    Verified,
    /// verify 失败，agent 可自修正后重试
    Failed,
}
```

### 2.3 Node

```rust
/// node 记录
pub struct Node {
    pub id: NodeId,                // "n1", "n2"...
    pub contract: NodeContract,
    pub status: NodeStatus,
    /// node 开始时的 turn_id，verify/rollback 的范围起点
    pub start_turn_id: TurnId,
    /// verify 失败次数
    pub retry_count: u32,
    /// 验证日志路径
    pub verify_log_path: PathBuf,
    pub created_at: DateTime<Utc>,
}
```

### 2.4 NodeStates（持久化）

node 链持久化到 session.json 的 `node_states` 字段（内层已预留）。线性链，不嵌套。

```json
{
  "node_states": [
    {
      "id": "n1",
      "contract": { "goal": "...", "verify_commands": [...], "expected_files": [...] },
      "status": "verified",
      "start_turn_id": "turn-0",
      "retry_count": 0,
      "verify_log_path": "snapshots/es-a3f2c1/verify_log_n1.json",
      "created_at": "..."
    }
  ],
  "current_node": "n1"
}
```

session.json 原子写（tmp+rename），延续内层崩溃一致性。

## 3. 状态机

```
                     begin_node
    ──────────────────────────────────────>  Running
        │                                       │
        │                                  verify_node
        │                                       │
        │                                       ▼
        │                                  Verifying
        │                                   │      │
        │                            pass   │      │ fail (retry < max)
        │                                   ▼      │
        │                                Verified  │
        │                                          │
        │                              fail (retry >= max)
        │                                          │
        │                                          ▼
        │                                  session.failed
        │                                  (返回给 agent)
        │
   rollback_node
   (回退到上一个 verified node，之后的 node 删除)
```

### 3.1 转换规则

| 转换 | 触发 | 条件 |
|------|------|------|
| Pending -> Running | begin_node | 立即（Pending 是瞬态） |
| Running -> Verifying | verify_node | 当前 node 为 Running |
| Verifying -> Verified | gate pass | 所有 commands exit 0 + 无越界 |
| Verifying -> Failed | gate fail | retry_count < auto_retry_max |
| Failed -> Verifying | verify_node（重试） | retry_count < auto_retry_max |
| Failed -> session.failed | gate fail | retry_count >= auto_retry_max |
| Verified -> 新 node | begin_node | 当前 node 为 Verified |
| 任意 -> 回退 | rollback_node | 至少一个 verified node |

### 3.2 AutoRetry 语义

- `auto_retry_max` 默认 2（可配置 `ExecSessionSettings.auto_retry_max`）
- verify 失败**不自动回退**，把失败原因返回给 agent
- agent 在失败状态上自修正后重新调 `verify_node`（同 node，retry_count++）
- retry_count >= max 时 session.status = failed，返回给 agent 升级
- 回退是 agent 的显式动作（调 rollback_node），不是 verify 失败的副作用

延续内层设计文档 §3.3 原则：保留错误状态比抹掉更有信息量。

## 4. 工具

### 4.1 begin_node

```rust
/// 开始一个新的可验证工作单元
begin_node({
  "goal": "add memory clear command",
  "verify_commands": ["cargo test --test integration memory", "cargo clippy -- -D warnings"],
  "expected_files": ["src/cli.rs", "src/memory/list.rs"]
})
```

- **前置**：当前 node 为 Verified 或无 node（不能在 Running/Verifying/Failed 时开新 node）
- **效果**：创建新 node，status=Running，记录 start_turn_id（当前 turn）
- **返回**：`{ node_id, status: "running" }`
- **is_read_only()** = false

### 4.2 verify_node

```rust
/// 验证当前 node（委托内层 VerifyGate）
verify_node()
```

- **前置**：当前 node 为 Running（首次验证）或 Failed（重试）
- **效果**：委托内层 VerifyGate 执行 verify_commands + 越界检测
  - 越界范围：node.start_turn 到当前 turn 的改动并集 ⊆ expected_files
  - pass -> Verified
  - fail + retry_count < max -> Failed，返回失败原因
  - fail + retry_count >= max -> session.failed，返回给 agent
- **返回**：`{ status: "verified" | "failed", retry_count, failure_reason?, verify_log }`
- **is_read_only()** = false

### 4.3 rollback_node

```rust
/// 回退到上一个 verified node
rollback_node()
```

- **前置**：至少一个 verified node
- **效果**：委托 SessionCoordinator::rollback_to(第一个被删除 node 的 start_turn)
  - 定位最近的 verified node，其后所有 node 被删除
  - 回退目标 = 第一个被删除 node 的 start_turn（即 verified node 之后那个 node 的起始 turn）
  - 回退算法复用内层：git reset --hard + CheckpointStore::rewind + 删 untracked
  - current_node 回到 verified node
  - 示例：node-1(verified, start=turn-0), node-2(failed, start=turn-4) -> rollback_to(turn-4), 删 node-2, current=node-1
- **返回**：`{ rolled_back_to: node_id, removed_nodes: [node_ids] }`
- **is_read_only()** = false

### 4.4 与内层 verify_and_complete 的关系

两者共存，职责不同：

| 工具 | 层级 | 验证范围 | 用途 |
|------|------|---------|------|
| verify_and_complete | 内层 L1 | 单 turn | 轻量验证（单 turn 完成检查） |
| verify_node | 外层 | node 跨多 turn | 长程验证（整块工作验证） |

agent 可按需选择。verify_and_complete 标记 turn 级完成，verify_node 标记 node 级完成。

## 5. 复用内层

```
外层 NodeRuntime
  │
  ├── verify_node ──────> 内层 VerifyGate
  │                         - 执行 verify_commands（经 guardian + sandbox）
  │                         - 越界检测（参数化范围：node.start_turn ~ current_turn）
  │                         - 产出 verify_log
  │
  ├── rollback_node ────> 内层 SessionCoordinator::rollback_to
  │                         - 回退目标：第一个被删除 node 的 start_turn
  │                         - 算法：git reset --hard + CheckpointStore::rewind + 删 untracked
  │
  └── begin_node ───────> 内层 SessionCoordinator
                            - 关联当前 turn_id 作为 node.start_turn_id
```

内层完全不动：CheckpointStore / turn 级 verify_and_complete / undo 工具不变。

## 6. SessionHooks 扩展

```rust
pub trait SessionHooks {
    // 内层已有
    fn verify_fail(&self, ctx: &VerifyFailContext) -> VerifyFailAction { ... }

    // 外层新增（默认 no-op）
    /// node 转入 Running 前调用
    fn pre_node(&self, node: &Node) { }
    /// node 到达 Verified 或 Failed 后调用
    fn post_node(&self, node: &Node, result: &VerifyResult) { }
}
```

NoHooks 实现不变（默认 no-op）。调用方（如 comet plugin）可实现这些钩子观察 node 转换，runtime 不依赖。

## 7. 配置

`ExecSessionSettings` 扩展（`src/config/agent.rs`）：

```rust
pub struct ExecSessionSettings {
    pub enabled: bool,           // 已有，默认 true
    pub auto_retry_max: u32,     // 新增，默认 2
}
```

## 8. 模块结构

```
src/exec_session/
  mod.rs              (改: 导出新类型)
  coordinator.rs      (改: SessionCoordinator 增加 node_states 字段读写)
  session.rs          (改: SessionState 增加 node_states + current_node 字段)
  hooks.rs            (改: SessionHooks 增加 pre_node/post_node 默认实现)
  verify_gate.rs      (不改: VerifyGate 复用，越界范围参数化由 NodeRuntime 传入)
  git.rs              (不改)
  node.rs             (新: Node, NodeContract, NodeStatus, NodeStates)
  node_runtime.rs     (新: NodeRuntime 协调 node 生命周期)

src/tools/meta/
  begin_node.rs       (新: BeginNodeTool)
  verify_node.rs      (新: VerifyNodeTool)
  rollback_node.rs    (新: RollbackNodeTool)
```

## 9. 不做的事（YAGNI 边界）

- node 嵌套（线性链，1 session = 1 node 链）
- node 级 git refs 保护（复用 turn 级，内层已做）
- node 级快照（复用 turn 级 CheckpointStore）
- 跨会话 resume（#2 change，不在本次）
- comet-adapter 代码模块（已消解，comet skill 指令维护）
- 非命令式 verify（第一版只支持命令式，延续内层）
- 自动回退（rollback 只由 agent 主动调）
- node 并行（线性链，一次只有一个 current node）

## 10. 风险与权衡

| 风险 | 缓解 |
|------|------|
| node_states 持久化失败 | session.json 原子写（tmp+rename），失败 fail fast 返回 agent |
| verify_node 越界范围计算不准 | actual = node.start_turn ~ current_turn 的 CheckpointStore manifest + git diff + untracked 三源并集 |
| rollback 误删 agent 工作 | rollback 是 agent 显式调用；回退到 verified node（安全点）；guardian 提示 |
| auto_retry_max 过小导致频繁升级 | 可配置，默认 2；agent 也可选 rollback 换方向 |
| 与内层 verify_and_complete 语义重叠 | 两者共存，层级不同（turn vs node），agent 按需选择 |
| 非 git 项目无越界检测 | 降级到纯 CheckpointStore manifest（file 改动仍可检测） |

## 11. 验收标准

- [ ] NodeContract / NodeStatus / Node 数据结构实现，持久化到 session.json node_states
- [ ] node 状态机：pending->running->verifying->verified/failed，转换原子持久化
- [ ] begin_node / verify_node / rollback_node 三工具实现 Tool trait，is_read_only()=false
- [ ] verify_node 委托内层 VerifyGate，越界范围参数化（node.start_turn ~ current_turn）
- [ ] rollback_node 委托 SessionCoordinator::rollback_to，删除 verified node 之后的 node
- [ ] AutoRetry：failed -> verifying（retry_count++），超限 session.failed 返回 agent
- [ ] SessionHooks 增加 pre_node/post_node 默认 no-op
- [ ] ExecSessionSettings 增加 auto_retry_max（默认 2）
- [ ] exec-session disabled 时三工具不注册
- [ ] 解耦不变式：src/exec_session/ 无 "comet" 字符串（除枚举变体与注释）
- [ ] 内层 L1 现有行为不受影响（CheckpointStore / verify_and_complete / undo 不变）
- [ ] e2e 测试：完整 node 生命周期（begin -> work -> verify pass -> begin next -> verify fail -> retry -> verify pass -> rollback）
