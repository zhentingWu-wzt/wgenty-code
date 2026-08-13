# 实现计划：/clear 保存当前会话并开启新会话

- 日期：2026-08-06
- 设计文档：docs/superpowers/specs/2026-08-06-clear-save-and-new-session-design.md
- 执行方式：inline（当前会话顺序执行）
- 范围：4 个文件的局部改动

## 目标

/clear 从"清空当前会话内容、保持同一 session_id"改为"保存当前会话 + 创建并切换到新会话"。

## 实现调整（相对设计文档）

`submit_input` 是同步函数，`conversation_history` 是 `tokio::sync::Mutex`（需 `.lock().await`），因此 `old_history` 的快照+清空不能在同步阶段做，移入 spawn 块内（同一把锁内先 clone 再 clear，保证 save 拿到完整快照）。`old_ui_messages` 仍是同步 clone（`committed_messages.iter().map` 不需 await）。语义与设计一致。

## Task 1: 新增 AppEvent::SessionSwitched + 更新 phase 分类

文件：src/tui/app/types.rs、src/tui/util.rs

### 1a. types.rs 枚举新增（在 AgentGenerationReset 之前插入）

    /// `/clear` created a new session; the main loop adopts the new id/name.
    /// Subagent generation is reset separately via the follow-up
    /// [`AppEvent::AgentGenerationReset`].
    SessionSwitched {
        id: String,
        name: String,
    },

### 1b. util.rs agent_phase_from_event 的 None 分支加入 SessionSwitched

在 425-460 的"不改变 phase"分组 match 中，于 `| AppEvent::AgentGenerationReset { .. }` 行附近加入：

    | AppEvent::SessionSwitched { .. }

### 1c. 更新 test_non_phase_events_return_none（util.rs:567）

在测试末尾追加：

    // /clear 切换会话不改变 agent phase（仍为 Idle）
    assert_eq!(
        agent_phase_from_event(&AppEvent::SessionSwitched {
            id: "s1".into(),
            name: "New Session".into()
        }),
        None
    );

### 验证

    cargo test --lib tui::util::tests::test_non_phase_events_return_none
    cargo check

## Task 2: 重写 input.rs 的 /clear 分支

文件：src/tui/app/input.rs（替换 L41-86 的 `/clear` 分支）

替换为：

    if text.trim() == "/clear" {
        // Empty session: nothing to save, and creating another empty session
        // would just clutter the session list.
        if self.committed_messages.is_empty() {
            self.push_system_message("会话为空，无需清除");
            return;
        }

        // Snapshot UI transcript synchronously (no await needed) before
        // clearing the display. History is snapshotted inside the spawn
        // below because the tokio Mutex cannot be locked from this sync ctx.
        let old_id = self.session_id.clone();
        let old_name = self.session_name.clone();
        let old_ui_messages: Vec<_> = self
            .committed_messages
            .iter()
            .map(UIMessage::to_session_ui_message)
            .collect();

        // Clear visible state immediately so /clear returns a clean slate.
        self.committed_messages.clear();
        self.streaming_content.clear();
        self.streaming_active = false;
        self.scroll_offset = 0;
        self.user_scrolled = false;
        self.sandbox_bypassed_session = false;
        self.cancel_current_turn();
        self.phase = AgentPhase::Idle;
        self.suppress_phase_updates = true;
        self.pending_inputs.clear();

        // Async: snapshot+clear history, save old session, create new
        // session, then switch. The save uses the pre-clear snapshot so it
        // captures the full transcript under the old session id.
        let client = self.daemon_client.clone();
        let history = self.conversation_history.clone();
        let event_tx = self.event_tx.clone();
        tokio::spawn(async move {
            let old_history = {
                let mut h = history.lock().await;
                crate::api::types::sanitize_tool_call_pairing(&mut h);
                let snapshot = h.clone();
                h.clear();
                snapshot
            };

            // 1. Save the old session (best-effort).
            if let Err(error) = client
                .save_session(&old_id, &old_name, &old_history, &old_ui_messages)
                .await
            {
                tracing::warn!(
                    session_id = %old_id,
                    error = %error,
                    "failed to save session before /clear switch"
                );
                let _ = event_tx.send(AppEvent::SystemNotice(
                    "⚠️ 上一会话保存失败，已尝试切换到新会话".to_string(),
                ));
            }

            // 2. Create a new session and switch.
            match client.create_session(None).await {
                Ok(resp) => {
                    let _ = event_tx.send(AppEvent::SessionSwitched {
                        id: resp.id,
                        name: resp.name,
                    });
                }
                Err(error) => {
                    tracing::warn!(error = %error, "create_session failed after /clear");
                    let _ = event_tx.send(AppEvent::SystemNotice(format!(
                        "⚠️ 创建新会话失败：{error}（当前会话已清空，可继续使用）"
                    )));
                    // Fallback: reset generation under the old id so subagent
                    // state and suppress_phase_updates are cleaned up.
                    match client.reset_agent_generation(&old_id).await {
                        Ok(generation) => {
                            let _ = event_tx
                                .send(AppEvent::AgentGenerationReset { generation });
                        }
                        Err(error) => {
                            tracing::warn!(
                                error = %error,
                                "reset_agent_generation failed; retaining old generation"
                            );
                            let _ = event_tx.send(AppEvent::AgentGenerationReset {
                                generation: u64::MAX,
                            });
                        }
                    }
                }
            }
        });
        return;
    }

### 验证

    cargo check
    cargo clippy --all-targets -- -D warnings

## Task 3: 新增 event.rs 的 SessionSwitched 处理

文件：src/tui/app/event.rs（在 AgentGenerationReset 分支之前插入）

    AppEvent::SessionSwitched { id, name } => {
        // Adopt the newly created session. Subagent state cleanup and
        // suppress_phase_updates are handled by the AgentGenerationReset
        // event spawned below, mirroring the original /clear path.
        self.session_id = id.clone();
        self.session_name = name;
        self.session_exit_saved
            .store(false, std::sync::atomic::Ordering::Release);
        let client = self.daemon_client.clone();
        let event_tx = self.event_tx.clone();
        tokio::spawn(async move {
            match client.reset_agent_generation(&id).await {
                Ok(generation) => {
                    let _ = event_tx.send(AppEvent::AgentGenerationReset { generation });
                }
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        "reset_agent_generation failed; retaining old generation"
                    );
                    let _ = event_tx.send(AppEvent::AgentGenerationReset {
                        generation: u64::MAX,
                    });
                }
            }
        });
    }

### 验证

    cargo check
    cargo clippy --all-targets -- -D warnings

## Task 4: 全量构建验证

    cargo fmt
    cargo fmt --check
    cargo clippy --all-targets -- -D warnings
    cargo test --all

### 手动验证（需启动 daemon）

- 有对话时 /clear -> 旧会话出现在 /session 列表、当前为空白新会话、子代理视图清空
- 空会话 /clear -> 提示"会话为空，无需清除"且不新建
- /clear 后立即发新消息 -> 新消息进入新会话、不带旧上下文

## 自审

- 每个任务有明确验证步骤（测试 / 编译 / clippy）。✓
- 代码引用的字段/方法均已确认存在：session_id/session_name/session_exit_saved(Arc<AtomicBool>)/agent_generation/daemon_client/event_tx/conversation_history/committed_messages/cancel_current_turn/UIMessage::to_session_ui_message/sanitize_tool_call_pairing/create_session(Option<&str>)->SessionResponse/save_session/reset_agent_generation。✓
- 竞态：history 快照+清空在同一把 tokio Mutex 锁内（spawn 中），save 用 clone 快照；UI 清空在同步阶段立即生效。新 turn 读 history 在 spawn 清空之后为空。✓
- suppress_phase_updates 保证：成功路径 SessionSwitched -> spawn reset -> AgentGenerationReset 设 false；失败路径直接 await reset -> AgentGenerationReset 设 false。两条路径都 reset。✓
- 空会话短路避免空会话堆积。✓
- session_exit_saved 重置避免退出时跳过新会话 flush。✓
- 不改动 save_session_snapshot/spawn_save_session/flush_session_on_exit/daemon API。✓
