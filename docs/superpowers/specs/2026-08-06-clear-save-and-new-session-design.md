# /clear 保存当前会话并开启新会话 - 设计文档

- 日期：2026-08-06
- 状态：已批准（待实现）
- 范围：TUI /clear 命令行为改造

## 1. 动机

当前 /clear（src/tui/app/input.rs:41-86）只清空当前会话的对话内容（UI 消息、conversation_history、agent generation），但**保持同一个 session_id**。这导致：清空后的对话在下次保存时仍覆盖到同一会话文件，原会话历史被丢失；用户也没有"开新会话继续工作、旧会话留存"的语义。

目标：/clear 改为**保存当前会话 + 创建并切换到新会话**，让旧会话成为可恢复的历史记录，新会话从空白开始。

## 2. 当前行为

/clear 同步部分：
- 清空 committed_messages、streaming_content、pending_inputs
- 取消当前 turn，phase = Idle，suppress_phase_updates = true
- 异步清空 conversation_history

异步部分（两个独立 spawn）：
- 用**当前 session_id** 调 reset_agent_generation（input.rs:68-85）

关键：session_id 始终不变。

## 3. 新行为

/clear 触发后：
1. **保存当前会话**：用旧 session_id 把完整 history + UI messages 持久化到 daemon（PUT /api/v1/sessions/:id）。
2. **创建新会话**：POST /api/v1/sessions，拿回新 SessionResponse（含新 id/name）。
3. **切换**：主线程把 session_id/session_name 切到新会话，并用新 session_id 调 reset_agent_generation，确保新会话的子代理 generation 干净。
4. UI 与 history 已在同步阶段清空，新会话从空白开始。

## 4. 设计细节

### 4.1 主流程（方案 A：同步 clone+清空，异步网络 IO+切换）

/clear 分支改为：

**同步阶段（主线程，立即返回）**
- 空会话短路：若 committed_messages.is_empty()，push 系统消息"会话为空，无需清除"，直接 return（不创建新会话，避免空会话堆积）。
- clone 旧会话快照：
  - old_id = self.session_id.clone()、old_name = self.session_name.clone()
  - lock conversation_history，调 sanitize_tool_call_pairing，clone 出 old_history，**随后 clear** history（保证 /clear 后立即干净，新消息不带旧上下文）
  - 把 committed_messages 映射为 old_ui_messages，**随后 clear** committed_messages
- 清空 streaming_content、pending_inputs，phase = Idle，suppress_phase_updates = true，cancel_current_turn()，sandbox_bypassed_session = false，scroll_offset = 0，user_scrolled = false
- spawn 异步块（携带上面 clone 的旧数据 + client/history/tx 句柄）

**异步阶段（spawn）**
1. client.save_session(&old_id, &old_name, &old_history, &old_ui_messages).await
   - 失败：记录 tracing::warn!，继续（best-effort），通过 event_tx 发 AppEvent::SystemNotice("⚠️ 上一会话保存失败，已切换到新会话")
2. client.create_session(None).await（默认名，与启动一致）
   - 成功：发 AppEvent::SessionSwitched { id, name }
   - 失败：发 AppEvent::SystemNotice("⚠️ 创建新会话失败：{error}")，此时 session_id 仍是旧的、history 已清空——用户可继续在已清空的旧会话上工作（降级，不阻塞）

### 4.2 新事件 AppEvent::SessionSwitched

在 types.rs 的 AppEvent 枚举新增：

    /// /clear 创建了新会话；主线程采纳新 id/name 并 reset generation。
    SessionSwitched {
        id: String,
        name: String,
    },

**事件处理（event.rs）**：设置 self.session_id = id、self.session_name = name，然后复用现有 reset_agent_generation 路径（用新 session_id），保证新会话子代理 generation 干净。session_exit_saved 重置为 false（新会话尚未退出保存）。

**穷尽匹配更新**：src/tui/util.rs:425-460 的"不改变 phase"的事件分组 match 需加入 SessionSwitched { .. }（切换不改变 phase，仍为 Idle）。

### 4.3 边界处理

| 场景 | 处理 |
|------|------|
| 当前会话无消息（committed_messages 为空） | no-op + 系统消息"会话为空，无需清除"；不创建新会话 |
| 保存当前会话失败 | best-effort，仍创建新会话并切换 + SystemNotice 警告 |
| 创建新会话失败 | 不切换 session_id（保持旧 id），history 已清空，用户可在已清空的旧会话上继续；SystemNotice 警告 |
| 新会话命名 | create_session(None) 默认名；首条消息后由现有标题逻辑生成 |

### 4.4 竞态分析

- **history 清空 vs 新消息**：history 在同步阶段清空，/clear 返回后主线程立即可见空 history；后续新 turn 不会读到旧上下文。OK
- **保存读取 history**：保存用的是同步阶段 clone 的 old_history（完整快照），与清空无竞态。OK
- **session_id 切换时机**：session_id 在 SessionSwitched 事件处理时才切换（create 成功后），保存用的是 spawn 前 clone 的 old_id，两者不冲突。OK
- **exit_saved 标志**：切换到新会话后重置 session_exit_saved = false，避免退出时跳过新会话的 flush。OK

## 5. 受影响文件

| 文件 | 改动 |
|------|------|
| src/tui/app/input.rs | 重写 /clear 分支（L41-86）：同步 clone+清空，spawn 保存+创建+切换 |
| src/tui/app/types.rs | AppEvent 枚举新增 SessionSwitched { id, name } |
| src/tui/app/event.rs | 新增 SessionSwitched 处理：设 session_id/session_name、reset session_exit_saved、reset_agent_generation(新 id) |
| src/tui/util.rs | 穷尽 match 加入 SessionSwitched { .. } |

## 6. 不改动

- save_session_snapshot / spawn_save_session / flush_session_on_exit 等退出与 turn 完成路径不变。
- /session 面板、create_session/save_session/load_session 客户端方法签名不变。
- daemon 侧会话 API 不变。

## 7. 验证

- cargo fmt --check / cargo clippy --all-targets -- -D warnings / cargo test
- 手动：有对话时 /clear -> 旧会话出现在 /session 列表、当前为空白新会话；空会话 /clear -> 提示且不新建。
