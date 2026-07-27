# Plan: /undo 交互式回滚

## Plan Header
- Feature: `/undo` interactive rollback (turn picker + scope picker: code/chat/both)
- Spec: `docs/superpowers/specs/undo-interactive-rollback.md`
- Date: 2026-07-27
- Scope: TUI REPL only

## File Structure

**新增**:
- `src/tui/components/turn_picker.rs` - TurnPicker popup 组件
- `src/tui/components/undo_scope_picker.rs` - UndoScopePicker popup 组件

**修改**:
- `src/tui/app/types.rs` - 新增 `TurnRecord` / `UndoScope` / `UndoReport`
- `src/tui/app/mod.rs` - App 加 `turn_records` 字段 + `undo_to_turn` 方法 + `/undo` 入口
- `src/tui/app/turn.rs` - `spawn_agent_turn` 记录 turn、turn 完成填 `message_end_idx`/`file_count`
- `src/tui/app/compaction.rs`(或 loop 压缩处) - compaction 同步 `turn_records`
- `src/runtime/command.rs` - `/undo` 注册为 builtin（TUI 侧构造 builtins 时加入）
- `src/context/session.rs` - `Session` 加 `turn_records` 字段（`#[serde(default)]` 向后兼容）
- `src/context/mod.rs` - `SessionUiMessage` 旁导出 `TurnRecord`（若放 context 层）
- `src/tui/client.rs` - `save_session`/`load_session` 携带 `turn_records`
- `src/daemon/models.rs` - `SessionResponse` 加 `turn_records`

## Tasks

### Task 1: TurnRecord 数据结构 + Session 持久化
**TDD**:
1. 写测试 `src/context/session.rs` tests：`Session` 序列化含 `turn_records`；旧 session（无 turn_records）反序列化为空 vec
2. 验证 `cargo test context::session` 失败
3. 实现：`TurnRecord { turn_id, created_at, user_summary, checkpoint_turn_id, message_end_idx, file_count }`（`#[serde(default)]`）；`Session` 加 `#[serde(default)] turn_records: Vec<TurnRecord>`
4. 验证测试通过
5. commit: `feat(tui): add TurnRecord struct and persist in Session`

### Task 2: spawn_agent_turn 记录 turn
**TDD**:
1. 写测试 `src/tui/app/turn.rs` tests：模拟 spawn_agent_turn，验证 `turn_records` 末尾新增一条（turn_id/created_at/user_summary/checkpoint_turn_id 正确）；turn 完成后 `message_end_idx == conversation_history.len()`、`file_count` 正确
2. 验证失败
3. 实现：`App.turn_records: Vec<TurnRecord>`；`spawn_agent_turn` 开头 push 一条（end_idx/file_count 先置 0）；turn 完成（agent 响应结束）回调里填 `message_end_idx = conversation_history.len()`、`file_count`（从 checkpoint manifest 或 mutating 工具计数）
4. 验证通过
5. commit: `feat(tui): record TurnRecord on spawn_agent_turn`

### Task 3: compaction 同步 turn_records
**TDD**:
1. 写测试：构造 turn_records + 触发 compaction（boundary 推进），验证 boundary 之前的 turn 被丢弃、boundary 之后 turn 的 `message_end_idx` 减去被压缩前缀长度
2. 验证失败
3. 实现：在 compaction 成功后（`loop_.rs` CompactionStarted 分支或 TUI 对应处）调用 `sync_turn_records_on_compaction(&mut turn_records, boundary, compressed_len)`：retain boundary 之后、调整 end_idx
4. 验证通过
5. commit: `feat(tui): sync turn_records on compaction`

### Task 4: undo_to_turn 原语
**TDD**:
1. 写测试 `src/tui/app/mod.rs` tests：
   - Chat scope：`undo_to_turn(N, Chat)` 后 `conversation_history.len() == turn_N.message_end_idx`、`committed_messages` 同步截断、`turn_records` 保留 0..=N
   - Code scope：`undo_to_turn(N, Code)` 调用 `CheckpointStore.rewind(turn_{N+1}.checkpoint_turn_id)`；turn N 之后无 checkpoint 则 `code_skipped=true`
   - Both scope：先 Code 后 Chat
   - turn N 是最后一个：无操作
2. 验证失败
3. 实现：`App::undo_to_turn(&mut self, turn_id, scope) -> Result<UndoReport>`；`UndoScope { Code, Chat, Both }`；`UndoReport { files_restored, messages_truncated, turns_removed, code_skipped }`
4. 验证通过
5. commit: `feat(tui): implement undo_to_turn primitive`

### Task 5: /undo slash command 注册
**TDD**:
1. 写测试 `src/runtime/command.rs` tests：`router.route("/undo")` 返回 `BuiltIn`
2. 验证失败
3. 实现：TUI 侧构造 `CommandRouter::new(builtins)` 时加入 `"undo"`；TUI 输入处理 `/undo` -> 打开 TurnPicker（Esc 关闭）
4. 验证通过
5. commit: `feat(tui): register /undo as builtin slash command`

### Task 6: TurnPicker popup 组件
**TDD**:
1. 写测试 `src/tui/components/turn_picker.rs` tests：给定 turn_records + compaction_boundary，渲染列表只含 boundary 之后 turn；上下键移动选中；回车返回选中的 turn_id；Esc 返回 None
2. 验证失败
3. 实现：`TurnPicker` 组件（参考 `SessionState` popup），列表项 `#idx time summary (N files)`，有 checkpoint 标注
4. 验证通过
5. commit: `feat(tui): add TurnPicker popup component`

### Task 7: UndoScopePicker popup 组件
**TDD**:
1. 写测试 `src/tui/components/undo_scope_picker.rs` tests：三选项（代码/聊天/两者），默认选"两者"；上下键移动；回车返回 `UndoScope`；Esc 取消
2. 验证失败
3. 实现：`UndoScopePicker` 组件，默认 Both
3. 验证通过
4. commit: `feat(tui): add UndoScopePicker popup component`

### Task 8: 集成 /undo 流程 + UndoReport 显示
**TDD**:
1. 写测试：`/undo` -> TurnPicker 选 turn -> UndoScopePicker 选 scope -> `undo_to_turn` -> 显示 UndoReport；边界：最后一个 turn / 无 checkpoint / compaction 后 turn
2. 验证失败
3. 实现：TUI 状态机串联 TurnPicker -> UndoScopePicker -> undo_to_turn -> 渲染 UndoReport（files_restored/messages_truncated/turns_removed/code_skipped）
4. 验证通过
5. commit: `feat(tui): wire /undo flow end-to-end`

### Task 9: 回滚后状态调整 + UI 刷新
**TDD**:
1. 写测试：回滚后 `turn_count` 递减、`current_turn_id` 若被移除则清空；`committed_messages` 截断后 chat 视图重绘正确
2. 验证失败
3. 实现：`undo_to_turn` 末尾调整 `turn_count`/`current_turn_id`；触发 UI 重绘（committed_messages 变更通知）
4. 验证通过
5. commit: `feat(tui): adjust turn_count/current_turn_id and refresh UI after undo`

## Self-Review
- ✅ 每个 task 是最小可测试单元，TDD
- ✅ 依赖顺序：Task 1（结构）-> 2（记录）-> 3（compaction 同步）-> 4（原语）-> 5/6/7（命令+UI）-> 8（集成）-> 9（收尾）
- ✅ 前置依赖（消息持久化）已确认存在，Task 1 复用 Session
- ⚠️ 风险点：Task 2 的 `file_count` 来源（checkpoint manifest 文件数 vs mutating 工具计数）需实现时确认；Task 3 compaction 触发点在 TUI 侧的精确位置需确认（daemon 路径 vs TUI 路径）
- ⚠️ Task 4 Code 回滚的 checkpoint 选择（turn N+1 第一个有 checkpoint 的 turn）逻辑需仔细测试中间 turn 无 checkpoint 的场景

## Execution Handoff
建议 **inline 执行**（主会话顺序执行 Task 1-9），因为：
- task 间依赖紧密（结构 -> 记录 -> 原语 -> UI -> 集成）
- 多处改同一文件（mod.rs/turn.rs），并行易冲突
- 每个 task 有测试验证，inline 便于即时 review

若要加速，Task 6/7（两个 popup 组件）可并行（独立文件），但建议先完成 Task 1-5。
