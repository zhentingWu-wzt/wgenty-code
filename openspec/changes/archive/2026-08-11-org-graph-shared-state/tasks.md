# Tasks

## 1. WorkState schema 与模块骨架

- [x] 1.1 新建 `src/org_graph/work_state.rs`，定义 `WorkState` struct，完整 7+1 字段 schema：`requirement: Option<String>` / `generated_diff: Option<GeneratedDiff>` / `compile_result: Option<CompileResult>`（含 `ok: bool` 与 `stderr`）/ `test_result: Option<TestResult>`（含 `pass: bool` 与 `failed_cases`）/ `human_review: Option<HumanReview>`（enum `Approve`/`Reject`）/ `verify_result: Option<VerifyOutcome>`（pilot 核心字段）/ `budget: Option<Budget>`（含 `max_iter` / `iter_used` / `token_used`）/ `step_log: Vec<StepRecord>`。同时定义子类型 `GeneratedDiff` / `CompileResult` / `TestResult` / `HumanReview` / `Budget` / `VerifyOutcome` / `VerifyFailureKind`（`CommandFailed{exit_code,stderr}` / `BoundaryViolation{unexpected_files}`）与权限/审计类型 `FieldPerms` / `WorkField`（8 变体）/ `StepRecord` / `StepAction`。所有类型派生 `Serialize/Deserialize/Clone/Debug`（枚举合理处加 `Copy/Hash/PartialEq/Eq`）
- [x] 1.2 在 `src/org_graph/mod.rs` 导出 `pub mod work_state;` 及相关公开类型
- [x] 1.3 为 schema 加单测：序列化往返（serialize → deserialize 字段等价，覆盖 `WorkState` 全字段与各子类型 `GeneratedDiff`/`CompileResult`/`TestResult`/`HumanReview`/`Budget`/`VerifyOutcome`/`VerifyFailureKind`）、默认值符合预期（全 `Option` 为 `None`、`step_log` 为空）、各结构化子字段类型不丢失

## 2. 字段级访问权限（真强制）

- [x] 2.1 设计并定义每个 `NodeType` 的「可读字段集 + 可写字段集」声明矩阵，覆盖全部 8 个 `WorkField`（含预留字段 `compile_result` / `test_result` / `human_review` 对所有现存 `NodeType` 的 writable 强制为 `{}`——这是全字段权限真强制的核心保证；矩阵细则见 design doc §4）
- [x] 2.2 实现 `WorkState` 受权限约束的读写 API：调用方提供 `NodeType`，越权写直接返回 `ContractViolation`（复用 `ContractDimension`，复用 `Permission` 还是新增 `State` 维度见 design 阶段定）
- [x] 2.3 为权限强制加单测：节点正常读写授权字段成功（含 GeneralPurpose 合成写 `generated_diff`/`budget`）、越权写字段被拒绝并触发 `ContractViolation{State}`、授权读不写审计日志而授权写记入 `step_log`、**预留字段强制为空**（任何 `NodeType` 调 `set_compile_result`/`set_test_result`/`set_human_review` → `ContractViolation{State}`）

## 3. turn 集成与检查点持久化

- [x] 3.1 把 `WorkState` 锚定到 `exec_session` 的 turn：turn 开始时创建/继承 `WorkState` 实例（turn 间继承策略见 design 阶段定，默认只读字段继承、可写字段按 pilot 语义）
- [x] 3.2 随 `CheckpointStore` 持久化 `WorkState`：turn 检查点时一并落盘，崩溃后从最近 turn 快照恢复结构化字段
- [x] 3.3 为 turn 集成加单测：写入结构化字段后崩溃→从检查点恢复→字段值完整；既有文件级 capture/rewind 行为零回归

## 4. pilot 路由点（读结构化字段 + 验证强制）

- [x] 4.1 二次查证已完成（design §1.5，带 file:line 证据）：compile 闭环不存在（`NodeType` 枚举 `src/org_graph/contract.rs:10-17` 无 `Compile` 变体、`parse_node_type` `src/tools/meta/task.rs:1070-1080` 不识别 compile、全仓库无 `compile_node`/`CompileResult`/`NodeType::Compile`）、test 闭环同理不存在（测试结果至多作不透明 exit code 塞进 `VerifyFailure::CommandFailed`，无 `failed_cases` 解析）——故 pilot 锚定唯一真实闭环 verify（`verify_gate.rs:208 verify_and_complete` → `VerifyResult{fail_reason: Option<VerifyFailure>}`，出口 `node_runtime.rs:204` 用 `format!("{f:?}")` 降级）。实现期按 design D5 硬约束复核降级点仍在原位
- [x] 4.2 实现 pilot 路由点：`verify_node` 出口从 `format!("{f:?}")` 降级改为「投影 `VerifyResult` → 经 `set_verify_result(NodeType::Verification, outcome)` 写入 WorkState（受字段级权限强制）→ 读回 `verify_result()` 的 `VerifyFailureKind` 枚举做 retry 决策」（不是 compile/test——查证确认二者闭环不存在，详见 design §1.5）
- [x] 4.3 确保 pilot 路由点涉及的写字段场景受 2.x 字段级权限强制（节点须声明可写该字段），并提供验证该强制的测试

## 5. 三层状态分层与零回归

- [x] 5.1 验证 `WorkState` 与 `SessionState`（会话骨架）/ `AppState`（全局配置）三层职责分明：`SessionState` 字段语义不变、`AppState` 完全不动
- [x] 5.2 `SubagentResultMailbox` 在非 pilot 路径维持原状（与 `WorkState` 并存，不强制同步、不强制淘汰）
- [x] 5.3 与 `org-graph-dispatch-telemetry` 正交性验证：两者字段/schema 互不依赖、互不修改

## 6. 集成验证

- [x] 6.1 `cargo build` 通过；`cargo test` 全绿（新增测试通过 + 既有节点契约 / 派发 / 契约强制 / transcript 测试零回归）
- [x] 6.2 手动验证 pilot 路由点按结构化字段正确路由（成功路径 + 失败回流路径），且越权写字段被拦截

## 代码审查（build → verify 前 · review_mode=standard）

最终全量代码审查（Superpowers `requesting-code-review`，opus）覆盖整 change `a819ff03..d5d82a21`，结论 *With fixes*：1 Critical + 2 Important + 5 Minor。coordinator 裁决后派发一次合并修复（commit `536fd182`），sonnet 实现、按 TDD，修复后 sonnet scoped re-review 复核确认。

**已修复（fix commit `536fd182`，re-review 全部 CLOSED）：**
- Critical #1 — `WorkState` 8 字段原为 `pub`，`work_state_mut()` 使调用方可绕过 `set_*` 权限强制（advisory 而非 mandatory），违背 change 核心论点「全字段权限真强制」与 design §5「step_log 不可直接 set」。修复：8 字段改私有；新增 `set_requirement`/`requirement()`（特权、coordinator 拥有、不记 step_log）与只读 `step_log()`；全部外部访问改走方法。生产 pilot（node_runtime.rs）本就用 setter，仅 test 绕过点被改写。
- Important #2 — `capture_current_work_state`/`restore_work_state_for_turn` 已实现但无生产调用方，spec SHALL「崩溃后从检查点恢复结构化工作产物」未兑现。修复：`verify_node` 成功+失败两分支在 `set_verify_result` 后调 `capture_current_work_state()`（写点落盘，覆盖中途崩溃场景）。
- Important #3 — `rollback_to` 恢复 git refs + 文件 + untracked 但未恢复 work_state。修复：`rollback_to` 移动游标后调 `restore_work_state_for_turn(turn_id)`（legacy turn → `default()`，向后兼容）。
- Minor #4 — `verify_node` 读回路径的 `.expect("just written")` 改为传播 `anyhow` 错误（读回本身保留——它是 pilot 的结构化读）。

**接受/延后（非 CRITICAL，按 review-gate 记录接受理由与影响范围）：**
- Minor #7（retry-overwrite 测试）— **接受**。测试存在且守护 same-turn retry 路径（panic + turn 不推进）；其「覆盖第一次」断言因 `TestSetup::new(1)` 两次 verify 产出同构 outcome 而偏弱（tautological），但 overwrite-in-place 由 `set_verify_result` 的直接赋值结构性保证、并由 `work_state` 单测覆盖。可观测性增强（`step_log().len()==2` 断言）作为非承重 polish 延后。re-review 标 PARTIALLY CLOSED、整体裁决仍 *Fix accepted: Yes*。
- Minor #5（`NodeType` derive `Copy`）— **延后**。触及既有共享枚举 derive、纯人因工程；本期 `.clone()` 调用工作正常。
- Minor #6（`failure_reason` String 丢失 `command` 字段）— **接受**。design 投影规则（`project_failure` 丢 `command`，retry 只需 `exit_code` 语义）有意为之；无生产解析方（`node_tools.rs:163` 作不透明文本透传，测试用 `contains`）。
- Minor #8（capture/restore API 不对称）— **延后**。纯 cosmetic。

**验证证据：** `cargo test --lib` 1468 passed / 0 failed / 1 ignored（基线 1465 + 修复新增 3 测试）；`cargo build --lib` 干净；`cargo clippy --lib --no-deps` 无 warning。scoped re-review 独立复跑确认同数。

**最终裁决：** 所有 CRITICAL/Important 已 CLOSED，无新问题、无回归。change 满足 build → verify review-gate，进入阶段守卫。
