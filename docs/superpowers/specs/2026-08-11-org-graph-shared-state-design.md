---
comet_change: org-graph-shared-state
role: technical-design
canonical_spec: openspec
---

# Design: Org-Graph Shared-State

本设计是对 open 阶段 `openspec/changes/org-graph-shared-state/design.md`(高层框架)的**深度技术细化**,不替代它。canonical spec 为该 change 的 OpenSpec delta spec。

## 1. 关键背景发现(brainstorming 查证)

CodeGraph 查证 `exec_session` 的 verify 闭环,得出两个重塑 pilot 的事实:

1. **verify→retry 闭环已存在且内部已结构化**。`verify_gate.rs:208 verify_and_complete` 真实执行 verify 命令(反编造),产出强类型 `VerifyResult { success, commands_run, fail_reason: Option<VerifyFailure>, ... }`,`VerifyFailure` 是枚举:`CommandFailed { command, exit_code, stderr }` / `BoundaryViolation { unexpected_files }`。失败驱动 retry:`VerifyFailAction::AutoRetry { remaining }` → session 保持 InProgress。

2. **结构化结果在出口被降级成文本**。`node_runtime.rs:204` 把强类型 `VerifyFailure` 用 `format!("{f:?}")` 退化成 `failure_reason: Option<String>` 回传给 `NodeVerifyResult`。结构化的 exit_code/stderr/unexpected_files 流失成自然语言字符串。

**结论**:proposal 核心论点(状态漂移)在代码里有活样本。pilot 的最优形态不是新建路由,而是**修复这个已存在的结构化降级点**——范围更小、零新路由、直击核心论点。

## 2. 模块布局

```
src/org_graph/
├── work_state.rs      # 新增:WorkState schema + FieldPerms + 读写 API + VerifyOutcome
├── contract.rs        # 修改:ContractDimension 加 State 变体
├── registry.rs        # 修改:NodeType 加 fn field_perms() -> FieldPerms
└── mod.rs             # 修改:导出 work_state
```

`org_graph` 保持纯数据 + 纯函数,零 `exec_session` 依赖。集成(转换、turn 锚定)在 `exec_session` 侧。

## 3. 核心类型(`src/org_graph/work_state.rs`)

```rust
/// 当前 turn 的结构化工作产物。pilot 只强制 verify_result;
/// 其余字段为 Option 占位,强制逻辑留后续 change(避免空壳 API)。
pub struct WorkState {
    pub requirement: Option<String>,            // 跨 turn 继承
    pub verify_result: Option<VerifyOutcome>,   // pilot 核心字段
    pub step_log: Vec<StepRecord>,              // 审计轨迹(授权写记入)
    // generated_diff / test_result / human_review / budget:
    // spec 列出但 pilot 不涉及,本期不声明读写强制逻辑。
}

/// org_graph 内聚的独立类型(exec_session 集成时从 VerifyResult 投影转换)。
/// 只保留 retry 决策需要的 success + fail_reason 枚举,不复制完整 VerifyResult。
pub struct VerifyOutcome {
    pub success: bool,
    pub fail_reason: Option<VerifyFailureKind>,
}
pub enum VerifyFailureKind {
    CommandFailed { exit_code: Option<i32>, stderr: String },
    BoundaryViolation { unexpected_files: Vec<String> },
}

/// 字段权限矩阵。
pub struct FieldPerms {
    pub readable: HashSet<WorkField>,
    pub writable: HashSet<WorkField>,
}
pub enum WorkField { Requirement, VerifyResult, StepLog }
```

`WorkState` / `VerifyOutcome` / `VerifyFailureKind` / `FieldPerms` / `WorkField` 全部派生 `Serialize/Deserialize/Clone/Debug`,支持 CheckpointStore 持久化与单测。

## 4. 字段权限矩阵(`NodeType::field_perms`)

```rust
impl NodeType {
    pub fn field_perms(&self) -> FieldPerms {
        match self {
            // verify 节点:执行 verify → 写 verify_result,读 requirement/verify_result/step_log
            NodeType::Verification => FieldPerms {
                readable: {Requirement, VerifyResult, StepLog},
                writable: {VerifyResult, StepLog},
            },
            // GeneralPurpose(可做 retry 决策的协调者):读 verify_result,写 step_log
            NodeType::GeneralPurpose => FieldPerms {
                readable: {Requirement, VerifyResult, StepLog},
                writable: {StepLog},
            },
            // explore/plan/guide:本期不涉及 verify_result,只读 requirement
            NodeType::Explore | NodeType::Plan | NodeType::WgentyCodeGuide => FieldPerms {
                readable: {Requirement},
                writable: {},
            },
        }
    }
}
```

## 5. 读写 API(查表强制)

```rust
impl WorkState {
    /// 写:查 caller 的 field_perms,越权 → ContractViolation{dimension: State}
    pub fn set_verify_result(
        &mut self, caller: NodeType, outcome: VerifyOutcome,
    ) -> Result<(), ContractViolation> {
        if !caller.field_perms().writable.contains(&WorkField::VerifyResult) {
            return Err(ContractViolation {
                node_type: caller,
                dimension: ContractDimension::State,  // 新增变体
                reason: "node type not permitted to write verify_result".into(),
            });
        }
        self.verify_result = Some(outcome);
        self.step_log.push(StepRecord::wrote(caller, WorkField::VerifyResult));
        Ok(())
    }
    /// 读:同理查 readable,越权读也报(读写对称)。
    pub fn verify_result(&self, caller: NodeType)
        -> Result<Option<&VerifyOutcome>, ContractViolation> { ... }
}
```

设计点:
- **每字段一具名方法**(`set_verify_result`/`verify_result`),非泛型 get/set。字段少且固定,具名让越权点显式、可 grep,对齐现有 `filter_allowed_tools` 风格。
- **写成功自动记 step_log**(谁在何时写了哪个字段);读不记(读高频,记会爆)。
- **读越权也报**,读写权限对称,便于审计。

## 6. pilot 集成:降级点修复

### 6.1 verify_node 出口(`exec_session/node_runtime.rs:204`)

现状:
```rust
let failure_reason = result.fail_reason.as_ref().map(|f| format!("{f:?}"));
Ok(NodeVerifyResult { failure_reason, ... })
```

修复后:
```rust
// 1. 结构化投影写入 WorkState(经权限强制)
let outcome = VerifyOutcome::from(&result);
work_state.set_verify_result(NodeType::Verification, outcome)?;

// 2. NodeVerifyResult.failure_reason 兼容期保留,但源头改为从 WorkState 读回
let failure_reason = work_state.verify_result(NodeType::Verification)?
    .and_then(|o| o.fail_reason.as_ref().map(|f| f.to_debug_string()));
Ok(NodeVerifyResult { failure_reason, ... })
```

**兼容期取舍**:`NodeVerifyResult.failure_reason: Option<String>` 本期保留(向后兼容现有调用点/测试),但源头从 `format!("{f:?}")` 改为「从 WorkState 强类型枚举读回再转 debug string」。现有读 failure_reason 的代码不破坏(零回归);retry 决策改为直接读 `work_state.verify_result()` 拿枚举分支。

### 6.2 turn 继承(`exec_session/coordinator.rs` 的 begin_turn 边界)

```rust
pub fn begin_turn(&mut self) -> Result<&TurnRecord> {
    // ...现有 turn 逻辑...
    self.work_state = self.work_state.inherit_for_new_turn();
}
```

`inherit_for_new_turn()`:`requirement` 克隆保留,`verify_result`/`step_log` 清空。同 turn 内 `verify_node` retry 不走 `begin_turn`(retry 是 node 重试,不是 turn 重置),WorkState 自动保留——对齐「同 turn 保留/跨 turn 产物重置」语义。

### 6.3 CheckpointStore 持久化

```rust
// turn 检查点时,WorkState 序列化进 CheckpointStore 旁路存储(不动文件 capture 语义)
checkpoint_store.capture_work_state(turn_id, &work_state)?;
// 崩溃恢复时,从最近 turn 的 WorkState 快照恢复
let work_state = checkpoint_store.restore_work_state(latest_turn_id)?;
```

WorkState 挂在 `SessionCoordinator` 内(复用现有 `Arc<RwLock>` 锁层级,不引入新锁)。legacy turn 缺 WorkState 时返回 `WorkState::default()`,向后兼容。

## 7. 错误处理边界

| 场景 | 行为 |
|------|------|
| verify 节点越权写 verify_result(防御性,实际不发生) | `ContractViolation{State}`,WorkState 不变 |
| 非 verify 节点读/写 verify_result(未授权) | `ContractViolation{State}`,返回 Err |
| WorkState 持锁竞争 | 复用 coordinator 现有锁层级,无新锁 |
| CheckpointStore 恢复时 WorkState 缺失(legacy turn) | 返回 `WorkState::default()`,向后兼容 |
| `VerifyOutcome::from` 转换时 VerifyResult 字段缺失 | fail_reason 为 None 即 `VerifyOutcome{success, fail_reason: None}`,不 panic |

## 8. 测试策略

- **schema 单测**:`VerifyOutcome` serde 往返;`FieldPerms` 矩阵对 5 个 NodeType 符合预期。
- **权限强制单测**:verify 节点正常写 verify_result;非 verify 节点越权写 → `ContractViolation{State}`;读越权报;授权写记 step_log、读不记。
- **pilot 降级点单测**:verify 失败后 WorkState.verify_result.fail_reason 为强类型枚举(非 `format!("{:?}")` 文本);retry 决策读枚举分支(CommandFailed vs BoundaryViolation)。
- **turn 继承单测**:同 turn retry 后 WorkState 保留;`begin_turn` 后产物重置、requirement 继承。
- **CheckpointStore 单测**:写入 WorkState 后崩溃→恢复→字段完整;legacy turn 缺 WorkState 返回空状态不崩。
- **零回归**:`verify_and_complete`/`verify_node` 现有测试全绿(verify_log 不变、retry 预算不变、session status 流转不变);既有节点契约/派发/契约强制测试零回归;与 dispatch-telemetry 正交。

## 9. Spec Patch(回写 delta spec)

brainstorming 发现 proposal/spec 的 pilot 措辞与真实代码不符,需回写(只改措辞和 Scenario,不动 Requirement 结构):

1. **Requirement「路由判定读取结构化字段而非解析文本」**:措辞从「至少一个真实存在的路由判定(pilot)」精确化为「修复 verify_node 出口的 `format!("{f:?}")` 结构化降级点」;Scenario 从「下一跳判定」改为「retry 决策与下游读取强类型 VerifyFailure 枚举分支」。
2. **Requirement「节点对状态字段的访问受权限约束」**:补一条 Scenario:越权写报 `ContractDimension::State`(确认新增维度)。
3. **Requirement「强类型共享工作状态」**:字段清单措辞明确本期强制 verify_result,其余字段(diff/test/budget 等)为 Option 占位、强制逻辑留后续 change。

## 10. 风险

- **[Risk] ContractDimension 加 State 变体破坏 exhaustive match** → 落地时全仓库检查 `match` on ContractDimension / CoordinatorError,补 arm 或 `_ =>`。serde 已派生,向后兼容。
- **[Risk] WorkState 与 verify_log 双写数据漂移** → 两者职责分明(verify_log=历史轨迹/离线排查,WorkState=当前状态/retry 决策),不要求强一致;pilot 路径两者都写但语义独立。
- **[Risk] turn 间重置边界判断错误** → 以 `begin_turn` 为跨 turn 边界信号重置产物字段;retry 是 node 级重试不触发 turn 重置。
- **[Trade-off] NodeVerifyResult.failure_reason 兼容期保留** → 换零回归,代价是 String 字段暂留;后续 change 可彻底移除。
