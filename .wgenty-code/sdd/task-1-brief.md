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
