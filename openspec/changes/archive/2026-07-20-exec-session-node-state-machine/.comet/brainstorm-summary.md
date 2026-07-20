# Brainstorm Summary: exec-session-node-state-machine

## 已确认方向（open 阶段）

- 外层 = node 级运行时（状态机 + verify 契约 + 回退），不含宏观编排
- node 契约由 agent 声明：`{goal, verify_commands, expected_files}`
- runtime 与 comet 解耦：零 comet 知识，失败返回给 agent（不通知特定调用方）
- 无 comet-adapter 代码模块：comet 通过 skill 指令引导 agent 使用 begin_node
- 复用内层 L1（SessionCoordinator + VerifyGate + CheckpointStore），不重做快照/回退
- SessionSource::Comet 仅来源标记，runtime 不据此分支

## 待决技术问题

### Q1: node 与 turn 的关系（已确认）
node 线性链（不嵌套），1 session = 1 node 链，node 内含多个 turn。node 的 verify/rollback 基于 node.start_turn 到当前 turn 的范围。

### Q2: failed -> running 转换触发方式（已确认）
方案 A：同 node 重试。agent 修正后重新调 verify_node（同 node，retry_count++）。延续内层 AutoRetry 语义。

### Q3: verify_node vs 内层 verify_and_complete（已确认）
两者共存。verify_and_complete 验证单 turn，verify_node 验证 node 跨多 turn。agent 按需选择。

### Q4: node 嵌套（已确认）
不支持嵌套（YAGNI），node 链线性。

## 技术设计草案（基于内层设计推断）

### 数据结构
```
NodeContract { goal: String, verify_commands: Vec<String>, expected_files: Vec<PathBuf> }
NodeStatus { Pending, Running, Verifying, Verified, Failed }
Node { id, contract, status, start_turn_id, retry_count, verify_log_path, created_at }
NodeStates (持久化到 session.json 的 node_states 字段)
```

### 状态机
```
pending -> running (begin_node)
running -> verifying (verify_node)
verifying -> verified (gate pass)
verifying -> failed (gate fail, retry_count < max)
failed -> running (agent 自修正后重调 verify_node)
failed -> session.failed (retry_count >= max, 升级)
```

### 工具
- begin_node(goal, verify_commands, expected_files) -> pending->running
- verify_node() -> running->verifying->verified/failed (委托内层 VerifyGate)
- rollback_node() -> 回退到上一个 verified node (委托 SessionCoordinator::rollback_to)

### 持久化
session.json 增加 node_states 字段：
```json
{
  "node_states": [
    { "id": "n1", "contract": {...}, "status": "verified", "start_turn_id": "turn-0", ... }
  ],
  "current_node": "n1"
}
```

### 复用内层
- VerifyGate: verify_node 委托执行 commands + 越界检测
- SessionCoordinator: rollback_node 委托 rollback_to(verified_node.start_turn 的前一个 turn)
- CheckpointStore: 不动（file capture/rewind 复用）
```
