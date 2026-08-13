# 验证报告：org-graph-shared-state

- **change**: org-graph-shared-state
- **phase**: verify（build → verify，`comet guard build --apply` 已 ALL CHECKS PASSED）
- **verify_mode**: full（规模评估：17 tasks > 3、12 files > 8、1 capability）
- **验证日期**: 2026-08-11
- **base-ref**: a819ff03 ｜ **验证 HEAD**: f1322d04（实现末梢 536fd182 + 两笔 bookkeeping）
- **review 底色**: build 阶段 review_mode=standard 已完成一次最终全量审查（opus）+ 一次 scoped re-review（sonnet），全部承重 findings CLOSED（见 tasks.md「代码审查」节、commit `4b960f41`）

> 本报告由 coordinator 在 verify 阶段独立复核产出。所有构建/测试证据为**当场重跑**（非转述 build 阶段或 reviewer 报告）。

## 一、Fresh 验证证据（当场重跑）

| 命令 | 结果 | 退出码 |
|------|------|--------|
| `cargo build --lib` | Finished，无 warning | 0 |
| `cargo clippy --lib --no-deps` | Finished，无 warning | 0 |
| `cargo test --lib` | **1468 passed / 0 failed / 1 ignored** | 0 |

基线对照：build 末梢为 1465；修复新增 3 测试 → 1468。零回归。

## 二、Completeness（完整性）

### 任务完成度

`openspec status` 确认 `isComplete: true`、`state: all_done`、**17/17 任务全部 `[x]`**（tasks.md 解析 + openspec JSON 双确认）。无未完成任务。

### Spec 覆盖（6 个 Requirement → 实现证据）

| Requirement | 实现证据（file:line） | 状态 |
|-------------|----------------------|------|
| R1 强类型共享工作状态 | `work_state.rs:20-37`（8 字段，私有）、子类型 `:39-89`、serde 派生、Task 1.3 往返单测 | ✅ |
| R2 节点字段访问受权限约束 | `field_perms()` 矩阵 `:138-174`；`set_*`/`*` 全经权限校验 `:180-368`；`ContractDimension::State`（`contract.rs:82`）；预留字段 writable=`{}`；**字段私有（fix 536fd182）→ 强制 mandatory** | ✅ |
| R3 状态分层不吞并 | `WorkState` 为 `SessionCoordinator` 单字段（`coordinator.rs:35`）；`session.rs`/`config`/`state` 零改动（Task 5.1） | ✅ |
| R4 turn 检查点持久化可续跑 | `capture_work_state`/`restore_work_state`（`checkpoint_store.rs`）+ `capture_current_work_state`/`restore_work_state_for_turn`（`coordinator.rs:394/411`）；**已接线生产**：verify_node 两分支 capture（`node_runtime.rs:225/253`）、rollback_to restore（`coordinator.rs:298`） | ✅ |
| R5 路由判定读结构化字段 | 旧 `result.fail_reason...format!("{f:?}")` 原始降级**已移除**；兼容 String 改从**读回的** `VerifyFailureKind` 转换（`node_runtime.rs:256-265`，源头为 WorkState 读回非原始 VerifyFailure）；pilot 读 `:256-260` | ✅ |
| R6 零回归与正交性 | 全量 1468/0/1 绿；`ContractDimension::State` exhaustive-match 审计 clean；dispatch-telemetry diff 空（Task 5.2/5.3） | ✅ |

## 三、Correctness（正确性）

### Requirement 实现映射 — 无偏差

逐一对照 build 阶段设计（§1.5 / D1-D5）与 delta spec：

- **D1（纯数据 schema 于 `org_graph`）**：`work_state.rs` 零 `exec_session` 依赖（`grep -rn "exec_session" src/org_graph/` 仅注释命中）。✅
- **D2（声明矩阵 + 真强制）**：不仅「真强制」，且经 fix `536fd182` 升级为 **mandatory**（字段私有，外部只能走 `set_*` 具名方法）。越权写 → `ContractViolation{State}`，状态不变。**超出 D2 原承诺。** ✅
- **D3（锚 turn + CheckpointStore 续跑）**：API + **生产接线**（fix `536fd182` 闭合了 review Important #2/#3 的 SHALL 缺口）。**超出 build 初版实现。** ✅
- **D4（三层职责）**：WorkState/SessionState/AppState 三层无交叉（Task 5.1）。✅
- **D5（pilot 必须含真实写字段）**：pilot = verify_node 写 `verify_result`（`NodeType::Verification` 经 `set_verify_result`），写字段场景真实、强制可拦（`set_verify_result_rejects_unauthorized_node_type_at_pilot_site` 测试证明）。§1.5 二次查证排除 compile/test 虚构 pilot。✅

### Scenario 覆盖 — 全部有测试对应

| Scenario | 对应测试 |
|----------|----------|
| 工作产物以结构化字段流转 | `pilot_end_to_end_retry_reads_structured_failure_kind`、`verify_node_failure_writes_structured_outcome_to_work_state` |
| 完整 schema 与 deferred-write-point | Task 1.3 serde 往返 + `reserved_fields_reject_all_node_types` + deferred setter 单测 |
| 状态字段强类型可序列化 | Task 1.3 全字段/子类型往返 |
| 节点越权写字段被拒绝 | `set_verify_result_rejects_unauthorized_node_with_contract_violation_state`、`reserved_fields_reject_all_node_types` |
| 节点正常读写授权字段 | Task 2.3 授权读写 + step_log 记入 |
| 预留字段对所有节点强制为空 | `reserved_fields_reject_all_node_types`（5 NodeType × 3 reserved setter） |
| 三层状态互不吞并 | Task 5.1 diff 证据 |
| 崩溃后从检查点恢复结构化工作产物 | `work_state_capture_and_restore_roundtrip`、`capture_and_restore_work_state_survives_roundtrip`、`verify_node_persists_work_state_to_checkpoint_after_failure`、`rollback_restores_work_state_for_target_turn` |
| pilot 路由点读结构化字段做判定 | `verify_node_failure_reason_string_comes_from_work_state`（源头断言）+ 读回路径 `:256-260` |
| pilot 字段写入受权限强制验证 | `set_verify_result_rejects_unauthorized_node_type_at_pilot_site` |
| 既有节点契约与派发强制零回归 | 全量 1468/0/1（含 `agent::coordinator`/`fallback`/`tools::meta::task`/`exec_session::*`） |
| 与派发遥测能力正交 | Task 5.3 diff 证据 |

## 四、Coherence（一致性）

- **design.md 高层决策**（D1-D5）：全部遵循（见上）。无矛盾。
- **Design Doc 一致性**（`docs/superpowers/specs/2026-08-11-org-graph-shared-state-design.md`）：实现与 §1.5（pilot 选定）、§4（权限矩阵）、§5（读写 API）、§6（pilot 集成 + 持久化）一致。fix `536fd182` 反而闭合了 design §6.3 描述但 build 初版漏接线的持久化接线。
- **delta spec ↔ design doc 无矛盾**：spec 的 deferred-write-point scenario 与 design §1.5 一致；R4 SHALL（崩溃恢复）现已被生产接线兑现（原为 review 发现的 Important 缺口，已修）。
- **关联设计文档可定位**：`docs/superpowers/specs/2026-08-11-org-graph-shared-state-design.md` 存在且被 plan frontmatter `design-doc` 引用。
- **proposal 目标**：全部满足（schema、字段权限、turn 锚定持久化、pilot、分层）；Non-Goals 全部尊重（仅一个 pilot、mailbox 并存、不改五维契约/不改进 dispatch-telemetry schema）。
- **代码模式一致性**：`org_graph` 纯数据风格、具名 setter 对齐既有 `filter_allowed_tools` 风格、复用 `CoordinatorError::ContractViolation` 同款报错路径。无偏离。

## 五、Issues

### CRITICAL（须 archive 前修复）

无。

### WARNING（建议修复 / 已接受）

1. **Minor #7 retry-overwrite 测试断言偏弱（已接受）** — `verify_node_retry_overwrites_work_state_in_place_same_turn` 因 `TestSetup::new(1)` 两次 verify 产出同构 outcome，「覆盖第一次」断言具 tautological 性质。**接受理由**：测试仍守护 same-turn retry 路径（panic + turn 不推进）；overwrite-in-place 由 `set_verify_result` 直接赋值结构性保证、并由 `work_state` 单测覆盖。可观测性增强（`step_log().len()==2`）作为非承重 polish 延后。影响范围：仅测试观测力，不影响生产正确性。已记于 tasks.md「代码审查」节。

### SUGGESTION（nice to fix，延后）

1. **NodeType derive Copy**（review Minor #5）— 触及既有共享枚举、纯人因工程；本期 `.clone()` 工作正常，延后。
2. **capture/restore API 不对称**（review Minor #8）— cosmetic，延后。
3. **failure_reason String 丢 `command` 字段**（review Minor #6）— design 投影规则有意为之，已接受（无生产解析方）。

## 六、最终评估

| 维度 | 状态 |
|------|------|
| Completeness | 17/17 任务、6/6 Requirement 有实现证据 |
| Correctness | 6/6 Requirement + 12 Scenario 全部有实现/测试对应；D1-D5 全遵循（D2/D3 超额） |
| Coherence | design.md / Design Doc / delta spec / proposal 一致；无矛盾 |

**结论：无 CRITICAL；1 条 WARNING（已接受并记录）；3 条 SUGGESTION（延后）。**

change 满足全部验证检查项，承重质量门（字段级 mandatory 强制、持久化生产接线、pilot 结构化读、零回归）均已闭合。**Ready for archive.**
