# /undo 交互式回滚 — 设计文档

> 状态：设计待审 | 日期：2026-07-27 | 模块：tui / checkpoint

## 一、需求

用户在 TUI REPL 输入 `/undo`，弹出 turn 选择框，选中一个 turn 后选择回滚类型（代码 / 聊天 / 两者），执行回滚。

## 二、语义（已与用户确认）

| 维度 | 决策 |
|------|------|
| 聊天回滚 | 回到所选 turn **完成时**：保留该 turn 及之前的全部对话，删除其后的消息 |
| 代码回滚 | 仅文件级（`CheckpointStore.rewind`），不碰 git |
| 所选 turn 之后无 checkpoint | 代码回滚跳过（`code_skipped=true`），仅回滚聊天 |
| compaction | popup 只列 `compaction_boundary` 之后的 turn（压缩前的原始消息已被 summary 替代，不可恢复） |
| 作用域 | TUI REPL only；daemon/CLI 后续扩展 |
| redo | 不支持（回滚即删除 turn/消息，不可逆） |

## 三、现状分析（关键发现）

- **TUI 聊天消息**：`App.conversation_history: Arc<Mutex<Vec<ChatMessage>>>`，`HistoryStore::replace()` 可整体截断（`src/agent/runtime/history.rs`）
- **TUI turn**：仅 in-memory（`current_turn_id`、`turn_count`），**无持久化 turn 链**（`src/tui/app/turn.rs`）
- **checkpoint**：per-turn 文件快照，`CheckpointStore` 有 turn 列表 + `rewind()`（`src/tools/checkpoint_store.rs`）
- **exec_session.SessionState.turns**：comet/agent-self 流程专用，**TUI REPL 不走**（`src/exec_session/session.rs`）
- **slash command**：`CommandRouter`（builtins + workflow），`/undo` 可注册为 builtin（`src/runtime/command.rs`）
- **TUI popup**：`SessionState` 会话选择 popup 可作参考（`src/tui/components/session.rs`）

**核心缺口**：TUI REPL 缺少 turn 记录持久化 + 消息边界，`/undo` 选 turn 需补这块。

## 四、设计

### 4.1 TurnRecord（TUI 专用，填补缺口）

```rust
struct TurnRecord {
    turn_id: String,
    created_at: String,
    user_summary: String,       // 用户输入首句，popup 展示
    checkpoint_turn_id: String, // 该 turn 开始前的文件快照（pre-edit）
    message_end_idx: usize,     // 该 turn 完成时 conversation_history 的长度
    file_count: usize,          // 该 turn 编辑的文件数（0 = 纯对话）
}
```

- `App` 新增 `turn_records: Vec<TurnRecord>`
- `spawn_agent_turn` 开始时：记 `checkpoint_turn_id`（本 turn pre-edit 快照）、`turn_id`、`created_at`、`user_summary`；turn 完成（agent 响应结束）时填 `message_end_idx = conversation_history.len()`、`file_count`
- `turn_records` 与 `conversation_history` 一起持久化到 session 存储

### 4.2 `/undo` 交互流程

1. 用户输入 `/undo`（注册为 `CommandRouter` builtin）
2. TUI 弹 `TurnPicker` popup：列表 `#编号  时间  首句摘要  (N files)`，有 checkpoint 标注；只列 `compaction_boundary` 之后的 turn
3. 上下选 turn，回车确认，Esc 取消
4. 弹 `UndoScopePicker`：`代码 / 聊天 / 两者` 三选一
5. 回车执行，显示 `UndoReport`

### 4.3 `undo_to_turn` 原语（TUI App 方法）

```rust
enum UndoScope { Code, Chat, Both }
struct UndoReport {
    files_restored: Vec<PathBuf>,
    messages_truncated: usize,
    turns_removed: usize,
    code_skipped: bool,
}
fn undo_to_turn(&mut self, turn_id: &str, scope: UndoScope) -> Result<UndoReport>
```

语义：回滚到 turn N **完成时** = 保留 turn N，撤销 turn N+1 及之后。

- **Chat**：`conversation_history.truncate(turn_N.message_end_idx)` + `committed_messages` 同步截断（UI 显示消息，按 turn N 边界对齐）+ 移除 turn N **之后**的 `turn_records`（保留 turn N）
- **Code**：撤销 turn N 之后的文件改动 = 找 turn N **之后第一个有 checkpoint 的 turn M**（M > N），`CheckpointStore.rewind(M.checkpoint_turn_id)`（M 开始前的快照 = turn N 完成时的文件状态）。若 turn N 之后无任何 checkpoint，`code_skipped=true`
- **Both**：先 Code 后 Chat
- turn N 已是最后一个 turn：回滚无操作（已是完成时）

> 代码回滚用 turn N+1 的 checkpoint（= turn N 完成时），而非 turn N 自己的 checkpoint（那是 turn N 开始前）。

### 4.4 compaction 限制

`TurnPicker` 只列出 `compaction_boundary` 之后的 turn。压缩前的 turn 原始消息已被 `compacted_summary` 替代，无法恢复，故不列出。

### 4.5 compaction 与 turn_records 同步

compaction 发生时（`compaction_boundary` 推进、history 前缀被 summary 替代），`turn_records` 须同步：
- 丢弃 `compaction_boundary` 之前的 turn 记录
- boundary 之后 turn 的 `message_end_idx` 减去被压缩的前缀长度（重新基于压缩后 history 计数），否则索引错位

## 五、边界情况

- 所选 turn 是最后一个：聊天和代码都无操作
- 所选 turn 之后无 checkpoint（纯对话）：代码跳过
- 回滚后无 redo（不可逆）
- compaction 后的 turn 不在 popup 列出
- 回滚后 `compaction_boundary`/`compacted_summary` 仍有效（只回滚 boundary 之后，不动 summary）
- 回滚后 `turn_count` 递减、`current_turn_id` 若属于被移除 turn 则清空

## 六、实现要点

1. `App` 新增 `turn_records` 字段 + 持久化（与 conversation_history 同存储）
2. `TurnPicker`/`UndoScopePicker` popup 组件（参考 `SessionState`）
3. `/undo` 注册为 `CommandRouter` builtin，TUI 层处理
4. `undo_to_turn` 在 `App` 层实现
5. checkpoint 选择逻辑：turn N 之后第一个有 checkpoint 的 turn
6. `TurnPicker` 编号 = `turn_records` 索引+1；`UndoScopePicker` 默认选"两者"

## 七、待实现时确认的点

- **TUI `conversation_history` 持久化（前置依赖 / 可行性风险）**：确认现有 session 存储是否已含消息持久化。若无，需先实现消息持久化，`turn_records` 持久化依赖于此。这是整个方案的可行性关键，实现前必须先确认。
- `compaction_boundary` 在 TUI `App` 的访问路径
- `file_count` 统计来源（checkpoint manifest 的文件数？或 mutating 工具计数）
