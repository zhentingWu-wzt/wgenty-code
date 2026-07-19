# ExecutionSession 内层(SessionCoordinator + verify-gate)实现计划 v2

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> **v2 修订(回溯简化):** 去掉 FileBlobStore / SnapshotStore / SideEffects / declared_side_effects。复用现有 CheckpointStore(file capture/rewind 不动),新做 SessionCoordinator(session 串联 + git refs + untracked)+ verify_and_complete。Task 从 12 减到 8。

**Goal:** 实现 ExecutionSession 内层--SessionCoordinator(turn 串联 + git refs + untracked)+ verify_and_complete(显式 gate,防编造 + 越界检测)--让长程任务可中断 / 可回滚 / 可验证,与 comet 解耦。复用现有 CheckpointStore,不新做 SnapshotStore / FileBlobStore。

**Architecture:** C 方案分层。内层 = 纯执行原语:SessionCoordinator(复用 CheckpointStore 的 file capture/rewind,新加 session 串联 + git refs + untracked)+ verify_and_complete(显式 gate)。外层(node 状态机 / 跨会话 / comet-adapter)不在本计划。

**Tech Stack:** Rust 2021 (MSRV 1.75+), tokio, serde/serde_json, thiserror/anyhow, tempfile(原子写)。

**Global Constraints:**
- `cargo fmt --check` + `cargo clippy --all-targets -- -D warnings` 零 warning(CI 强制)
- 库代码 thiserror,应用层 anyhow + `.context()`,不裸 `unwrap()`/`?`
- 模块依赖:`tools/` 不依赖 `agent/`;新 `exec_session/` 不依赖 `agent/`,`agent/` 可依赖 `exec_session/`;`exec_session` 依赖 `tools::checkpoint_store`(tools 是底层,允许)
- 三平台兼容(linux/macos/windows);git 操作走命令行 `git`(不引 git2)
- 解耦不变式:`exec_session/` 代码无 "comet" 字符串(除注释/文档举例)
- 现有 `CheckpointStore` 和 undo 工具行为**完全不变**(CheckpointStore 不动)

**Spec:** `docs/superpowers/specs/2026-07-19-exec-session-inner-layer-design.md` v2

## File Structure

**Create:**
- `src/exec_session/mod.rs` - exec_session 模块入口
- `src/exec_session/session.rs` - SessionState(session.json 读写 + 原子写)
- `src/exec_session/coordinator.rs` - SessionCoordinator(begin_turn/end_turn/rollback_to + git refs + untracked)
- `src/exec_session/hooks.rs` - SessionHooks trait + DefaultHooks + HookDecision
- `src/exec_session/verify_gate.rs` - verify_and_complete 工具(防编造 + 越界 + verify_log)
- `tests/exec_session_e2e.rs` - 端到端测试

**Modify:**
- `src/lib.rs` - 注册 `exec_session` 模块
- `src/agent/core.rs`(或调度点)- agent loop 在 turn 边界创建/管理 SessionCoordinator
- `src/tools/registry.rs`(或 ToolRegistry 构造点)- 注册 verify_and_complete 工具

**不改动:**
- `src/tools/checkpoint_store.rs`(完全不动,复用)
- `src/tools/mod.rs` Tool trait(不加 declared_side_effects)
- 现有 filesystem/execution 工具(不加声明)

## Task 1: SessionState + SessionCoordinator 基础(session.json + begin_turn/end_turn)

**Goal:** 实现 SessionState(session.json 原子读写)+ SessionCoordinator 骨架(begin_turn/end_turn 更新 turns 链 + current_turn),暂不记 git refs/untracked(Task 3 加)。

**Files:** Create `src/exec_session/mod.rs`, `src/exec_session/session.rs`, `src/exec_session/coordinator.rs`;Modify `src/lib.rs`

**Interfaces:**
```rust
// src/exec_session/session.rs
pub struct SessionState {
    pub session_id: String,
    pub source: SessionSource,
    pub status: SessionStatus,
    pub created_at: String,
    pub updated_at: String,
    pub turns: Vec<TurnRecord>,
    pub current_turn: Option<String>,
}
pub enum SessionSource { Comet, AgentSelf, UserDirect }
pub enum SessionStatus { InProgress, Completed, Unverified, Failed }
pub struct TurnRecord {
    pub turn_id: String,
    pub parent: Option<String>,
    pub checkpoint_turn_id: String,  // 关联 CheckpointStore
    pub git_refs: Option<GitRefs>,    // Task 3 填
    pub untracked_files: Vec<String>, // Task 3 填
    pub created_at: String,
}
pub struct GitRefs { pub head: String }
impl SessionState {
    pub fn new(session_id: String, source: SessionSource) -> Self;
    pub fn load(dir: &Path) -> Result<Self>;
    pub fn save(&self, dir: &Path) -> Result<()>;  // tmp+rename 原子
    pub fn set_status(&mut self, status: SessionStatus);
    pub fn current_turn_record(&self) -> Option<&TurnRecord>;
}

// src/exec_session/coordinator.rs
pub struct SessionCoordinator {
    session: SessionState,
    session_dir: PathBuf,  // <project>/.wgenty-code/snapshots/<session_id>/
    checkpoint_store: Arc<CheckpointStore>,  // 复用,不新建
}
impl SessionCoordinator {
    pub fn new(session_id: String, source: SessionSource, project_root: &Path, checkpoint_store: Arc<CheckpointStore>) -> Result<Self>;
    pub fn begin_turn(&mut self) -> Result<&TurnRecord>;
    pub fn end_turn(&mut self) -> Result<()>;
    pub fn session(&self) -> &SessionState;
}
```

**Steps:**
- [ ] 1.1 写测试 `src/exec_session/session.rs`:SessionState::new 初始化 InProgress + 空 turns + current_turn=None;save 写 session.json(tmp+rename);load 读回一致;load 不存在返回错误
- [ ] 1.2 写测试:save 原子性(写 tmp 时模拟中断,session.json 保持旧版本或不存在,无 .tmp 残留)
- [ ] 1.3 写测试:SessionSource / SessionStatus / TurnRecord / GitRefs serde 序列化往返
- [ ] 1.4 写测试 `src/exec_session/coordinator.rs`:`new` 创建 session_dir + session.json(InProgress);`begin_turn` 开新 turn(parent 指向上一 turn 或 None,checkpoint_turn_id 关联 CheckpointStore 当前 turn_id),turns 长度 +1,current_turn 指向新 turn
- [ ] 1.5 写测试:`end_turn` 封存 turn + 更新 current_turn + save;连续 begin/end 形成 turn 链(parent 正确)
- [ ] 1.6 实现 session.rs + coordinator.rs。CheckpointStore 通过 Arc 共享(复用现有实例)。跑 `cargo test exec_session`
- [ ] 1.7 `src/lib.rs` 加 `pub mod exec_session;`。跑 `cargo clippy --all-targets -- -D warnings`

**验证不变式:** CheckpointStore 代码无改动;现有 `cargo test checkpoint` 全过。

## Task 2: SessionHooks trait + DefaultHooks

**Goal:** 定义 hook 机制,DefaultHooks 提供非 comet 模式默认策略。预留 pre_node/post_node 给外层。

**Files:** Create `src/exec_session/hooks.rs`;Modify `src/exec_session/mod.rs`

**Interfaces:**
```rust
// src/exec_session/hooks.rs
pub trait SessionHooks: Send + Sync {
    fn verify_fail(&self, _ctx: &VerifyFailCtx) -> HookDecision { HookDecision::AutoRetry { max: 2 } }
    fn side_effect_out_of_scope(&self, _ctx: &SideEffectCtx) -> HookDecision { HookDecision::WarnAndContinue }
    fn rollback_triggered(&self, _ctx: &RollbackCtx) -> HookDecision { HookDecision::LogOnly }
    fn pre_node(&self, _ctx: &NodeCtx) -> HookDecision { HookDecision::Continue }
    fn post_node(&self, _ctx: &NodeCtx) -> HookDecision { HookDecision::Continue }
}
pub struct DefaultHooks;
impl SessionHooks for DefaultHooks {}
pub enum HookDecision { Continue, Block, AutoRetry { max: u8 }, WarnAndContinue, LogOnly }
pub struct VerifyFailCtx { pub reason: VerifyFailReason, pub attempt: u8 }
pub enum VerifyFailReason { CommandFailed { cmd: String, exit_code: i32 }, OutOfScope { files: Vec<PathBuf> } }
pub struct SideEffectCtx { pub declared: Vec<PathBuf>, pub actual: Vec<PathBuf> }
pub struct RollbackCtx { pub from: String, pub to: String }
pub struct NodeCtx { pub node_id: String }
```

**Steps:**
- [ ] 2.1 写测试:`DefaultHooks::verify_fail` 返回 `AutoRetry { max: 2 }`;`side_effect_out_of_scope` 返回 `WarnAndContinue`;`rollback_triggered` 返回 `LogOnly`;`pre_node`/`post_node` 返回 `Continue`
- [ ] 2.2 写测试:HookDecision 各变体;VerifyFailReason 序列化(CommandFailed / OutOfScope)
- [ ] 2.3 实现 hooks.rs。`exec_session/mod.rs` 导出。跑 `cargo test hooks`
- [ ] 2.4 跑 `cargo clippy --all-targets -- -D warnings`

## Task 3: turn 边界 git refs + untracked 记录

**Goal:** begin_turn 记 git_refs.head(`git rev-parse HEAD`)+ untracked_files(`git ls-files --others --exclude-standard`);非 git 项目降级(null/空)。

**Files:** Modify `src/exec_session/coordinator.rs`

**Interfaces:**
```rust
impl SessionCoordinator {
    // 内部辅助
    fn capture_git_refs(&self) -> Result<Option<GitRefs>>;  // git rev-parse HEAD;非 git 返回 Ok(None)
    fn capture_untracked(&self) -> Result<Vec<String>>;      // git ls-files --others --exclude-standard;非 git 返回 Ok(vec![])
}
// begin_turn 调用上述填入 TurnRecord.git_refs / untracked_files
```

**Steps:**
- [ ] 3.1 写测试:git 项目 fixture(临时 `git init` + commit),`capture_git_refs` 返回 `Some(GitRefs{head})`;`capture_untracked` 返回当前 untracked 列表
- [ ] 3.2 写测试:非 git 项目(临时空目录,无 .git),`capture_git_refs` 返回 `Ok(None)`;`capture_untracked` 返回 `Ok(vec![])`,不报错
- [ ] 3.3 写测试:`begin_turn` 后 TurnRecord.git_refs / untracked_files 正确填入(git 项目);非 git 项目填 None / 空
- [ ] 3.4 写测试:git 命令执行失败(如 git 不存在或权限问题)降级返回 None/空,不阻断 turn(符合 spec 3.4 快照失败策略)
- [ ] 3.5 实现 capture_git_refs / capture_untracked(走命令行 `git`,通过 tokio::process::Command 或现有命令执行通道;非 git 降级)。begin_turn 调用。跑 `cargo test exec_session`
- [ ] 3.6 跑 `cargo clippy --all-targets -- -D warnings`

**验证不变式:** 非 git 项目 begin_turn 不失败(降级);git 项目 head/untracked 正确。

## Task 4: 回退算法(git reset + CheckpointStore::rewind + 删 untracked)

**Goal:** SessionCoordinator::rollback_to(turn_id) 实现 spec 3.4 回退算法:`git reset --hard`(若 head 变)+ CheckpointStore::rewind + 删新增 untracked。

**Files:** Modify `src/exec_session/coordinator.rs`

**Interfaces:**
```rust
impl SessionCoordinator {
    pub fn rollback_to(&mut self, turn_id: &str, hooks: &dyn SessionHooks) -> Result<RollbackResult>;
}
pub struct RollbackResult {
    pub git_reset: bool,                  // 是否执行了 git reset --hard
    pub restored_files: Vec<PathBuf>,     // CheckpointStore::rewind 恢复的文件
    pub deleted_untracked: Vec<PathBuf>,  // 删除的新增 untracked
}
```

**Steps:**
- [ ] 4.1 写测试:git 项目,turn N 有 commit(head 变),rollback_to(turn_N) 执行 `git reset --hard <turn_N.head>`,验证 git log 回到 turn_N.head
- [ ] 4.2 写测试:git 项目,turn N 无 commit(head 未变),rollback_to 跳过 git reset(git_reset=false),只 rewind + 删 untracked
- [ ] 4.3 写测试:CheckpointStore::rewind 被调用,传 turn_N.checkpoint_turn_id;file_edit pre-edit 内容恢复;Tombstone(新建文件)被删除(验证现有 CheckpointStore::rewind 行为,本 Task 只确认调用正确)
- [ ] 4.4 写测试:删 untracked--turn N 期间新建的 untracked(当前 untracked - turn_N.untracked_files)被删除;turn N 之前就存在的 untracked 不删(对比列表,**不**用 `git clean -fd`)
- [ ] 4.5 写测试:非 git 项目,rollback_to 跳过 git reset + 跳过删 untracked(git_refs/untracked 为空),只 CheckpointStore::rewind(file 回退仍工作)
- [ ] 4.6 写测试:rollback_triggered hook 被调用(ctx.from = current_turn, ctx.to = turn_id),返回 LogOnly(默认)
- [ ] 4.7 写测试:回退后 current_turn 更新为 turn_N,session.json save
- [ ] 4.8 实现 rollback_to。顺序:`git reset --hard`(if head 变)-> CheckpointStore::rewind(turn_N.checkpoint_turn_id)-> 删新增 untracked(当前 untracked - turn_N.untracked_files)-> 更新 current_turn + save。跑 `cargo test rollback`
- [ ] 4.9 跑 `cargo clippy --all-targets -- -D warnings`

**验证不变式:** 回退后工作区 = turn_N 开始前状态(tracked + file_edit + untracked 三源全覆盖);非 git 降级仍工作;不误删 turn N 之前的 untracked。

## Task 5: verify_and_complete 工具(防编造 + 越界检测 + verify_log)

**Goal:** 实现 verify_and_complete 工具:runtime 亲自跑 commands(经 guardian/sandbox)+ 越界检测(actual ⊆ expected)+ 写 verify_log。**不接收** agent 贴的声称结果。

**Files:** Create `src/exec_session/verify_gate.rs`;Modify `src/exec_session/mod.rs`, `src/tools/registry.rs`(或 ToolRegistry 构造点)

**Interfaces:**
```rust
// src/exec_session/verify_gate.rs
pub struct VerifyGate {
    coordinator: Arc<RwLock<SessionCoordinator>>,
    // 依赖 guardian + sandbox(注入,和 exec_command 同等对待)
}
impl VerifyGate {
    // 亲自跑 commands,返回真实结果 + 越界检测
    pub async fn verify_and_complete(
        &self,
        commands: Vec<String>,
        expected_changed_files: Vec<PathBuf>,
    ) -> Result<VerifyResult>;
}
pub struct VerifyResult {
    pub success: bool,
    pub commands_run: Vec<CommandRun>,       // cmd / exit_code / stdout / stderr
    pub actual_changed_files: Vec<PathBuf>,
    pub expected_changed_files: Vec<PathBuf>,
    pub out_of_scope: Vec<PathBuf>,          // actual - expected
    pub fail_reason: Option<VerifyFailReason>,
}
pub struct CommandRun { pub cmd: String, pub exit_code: i32, pub stdout: String, pub stderr: String }
// verify_log.json: { attempts: [...], final_status }

// 注册为 Tool(实现 Tool trait)
// input_schema: { commands: [string], expected_changed_files: [string] }
// is_read_only: false(改 session.status)
```

**Steps:**
- [ ] 5.1 写测试:commands 全 exit 0 + actual ⊆ expected -> VerifyResult.success=true,session.status=Completed,verify_log 记录 attempt + final_status=completed
- [ ] 5.2 写测试:某 command exit 非 0 -> success=false,fail_reason=CommandFailed,session.status 保持 InProgress(不自动改 failed,由 hook 决定)
- [ ] 5.3 写测试:actual 有文件不在 expected(越界)-> success=false,fail_reason=OutOfScope,out_of_scope 列出越界文件
- [ ] 5.4 写测试:actual 计算 = CheckpointStore manifest 文件路径并集(session 范围)+ `git diff`(tracked 改动,若 git 项目)+ untracked 新增(对比 turn 链);三源全覆盖(用 fixture 验证)
- [ ] 5.5 写测试:commands 经 guardian 审查(注入 mock guardian,验证 review 被调用)+ sandbox 执行(验证沙箱标记);runtime 不裸跑
- [ ] 5.6 写测试:verify_log.json 写入 `<session_id>/verify_log.json`,每次 attempt 追加;final_status 字段
- [ ] 5.7 写测试:工具**不接收** agent 贴的声称结果字段(input_schema 只有 commands + expected_changed_files,无 result/status 字段)
- [ ] 5.8 实现 verify_gate.rs。commands 执行走现有 exec 通道(经 guardian + sandbox)。actual 计算三源。注册为 Tool。跑 `cargo test verify`
- [ ] 5.9 跑 `cargo clippy --all-targets -- -D warnings`

**验证不变式:** agent 无法贴假结果(机制杜绝);越界检测三源全覆盖;commands 经 guardian/sandbox。

## Task 6: verify_fail hook + unverified 兜底

**Goal:** verify 失败触发 verify_fail hook(AutoRetry,不回退);agent 完成但没调 verify_and_complete 时,agent loop 兜底标 unverified。

**Files:** Modify `src/exec_session/verify_gate.rs`, `src/agent/core.rs`

**Interfaces:**
```rust
// verify_gate.rs: verify 失败后调 hook
impl VerifyGate {
    // verify_and_complete 内部,失败时:
    //   attempt += 1
    //   hooks.verify_fail(VerifyFailCtx { reason, attempt }) -> HookDecision
    //   AutoRetry{max} 且 attempt <= max -> 返回失败给 agent(不回退,agent 续修)
    //   AutoRetry 且 attempt > max -> session.status = Failed
    //   Block -> session.status = Failed
    //   WarnAndContinue -> session.status = Completed(警告)
}

// agent/core.rs: 兜底
// 检测 session 结束信号(最终回复 / 用户结束 / 超时)
// session.status 仍 InProgress -> 标 Unverified + prompt 引导
```

**Steps:**
- [ ] 6.1 写测试:verify 失败(命令 exit 非 0),DefaultHooks.verify_fail 返回 AutoRetry{max:2},attempt=1 -> 返回失败给 agent,session.status 保持 InProgress(不回退)
- [ ] 6.2 写测试:连续失败 attempt=3(超 max:2)-> session.status = Failed,verify_log final_status=failed
- [ ] 6.3 写测试:自定义 hook 返回 Block -> session.status = Failed(立即)
- [ ] 6.4 写测试:自定义 hook 返回 WarnAndContinue -> session.status = Completed(警告标记)
- [ ] 6.5 写测试:agent loop 兜底--模拟 session 结束(最终回复)但 status 仍 InProgress -> 标 Unverified;verify_log final_status=unverified
- [ ] 6.6 实现 hook 调用 + agent loop 兜底检测。跑 `cargo test verify_fail`
- [ ] 6.7 跑 `cargo clippy --all-targets -- -D warnings`

**验证不变式:** 失败不回退(工作区保留);超 max 标 failed;agent 忘调 verify 兜底 unverified。

## Task 7: agent loop 集成(turn 边界挂 coordinator)

**Goal:** agent loop 在 turn 边界创建/管理 SessionCoordinator:turn 开始调 begin_turn,turn 结束调 end_turn。注册 verify_and_complete 工具到 ToolRegistry。

**Files:** Modify `src/agent/core.rs`(或调度点), `src/tools/registry.rs`

**Interfaces:**
```rust
// agent/core.rs: agent loop 持有 Option<SessionCoordinator>
// turn 开始:coordinator.begin_turn()(若 Some)
// turn 结束:coordinator.end_turn()(若 Some)
// session 创建时机:agent loop 启动时(或首次 turn),source = AgentSelf(DefaultHooks)
// verify_and_complete 工具通过 Arc<RwLock<SessionCoordinator>> 共享给 VerifyGate
```

**Steps:**
- [ ] 7.1 写测试:agent loop 启动,创建 SessionCoordinator(source=AgentSelf);首个 turn 开始调 begin_turn,turns 长度=1;turn 结束调 end_turn,current_turn 指向 turn-0
- [ ] 7.2 写测试:连续 3 个 turn,turn 链 parent 正确(turn-1 parent=turn-0,turn-2 parent=turn-1)
- [ ] 7.3 写测试:verify_and_complete 工具在 ToolRegistry 注册,agent 可调用;工具通过共享 coordinator 操作 session
- [ ] 7.4 写测试:agent loop 结束(最终回复),触发兜底检测(Task 6 的 unverified 逻辑)
- [ ] 7.5 写测试:SessionCoordinator 为 None 时(agent loop 不启用,如配置关闭),agent loop 正常工作,无 panic(优雅降级)
- [ ] 7.6 实现 agent loop 集成。配置开关(如 settings 加 `agent.exec_session.enabled`,默认 true)。跑 `cargo test agent_loop`
- [ ] 7.7 跑 `cargo clippy --all-targets -- -D warnings`

**验证不变式:** turn 边界自动挂 coordinator;verify_and_complete 可用;coordinator 关闭时优雅降级。

## Task 8: 端到端测试 + 解耦不变式验证

**Goal:** 端到端验证完整闭环:turn 串联 + git refs + 回退 + verify + 兜底;验证解耦不变式(exec_session 无 comet)+ CheckpointStore 不受影响。

**Files:** Create `tests/exec_session_e2e.rs`

**Steps:**
- [ ] 8.1 E2E 测试:git 项目,3 turn + file_edit + commit + verify_and_complete(全 pass)-> session.status=Completed;verify_log 记录
- [ ] 8.2 E2E 测试:verify 失败(命令 exit 非 0)-> AutoRetry;agent 续修后再次 verify pass -> Completed
- [ ] 8.3 E2E 测试:verify 越界(actual 有文件不在 expected)-> 失败;agent 调整 expected 后 pass
- [ ] 8.4 E2E 测试:rollback_to(turn-1)-> 工作区回到 turn-1 开始前(git reset + CheckpointStore::rewind + 删 untracked);current_turn=turn-1
- [ ] 8.5 E2E 测试:agent 忘调 verify_and_complete -> 兜底 unverified
- [ ] 8.6 E2E 测试:非 git 项目,turn + file_edit + verify(无 git refs/untracked)-> 降级工作,回退靠 CheckpointStore::rewind
- [ ] 8.7 E2E 测试:崩溃一致性--模拟 session.json 写中断(tmp 存在但 rename 未发生)-> resume 读旧版本或降级,不半个
- [ ] 8.8 解耦不变式测试:`grep -r "comet" src/exec_session/` 无结果(除注释/文档举例);或用编译期断言
- [ ] 8.9 不变式测试:现有 `cargo test checkpoint` + `cargo test undo` 全过(CheckpointStore 未受影响)
- [ ] 8.10 跑 `cargo test --all` + `cargo clippy --all-targets -- -D warnings` + `cargo fmt --check` 全绿

**验证不变式:** 完整闭环工作;解耦不变式成立;现有 CheckpointStore/undo 不受影响。

## Self-Review

**Spec 覆盖(对照 spec 节 7 验收标准 11 条):**
- [x] SessionCoordinator begin_turn/end_turn/rollback_to 复用 CheckpointStore -> Task 1, 4
- [x] session.json 含 turns 链 + git_refs.head + untracked_files + status,原子写 -> Task 1, 3
- [x] turn 边界记 git refs + untracked(非 git 降级)-> Task 3
- [x] verify_and_complete:亲自跑 commands + 越界检测 + guardian/sandbox -> Task 5
- [x] verify_fail hook(AutoRetry,不回退)+ 兜底 unverified -> Task 6
- [x] 回退算法:git reset --hard + CheckpointStore::rewind + 删 untracked -> Task 4
- [x] 崩溃一致性:session.json tmp+rename -> Task 1.2, 8.7
- [x] current_turn 游标(内层写外层读)-> Task 1
- [x] 快照失败策略:git 不可用降级 / session.json 写失败 fail fast -> Task 3.4
- [x] 解耦不变式:exec_session 无 comet -> Task 8.8
- [x] 现有 CheckpointStore/undo 测试不受影响 -> Task 8.9

**占位符检查:** 无 TBD/TODO;接口签名完整;步骤含实际命令(`cargo test <name>`, `cargo clippy`)

**类型一致性:** SessionState / TurnRecord / SessionHooks / VerifyResult 跨 task 接口一致;SessionCoordinator 持有 `Arc<CheckpointStore>`(复用,不新建)

**依赖序:** Task 1(基础)-> 3(git refs)-> 4(回退);Task 2(hooks)独立;Task 5(verify)依赖 1+3+4;Task 6 依赖 5;Task 7(agent loop)依赖 1-6;Task 8 依赖全部。可并行:1 与 2 互不依赖;3 依赖 1;4 依赖 1+3;5 依赖 1+3+4;6 依赖 5;7 依赖全部;8 最后

**执行方式建议:** subagent-driven-development,Task 1-2 可并行,Task 3-4 串行(依赖 1),Task 5-6 串行(依赖 4),Task 7 串行(改 agent loop),Task 8 最后。每个 task 独立 commit。
