# Brainstorm Summary

- Change: org-graph-shared-state
- Date: 2026-08-11

## 探索阶段关键发现(已确认事实,来自 CodeGraph 查证)

### 发现 1:verify→retry 闭环真实存在,且已部分结构化

`exec_session/verify_gate.rs:208 verify_and_complete` 已经:
- 真实执行 verify 命令(`self.executor.execute`,反编造)
- 产出强类型 `VerifyResult { success: bool, commands_run, fail_reason: Option<VerifyFailure>, ... }`
- `VerifyFailure` 是**枚举**:`CommandFailed { command, exit_code, stderr }` / `BoundaryViolation { unexpected_files }`——已带 exit_code 和 stderr 结构化字段
- 失败驱动 retry:`VerifyFailAction::AutoRetry { remaining }` → session 保持 InProgress 等下一次 node 执行

### 发现 2(决定性):结构化结果在流转中被降级成文本

`node_runtime.rs:204` 把结构化的 `VerifyFailure` 用 `format!("{f:?}")` 退化成 `failure_reason: Option<String>`,回传给 `NodeVerifyResult`。结构化的 `exit_code`/`stderr`/`unexpected_files` 因此流失成自然语言字符串。

**这正好印证 proposal 核心论点**:结构化产物在节点流转中被降级成自由文本。pilot 的真正价值**不是新建路由点**,而是修复这个已存在的「结构化→文本」降级点。

### 发现 3:pilot 范围比 proposal 设想的更小更精准

proposal 原设想「编译失败→回到代码生成」需要新建路由闭环。实际:`verify_node` 的 retry 闭环已存在(`verify_gate.rs` 的 `VerifyFailAction::AutoRetry`),pilot 只需把 `VerifyResult` 结构化写进 `WorkState`(取代/补充 `format!("{f:?}")` 降级),下游读 `state.verify_result.fail_reason` 枚举而非读 String。

**pilot 候选(待用户确认)**:不新建路由,而是把 `verify_node` 的结果写入路径从「`format!` 成 String」改为「结构化写 `WorkState`,retry 决策读字段」。范围更小、零新路由、直接命中核心论点。

## 确认的技术方案

### 方案确认 1:pilot = 修复 verify_node 出口的结构化降级点(用户已确认)

不新建路由。把 `node_runtime.rs:204` 的 `format!("{f:?}")` 降级点改为:
- verify 节点把 `VerifyResult` 结构化写进 `WorkState.verify_result`(取代/补充回传 `failure_reason: Option<String>`)
- retry 决策与下游读 `state.verify_result.fail_reason`(强类型 `VerifyFailure` 枚举)而非读 String

范围最小、零新路由、天然含「写字段」场景(verify 节点写 `verify_result`),满足 design D5 硬约束。

### 待确认(继续 brainstorm)

- ~~字段类型~~ → **已定:org_graph 内定义独立类型 `VerifyOutcome`**(success + fail_reason 枚举 + stderr 子集),`exec_session` 集成时负责 `VerifyResult` → `VerifyOutcome` 转换。保持 `org_graph` 模块内聚不依赖 `exec_session`。
- ~~字段权限矩阵形态~~ → **已定:`NodeType` 挂 `fn field_perms() -> FieldPerms`**(返回该节点可读/可写字段集),WorkState 读写 API 查表判定越权。显式、可测、强制逻辑集中。
- ~~ContractDimension~~ → **已定:新增 `State` 维度**。语义清晰(状态字段越权)。落地时需检查 `CoordinatorError`/`ContractDimension` 的 exhaustive match 调用点(serde 已派生,加变体向后兼容)。
- ~~WorkState 与 verify_log~~ → **已定:并存各管各**。verify_log 维持原状(每次 verify 追加的磁盘审计轨迹,离线排查用),WorkState.verify_result 只存当前最近一次(当前状态,retry 决策用)。职责分明,零改动 verify_log。
- ~~turn 间继承策略~~ → **已定:同 turn 内 retry 保留全部 / 跨 turn 时 `requirement` 继承、产物字段(verify_result/test_result 等)重置为新 turn 起点**。与 retry 闭环对齐。

## 关键取舍与风险

### 取舍
1. **pilot 范围最小化**:不新建路由,修复 `verify_node` 出口 `format!("{f:?}")` 降级点。代价:pilot 只覆盖 verify 这一个降级点,其他自由文本降级点(如 mailbox)留后续 change。接受——YAGNI。
2. **独立类型 `VerifyOutcome` vs 复用 `VerifyResult`**:选独立类型保持 `org_graph` 内聚。代价:`exec_session` 集成时多一层转换。接受。
3. **真强制 vs 软路径**:字段越权写直接 `ContractViolation`。代价:pilot 必须含真实写字段场景才有意义——verify 节点写 `verify_result` 满足此约束。

### 风险
- **[Risk] `ContractDimension` 加 `State` 变体破坏 exhaustive match** → 落地时全仓库检查 `match` on `ContractDimension` / `CoordinatorError`,补 `_ =>` 或新增 arm。serde 已派生,向后兼容。
- **[Risk] WorkState 与 verify_log 双写数据漂移** → 已定两者职责分明(verify_log=历史轨迹/离线排查,WorkState=当前状态/retry 决策),不要求强一致;pilot 路径两者都写但语义独立。
- **[Risk] turn 间重置边界判断错误** → 「同 turn retry」与「跨 turn」的判定依赖 `exec_session` 现有 turn 生命周期;落地时以 `begin_turn` 为跨 turn 边界信号重置产物字段。

## 测试策略

- **schema 单测**:`VerifyOutcome` serde 往返;`FieldPerms` 矩阵对 5 个内置 NodeType 的字段权限符合预期。
- **权限强制单测**:verify 节点正常写 `verify_result` 成功;非 verify 节点越权写 `verify_result` → `ContractViolation{dimension: State}`;授权读不写 step_log 而授权写记入。
- **pilot 降级点单测**:verify 失败后,WorkState.verify_result.fail_reason 为强类型枚举(非 `format!("{:?}")` 文本);retry 决策读枚举分支(CommandFailed vs BoundaryViolation)而非解析 String。
- **turn 继承单测**:同 turn retry 后 WorkState 保留;`begin_turn` 后产物字段重置、requirement 继承。
- **零回归**:`verify_and_complete` / `verify_node` 现有测试全绿(verify_log 行为不变、retry 预算不变、session status 流转不变);既有节点契约/派发/契约强制测试零回归。

## Spec Patch 候选(将回写 spec.md)

- Requirement「路由判定读取结构化字段而非解析文本」措辞精确化:pilot 不是「新建路由」,而是「修复 verify_node 出口的 `format!("{f:?}")` 结构化降级点」。对应 Scenario 从「下一跳判定」改为「retry 决策与下游读取强类型字段」。
- Requirement「节点对状态字段的访问受权限约束」补一条 Scenario:越权写报 `ContractDimension::State`(确认新增维度)。

## 分段确认记录(用户已逐段确认)

- **第 1 段(架构与核心类型)**:模块布局 `work_state.rs` 新增、`contract.rs` 加 State 变体、`registry.rs` 加 field_perms;pilot 只强制 verify_result,其余字段 Option 占位不写强制逻辑;VerifyOutcome 为 VerifyResult 精简投影。✅
- **第 2 段(权限矩阵与读写 API)**:Verification/GP 涉及 verify_result,其余 3 节点只读 requirement;每字段一具名方法(非泛型 get/set);读越权也报(读写对称)。✅
- **第 3 段(pilot 集成)**:NodeVerifyResult.failure_reason 兼容期保留但源头改为从 WorkState 读回;begin_turn 作跨 turn 重置边界(retry 不重置);CheckpointStore 加 work_state 旁路存储不动文件 capture 语义。✅
- **第 4 段(错误处理/测试/Spec Patch)**:5 类错误边界;6 组测试;3 处 Spec Patch 回写。✅

**整体设计已用户确认,进入 Design Doc 创建。**


## 关键取舍与风险

(待 brainstorming 推进后填写)

## 测试策略

(待 brainstorming 推进后填写)

## Spec Patch 候选

- spec.md 的 Requirement「路由判定读取结构化字段而非解析文本」措辞需精确化:pilot 不是「新建路由」,而是「修复 verify_node 出口的 `format!("{f:?}")` 结构化降级点」。Scenario 措辞相应调整(从「下一跳判定」改为「retry 决策与下游读取」)。

