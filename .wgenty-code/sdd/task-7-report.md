# Task 7 Report: agent loop 集成(turn 边界挂 coordinator)

## 实现摘要

在共享 `run_agent_loop` 的 turn 边界挂接 `SessionCoordinator`:turn 开始调
`begin_turn`,turn 结束(所有 return path)调 `end_turn`。注册
`verify_and_complete` 工具到 `ToolRegistry`。新增 `agent.exec_session.enabled`
配置(prepared,frontend wiring 在 follow-up)。

## 改动文件(9 个源文件 + 1 brief)

| 文件 | 改动 |
|------|------|
| `src/exec_session/coordinator.rs` | 新增 `SessionCoordinatorPort` trait(`begin_turn`/`end_turn`)+ `Arc<RwLock<SessionCoordinator>>` impl |
| `src/exec_session/mod.rs` | re-export `SessionCoordinatorPort` + `UnverifiedOutcome` |
| `src/agent/runtime/loop_.rs` | `LoopHooks.session` 字段;`run_agent_loop` wrapper(begin/end around `run_agent_loop_inner`) |
| `src/tools/mod.rs` | `ToolRegistry::register_exec_session_tools(coordinator)` helper |
| `src/config/agent.rs` | `ExecSessionSettings { enabled: bool }(默认 true)` + `AgentConfig.exec_session` |
| `src/cli/headless_runtime.rs` | `LoopHooks.session: None`(降级,待 wiring) |
| `src/teams/subagent_loop.rs` | `LoopHooks.session: None`(降级,待 wiring) |
| `src/tui/agent/core.rs` | `LoopHooks.session: None`(降级,待 wiring) |
| `src/agent/runtime/loop_tests.rs` | 5 新测试 + `run_with_session`/`make_coordinator` helper |

## 设计要点

### SessionCoordinatorPort(最小化 trait)
- 仅 `begin_turn` + `end_turn`(同步,`std::sync::RwLock`,不跨 await)。
- **不放 finalize**:兜底 `mark_unverified_if_incomplete` 在 `VerifyGate`,frontend
  持有 gate,session close 时调。避免 `coordinator -> verify_gate` 依赖,避免
  trait 膨胀。多 turn 语义:兜底是 session-scoped,非 per-turn(否则 turn-1 后
  标 Unverified 会破坏 turn-2 的 begin_turn)。
- impl 在 `Arc<RwLock<SessionCoordinator>>` -- loop hook 与 VerifyGate 共享同一
  coordinator,turn 边界与 verify-gate 状态转换作用于同一 session。

### run_agent_loop wrapper(inner 函数模式)
- 原 body 重命名为 `run_agent_loop_inner`;新 `run_agent_loop` 提取
  `args.hooks.session`(`Option<&dyn>` is Copy,生命周期 'a 独立于局部 args),
  begin_turn -> inner -> end_turn,返回。
- **覆盖所有 return path**(Ok: 743/771/832;Err: 147/283/304/520/696/747/781/819)
  via 单一 wrapper,无需逐个 patch。
- begin/end 失败只 `tracing::warn!`,不 abort turn(session 是辅助层,不阻断 agent)。

### ToolRegistry helper
- `register_exec_session_tools(coordinator)`:`VerifyGate::new_with_default_hooks`
  + `ProcessCommandExecutor` + `VerifyAndCompleteTool`。frontend 创建 coordinator
  后调用。`with_project_root` 不注册(需 session coordinator)。

### 配置(prepared)
- `agent.exec_session.enabled`(默认 true)。**本 task 不 wire frontend**(3 个
  调用点传 `session: None`)。flag 为 prepared config;frontend wiring(headless
  构造 coordinator + 传 hook + 注册工具,需处理 headless 双 begin_turn:
  `checkpoint_manager` vs `checkpoint_store`;TUI 多 turn 生命周期)是紧接着的
  follow-up。

## 测试(5 新,全过)

| 测试 | 验证 |
|------|------|
| `exec_session_single_turn_records_turn_chain` (7.1) | 1 turn -> turns.len==1, current_turn==turn-0, parent==None |
| `exec_session_three_turns_parent_chain` (7.2) | 3 turn -> parent 链 turn-1←turn-0, turn-2←turn-1 |
| `exec_session_verify_tool_registered_and_callable` (7.3) | ToolRegistry 注册 + tool execute(`true`) -> session Completed |
| `exec_session_unverified_fallback_when_agent_skips_verify` (7.4) | loop 无 verify -> InProgress -> gate.mark_unverified -> Unverified |
| `exec_session_none_degrades_gracefully` (7.5) | session=None -> loop 正常,无 panic |

## 验证结果

- `cargo test --lib exec_session` -> **66 passed**(61 旧 + 5 新)
- `cargo test --lib checkpoint` -> **21 passed**(CheckpointStore 未受影响)
- `cargo test --lib undo` -> **3 passed**(undo 未受影响)
- `cargo test --all` -> **1049 passed, 2 failed**(2 失败在 `services::auto_dream`,
  环境问题:global memory dir 锁竞争;`auto_dream.rs` 不在本 task diff,pre-existing)
- `cargo clippy --all-targets -- -D warnings` -> **零 warning**
- `cargo fmt --check` -> **clean**
- 解耦不变式:`grep -rn comet src/exec_session/` -> 仅 2 处(serde JSON 举例 +
  doc 注释),符合 spec §2.4

## 不变式

- ✅ turn 边界自动挂 coordinator(begin/end via wrapper)
- ✅ verify_and_complete 可注册可调用(7.3)
- ✅ coordinator 关闭时优雅降级(session=None,7.5)
- ✅ 现有 CheckpointStore/undo 不受影响(21+3 passed)
- ✅ exec_session 无 comet 依赖(除 doc 举例)

## 范围外(follow-up)

- headless/TUI/subagent 实际构造 coordinator + 传 hook + 注册工具(frontend
  wiring)。需处理:headless 双 begin_turn(`checkpoint_manager.begin_turn` vs
  `coordinator.begin_turn` -> `checkpoint_store.begin_turn`)、TUI 多 turn 生命周期、
  session close 调 `gate.mark_unverified_if_incomplete`。可作 Task 8 e2e 前置或
  独立 7b。
- E2E 测试(Task 8)。
