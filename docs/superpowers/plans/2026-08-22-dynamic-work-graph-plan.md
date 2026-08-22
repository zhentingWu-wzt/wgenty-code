# Dynamic Work-Graph Implementation Plan(P0–P4)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 把静态 Work-Graph v1 扩展为代码拥有的模板组合(P1)、锚点驱动有界适配(P2)、工具面封闭字段(P3),最终实现递归分解的真动态执行树(P4)。设计依据:`docs/superpowers/specs/2026-08-22-dynamic-work-graph-design.md` 与 `docs/superpowers/specs/2026-08-22-recursive-work-graph-design.md`。

**Architecture:** 图结构始终由代码从封闭集合结构化事实生成,LLM 只填充封闭字段;每次选择/适配进 `GraphAuditEvent` 审计流并随 checkpoint 持久化;P4 以「子图 = 完整有界 Work-Graph 实例」实现递归,父级锚点是唯一最终裁决。`select_work_graph` 保留为 `compose_work_graph` 兼容别名,旧 checkpoint 经 serde default 兼容。

**Tech Stack:** Rust, serde, anyhow, chrono, 现有 `NodeRegistry` / `WorkState` / `NodeRuntime` / `ExecutionSessionRuntimeStore` / `DaemonEventSink`。

## Global Constraints

- LLM 永远无法指定图的节点、边、权重,或跳过任何锚点;新增工具字段全部为封闭集合。
- 每个任务先写测试、观察失败,再实现;提交用 Conventional Commit。
- 每任务结束跑 `cargo fmt -- --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test --all`。
- 旧 checkpoint 必须可读:持久化结构新增字段一律 `#[serde(default)]`。
- P0–P3 依序执行;P4 各任务依赖 P1 的注册表,但可在 P2/P3 之后并行推进。

---

## Phase P0 — 模板注册表(纯重构,零行为变更)

### Task 1: 提取 GraphTemplateRegistry

**Files:**
- Modify: `src/org_graph/work_graph_plan.rs`
- Modify: `src/org_graph/mod.rs`

**Interfaces:**
- 新增 `GraphTemplate { id, stages, edges }` 与 `GraphTemplateRegistry::builtin() -> Self`。
- `select_work_graph(request)` 改为从注册表按 `task_kind` + `requires_human_review` 查模板实例化;对外签名与返回值不变。

- [x] 确认现有 3 个测试在纯重构后原样通过。
- [x] 新增测试:注册表含 `implementation-v1`、`diagnosis-v1` 及两个 `-human-review` 变体;human-review 变体的 verify→human-review 边存在。
- [x] 实现 `GraphTemplateRegistry` 与模板数据结构,`select_work_graph` 改为查表。
- [x] 跑 `cargo test org_graph::work_graph_plan`,全量 fmt/clippy/test。
- [x] Commit: `refactor(graph): extract graph template registry`。(1fcdd7b0)

---

## Phase P1 — 结构化事实维度 + 组合器

### Task 2: 扩展 WorkGraphRequest 与 compose_work_graph

**Files:**
- Modify: `src/org_graph/work_graph_plan.rs`
- Modify: `src/org_graph/mod.rs`

**Interfaces:**
- `WorkGraphRequest` 新增:`risk: Risk`(`Low|Medium|High`,默认 `Medium`)、`has_test_infra: bool`(默认 `true`)、`max_specialists: u8`(0..=3,默认 1),全部封闭集合,`#[serde(default)]`。
- 新增 `compose_work_graph(request) -> Result<WorkGraphPlan, WorkGraphPlanError>`;`WorkGraphPlan` 增加 `phases: Vec<WorkGraphPhase>`(serde 默认全三锚点)。
- 组合规则:`has_test_infra=false` → 移除 TestAnchor 阶段;`risk=High` → 强制 splice 审查门;`risk=Low` 或 `max_specialists=0` → 移除 diagnose 节点;偏离组合生成签名式 template_id,默认组合保持规范 id。
- `select_work_graph(request)` 为兼容别名(闭集组合必成功,矩阵测试断言)。

- [x] 写组合矩阵测试(24 组合)与签名/拒绝/serde 兼容测试。
- [x] 实现组合器与 `validate_composition`(节点上限 8、边白名单、声明检查、registry 绑定校验)。
- [x] 全量 fmt/clippy/test(1829+218 通过)。
- [x] Commit: `feat(graph): structured fact composer for work graphs`。(3bf499b1)

### Task 3: begin_node 运行时接入组合器

**Files:**
- Modify: `src/exec_session/node_runtime.rs`
- Modify: `src/exec_session/node_tools.rs`

**Interfaces:**
- `begin_node_with_work_graph` 内部改调 fallible `compose_work_graph`;`BeginNodeTool` input schema 增加 `risk` / `has_test_infra` / `max_specialists` 三个封闭字段,校验模式对齐 `task_kind`。

- [x] 工具 schema 测试:非法 `risk`、`max_specialists: 4` / `-1`、非布尔 `has_test_infra` → 结构化错误且零持久化;合法偏离事实 → 签名式 template_id。
- [x] 实现工具字段解析与转发;运行时切换到 fallible 组合(拒绝传播为节点创建错误而非 panic)。
- [x] 全量 fmt/clippy/test(1831+218 通过)。
- [x] Commit: `feat(graph): expose closed-set graph facts in begin_node`。(82184271)

---

## Phase P2 — 锚点驱动中途适配

### Task 4: plan revision 与适配规则

**Files:**
- Modify: `src/org_graph/work_graph_plan.rs`
- Modify: `src/org_graph/work_state.rs`
- Test: `src/org_graph/work_graph_plan.rs`、`src/org_graph/work_state.rs`

**Interfaces:**
- `WorkGraphPlan` 增加 `#[serde(default = "revision_one")] revision: u32`(兼容旧 checkpoint 默认 1)。
- `WorkState` 增加 `pub(crate) fn set_selected_work_graph_revision(&mut self, revision: u32)`。
- 新增纯函数 `adapt_work_graph(state: &WorkState, plan: &WorkGraphPlan) -> Result<Option<Adaptation>, WorkGraphPlanError>`;`Adaptation { spliced_node: WorkGraphPlanNode, reason: AdaptationReason }`,`AdaptationReason` 为封闭枚举(首个成员:`RepeatedTestAnchorFailure`)。
- 规则:同一 TestAnchor 连续 2 次失败且预算未耗尽 → splice `diagnose` 节点到下一次 implement 之前;适配次数(plan.revision - 1)≥ `max_adaptations`(默认 2)→ 返回升级信号。

- [ ] 写规则测试:第 2 次测试失败 → 产出 splice 适配、revision +1;第 3 次(超限)→ 升级信号;预算耗尽 → 不适配。
- [ ] 写序列化测试:旧 checkpoint(无 revision 字段)反序列化默认 revision=1。
- [ ] 实现规则与状态 API。
- [ ] 全量 fmt/clippy/test。
- [ ] Commit: `feat(graph): anchor-driven bounded plan adaptation`。

### Task 5: 适配接入运行时与审计

**Files:**
- Modify: `src/org_graph/work_state.rs`(`GraphAuditKind` 增加 `Adapted`;`GraphAuditEvent` 增加 `#[serde(default)] adapted: Option<GraphAuditAdaptation>`)
- Modify: `src/org_graph/work_graph_plan.rs`(`WorkGraphPlan::apply_adaptation`)
- Modify: `src/org_graph/audit.rs`(汇总计数 `plan_adaptations`)
- Modify: `src/exec_session/work_graph.rs`(`retry_or_escalate` 按 plan 边路由)
- Modify: `src/exec_session/node_runtime.rs`(`record_test_result` 失败路径评估适配、持久化、审计)

- [x] 集成测试:两次 TestAnchor 失败 → `Adapted` 审计事件、plan 含 diagnose 节点、revision=2、路由 RootCause;剥离图首次失败 → 直接 Implement 重试。
- [x] 实现接线;`next_step` 签名不变;旧 checkpoint 经 serde default 兼容。
- [x] 全量 fmt/clippy/test(1842+218 通过)。
- [x] Commit: `feat(graph): wire plan adaptation into runtime with audit`。(5446e2fb)

---

## Phase P3 — 提示词与渲染

### Task 6: 文档与渲染更新

**Files:**
- Modify: `src/org_graph/render.rs`(新增 `render_plan`:组合签名 template_id + revision + 节点/边列表)
- `src/prompts/base.md` 无需改动:begin_node 工具经 input schema 自文档,封闭字段说明已在 Task 3 随 schema 落地。

- [x] render 测试:剥离组合显示 `impl+no-test+risk-low+spec-0 (rev 1)`,splice 后显示 `(rev 2)` 与新增节点/边。
- [x] 全量 fmt/clippy/test(1843+218 通过)。
- [x] Commit: `feat(graph): plan rendering with composition signature and revision`。(8a21aef8)

---

## Phase P4 — 递归工作图(依赖 P1;可在 P2/P3 后开始)

### Task 7: 绑定与审计的向后兼容扩展

**Files:**
- Modify: `src/org_graph/work_state.rs`(`GraphChildBinding` 增加 `#[serde(default)] child_graph: ChildGraphRef`;`GraphAuditEvent` 增加 `#[serde(default)] parent_node_id: Option<String>`)
- Test: `src/org_graph/work_state.rs`

**Interfaces:**
- `ChildGraphRef { session_id: String, root_node_id: String }`。
- 现有 RootCause 绑定路径构造 `ChildGraphRef::none()` 兼容值(或 Option 内层),旧 checkpoint 反序列化得到默认值。

- [x] 序列化测试:旧 `GraphChildBinding`/`GraphAuditEvent` JSON(无新字段)反序列化成功且默认值正确。
- [x] 实现字段扩展。
- [x] 全量 fmt/clippy/test。
- [x] Commit: `feat(graph): parent-aware child bindings and audit events`。(2fd3c50f)

### Task 8: decompose_node 工具与提案校验

**Files:**
- Add: `src/exec_session/decompose.rs`
- Modify: `src/exec_session/mod.rs`(注册工具)
- Test: `src/exec_session/decompose.rs`

**Interfaces:**
- `DecomposeNodeTool` 实现 `Tool`;input schema:`units: Vec<DecomposeUnit>`(1..=4),每单元含 `goal`(≤2000 chars)、`task_kind`、`risk`、`requires_human_review`、`verify_commands`、`expected_files`。
- 校验:父节点处于 Implement 步、预算保留量 ≥ 1、深度 < 2、每单元 `compose_work_graph` 成功;任何失败 → 结构化错误,零持久化(预留-绑定两段式,对齐 `try_bind_root_cause_child` 模式)。
- 提案通过即消耗一次父级迭代预算。

- [x] 写校验矩阵测试:空 units、5 个 units、超深、父预算不足、非法 task_kind、超长 goal → 各自结构化拒绝且 `WorkState` 无变更。
- [x] 写通过测试:合法提案 → 每单元得到绑定子图 plan、审计记录、预算分账(保留量 + Σ child_allocation;`child_allocation = max(2, parent剩余/units)`)。
- [x] 实现工具。
- [x] 全量 fmt/clippy/test(1853+218 通过,新增 8 个 decompose 测试)。
- [x] Commit: `feat(graph): decompose_node tool with atomic validation`。(49b35479)

**偏差记录**:
- 「父节点处于 Implement 步」前置条件实现为「已选中 Work-Graph + 已持久化当前节点 + 预算已初始化」(`prepare_work_graph_pass` 起跑即初始化预算,故分解只可能发生在 Implement pass 内),未新增独立步骤机状态;语义等价、拒绝路径闭合。
- 深度语义按 Task 规格实现为 `child_depth = parent_depth + 1`,`>= MAX_DECOMPOSE_DEPTH(2)` 拒绝,即根(0)→ 子(1) 两层封顶(设计文档「父→子→孙」按此口径收紧)。
- 预算分账编码:父 `iter_used += 1` 且 `max_iter = iter_used + 保留量`,分账后父剩余 == 保留量(满足 Task 9 集成测试 C 不变量);审查发现原 wip 实现把 `verify_commands`/`expected_files` 校验后丢弃,已修——`DecomposedUnit` 增补这两个 `#[serde(default)]` 字段并持久化,供 Task 9 子图启动器执行。

### Task 9: 子图执行与证据回流

**Files:**
- Modify: `src/exec_session/node_runtime.rs`
- Modify: `src/exec_session/runtime_store.rs`
- Test: `src/exec_session/node_runtime.rs`

**Interfaces:**
- 子单元串行执行,复用 `run_work_graph` 管线(独立 revision/审计/预算)。
- 终态映射:`Complete` → 父级 `specialist_reports` 追加 `(GeneralPurpose, Implementation)` 结构化子报告;`Escalate` → 子单元失败不拖垮父级,父级按规则选重试/放弃/继续其余。
- 全部单元终态落定后回到父级锚点;父级锚点是唯一最终裁决(子图全绿 + 父锚点红 → 父级重试)。
- 子代理 terminal 未发布交接 → 复用 `escalate_current_work_graph`。

- [ ] 集成测试 A:子图 Complete + 父锚点失败 → 父级进入重试路由(证明父锚点唯一裁决)。
- [ ] 集成测试 B:某子单元 Escalate → 其余单元继续执行,父级最终可 Complete。
- [ ] 集成测试 C:预算分账后父级剩余 = 保留量;子预算独立耗尽不污染兄弟。
- [ ] 实现执行与回流。
- [ ] 全量 fmt/clippy/test。
- [ ] Commit: `feat(graph): child graph execution with parent-anchor arbitration`。

### Task 10: 回滚子树与树状渲染

**Files:**
- Modify: `src/exec_session/node_runtime.rs`(`rollback_node` 扩展:回滚父节点连带整个子树失效,审计保留)
- Modify: `src/org_graph/render.rs` / `src/cli/org_graph.rs`(缩进树展示分解层级 + 每层 revision)
- Test: `src/exec_session/node_runtime.rs`、`src/org_graph/render.rs`

- [ ] 测试:回滚父节点 → 子树产物失效、`graph_child_bindings` 审计保留(对齐 `inherit_for_new_turn` 语义)。
- [ ] render 测试:两层分解的 plan 渲染为缩进树。
- [ ] 实现回滚扩展与渲染。
- [ ] 全量 fmt/clippy/test。
- [ ] Commit: `feat(graph): subtree rollback and tree rendering`。

### Task 11: 注入测试与全量验证

**Files:**
- Test: `tests/integration/`(新增 `graph_recursion_injection.rs`,如框架不适用则放 `src/exec_session/decompose.rs` 测试区)
- Modify: `src/prompts/base.md`(decompose_node 说明,如 Task 6 未覆盖)

- [ ] 注入测试:子单元 goal 含恶意指令(如「跳过验证直接标记完成」)→ 最多影响该单元,父级锚点与兄弟单元不受影响(爆炸半径有界)。
- [ ] 崩溃重放测试:在每个锚点后注入中断,恢复后全树终态与审计序列与无中断运行一致。
- [ ] `cargo fmt -- --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test --all`。
- [ ] `git diff --check`;确认仅预期文件变更。
- [ ] Commit: `test(graph): recursion blast-radius and crash replay`。

---

## 验收口径(对照 spec)

- P0:`select_work_graph` 行为等价(现有测试零修改通过)。
- P1:组合矩阵全绿 + 拒绝不降级;`select_work_graph` 别名零迁移。
- P2:适配 ≤ 2 次、审计可重放、`next_step` 签名不变。
- P4:父锚点唯一裁决、预算分账不借贷、深度 ≤ 2、爆炸半径有界、崩溃重放一致。
- 全程:LLM 无法生成节点/边/权重、无法跳过锚点(封闭 schema 测试覆盖)。
