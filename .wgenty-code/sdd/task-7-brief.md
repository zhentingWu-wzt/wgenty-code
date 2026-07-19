# Task 7 Brief: agent loop 集成(turn 边界挂 coordinator)

## Goal
agent loop 在 turn 边界创建/管理 SessionCoordinator:turn 开始调 begin_turn,turn 结束调 end_turn。注册 verify_and_complete 工具到 ToolRegistry。

## Design(基于已读源码)

### 共享模型(已确认)
- `VerifyGate` 持有 `coordinator: Arc<std::sync::RwLock<SessionCoordinator>>`(verify_gate.rs:171)
- `begin_turn(&mut self)` / `end_turn(&mut self)`(coordinator.rs:70, 98)
- `mark_unverified_if_incomplete(&self)` 在 `VerifyGate`(verify_gate.rs:376),仅用 coordinator 方法 + `set_verify_log_final_status` 自由函数

### 新增 trait(最小化)
`SessionCoordinatorPort` —— loop hook 只需 turn 边界两个方法:
```rust
pub trait SessionCoordinatorPort: Send + Sync {
    fn begin_turn(&self) -> Result<(), String>;
    fn end_turn(&self) -> Result<(), String>;
}
impl SessionCoordinatorPort for Arc<RwLock<SessionCoordinator>> { ... }
```
- **不放 finalize_session**:兜底(`mark_unverified_if_incomplete`)在 `VerifyGate`,frontend 持有 gate,session close 时直接调。避免 coordinator→verify_gate 依赖,避免 port 膨胀。
- 同步方法(std::sync::RwLock,不跨 await,无需 async_trait)。

### LoopHooks 新字段
`src/agent/runtime/loop_.rs`:
```rust
pub struct LoopHooks<'a> {
    // ... existing ...
    pub session: Option<&'a dyn SessionCoordinatorPort>,
}
```
`#[derive(Default)]` 仍生效(Option 默认 None)。

### run_agent_loop wrapper(inner 函数模式覆盖所有 return path)
```rust
pub async fn run_agent_loop(args: RunLoopArgs<'_>) -> Result<String, RuntimeError> {
    let session = args.hooks.session;           // Copy 出 Option<&dyn>
    if let Some(s) = session {
        if let Err(e) = s.begin_turn() { tracing::warn!(...); }
    }
    let result = run_agent_loop_inner(args).await;
    if let Some(s) = session {
        if let Err(e) = s.end_turn() { tracing::warn!(...); }
    }
    result
}
async fn run_agent_loop_inner(args: RunLoopArgs<'_>) -> Result<String, RuntimeError> {
    // 原 body(destructure + loop)
}
```
- begin/end 失败只 warn,不 abort turn(session 是辅助层,不应阻断 agent 工作)。
- 覆盖所有 Ok/Err 路径(743, 771, 832, 147, 283, 304, 520, 696, 747, 781, 819)。

### ToolRegistry helper
`src/tools/mod.rs`:
```rust
pub fn register_exec_session_tools(&self, coordinator: Arc<RwLock<SessionCoordinator>>) {
    let executor = Arc::new(ProcessCommandExecutor);
    let gate = Arc::new(VerifyGate::new_with_default_hooks(coordinator, executor));
    self.register(Box::new(VerifyAndCompleteTool::new(gate)));
}
```
- frontend 创建 coordinator 后调用;`with_project_root` 不注册(需 session coordinator)。

### Settings(配置开关,prepared)
`src/config/agent.rs` AgentConfig 加:
```rust
#[serde(default)]
pub exec_session: ExecSessionSettings,
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExecSessionSettings {
    #[serde(default = "default_exec_session_enabled")]
    pub enabled: bool,  // 默认 true
}
```
- **本 task 不 wire frontend**(headless/TUI/subagent 仍传 None)。flag 为 prepared config,frontend wiring 是紧接着的 follow-up(可作 Task 8 e2e 前置或独立 7b)。文档明示。

### 不在本 task 范围
- headless/TUI/subagent 实际构造 coordinator + 传 hook + 注册工具(需处理 headless 双 begin_turn:checkpoint_manager vs checkpoint_store;TUI 多 turn 生命周期)。这些是 frontend wiring,独立处理。
- 崩溃一致性 E2E(Task 8)。

## 测试(7.1-7.5,在 loop_tests.rs)
- **7.1**:real coordinator(temp dir)+ ScriptedLlm(text x1)。run_agent_loop -> session.turns.len()==1, current_turn=="turn-0"。新增 `run_with_session()` helper。
- **7.2**:同一 coordinator 跑 3 次 run_agent_loop -> turns.len()==3, parent 链 turn-1.parent=turn-0, turn-2.parent=turn-1。
- **7.3**:ToolRegistry::with_project_root(temp) + register_exec_session_tools(coordinator) -> definitions() 含 "verify_and_complete";tool execute 返回结构化 result。
- **7.4**:real coordinator + gate,run_agent_loop(无 verify) -> gate.mark_unverified_if_incomplete() -> session.status==Unverified。
- **7.5**:LoopHooks.session=None,run_agent_loop 正常,无 panic(用现有 `run()` helper)。

## 文件改动
- `src/exec_session/coordinator.rs`:加 `SessionCoordinatorPort` trait + impl
- `src/exec_session/mod.rs`:re-export `SessionCoordinatorPort`
- `src/agent/runtime/loop_.rs`:LoopHooks.session + run_agent_loop wrapper + inner
- `src/agent/runtime/mod.rs`:可能 re-export `SessionCoordinatorPort`(或 LoopHooks 直接用 exec_session path)
- `src/tools/mod.rs`:register_exec_session_tools
- `src/config/agent.rs`:ExecSessionSettings + AgentConfig.exec_session
- `src/agent/runtime/loop_tests.rs`:5 个新测试 + run_with_session helper

## 验证
- `cargo test agent_loop` / `cargo test exec_session`
- `cargo clippy --all-targets -- -D warnings`
- 解耦不变式:`grep comet src/exec_session/` 仅 doc 举例
- 现有 loop_tests 不受影响(session=None 默认)
