# Recursive Work-Graph Design(P4:递归分解的真动态执行树)

## Goal

在不破坏锚点体系公理(提案/证据分离、可重放、预算强制、注入免疫)的前提下,实现 L4 级动态
性:**执行树随运行时发现无界生长,而每个单元内部仍是代码拥有的静态图**。

`implement` 节点可以把自身分解为一组子工作单元(每个子单元是一份完整的有界 Work-Graph 实
例)。LLM 拥有「分解成什么」(提案),代码拥有「每个单元如何被验证」(裁决)——与对待代码
edit 完全同构。

前置:主设计文档 `2026-08-22-dynamic-work-graph-design.md`(P1 模板注册表)。

## Invariants(继承,不可协商)

- 子图只能是代码拥有的有界模板实例(复用 `compose_work_graph`);LLM 无法指定子图的节点、
  边或跳过子图锚点。
- 父节点成功与否**只**由父级锚点决定;子图 `Complete` 只是证据输入,不是成功信号。
- 每份子图有独立预算(从父预算切分),独立审计流;预算不允许跨子图借贷。
- 全树可重放:持久化状态 + 代码唯一确定后续路由,任意深度成立。
- 注入爆炸半径有界:被污染的分解提案最多产生 N 个「各自必须活过锚点」的子单元,无法改写
  验证计划的形状。

## Scope

### 1. 分解提案:`decompose_node` 工具

新增 agent 工具(封闭 schema,同 `begin_node` 模式):

```text
decompose_node {
  units: Vec<DecomposeUnit>       // 1..=MAX_UNITS(初始 4)
}
DecomposeUnit {
  goal: String,                   // 有界长度
  task_kind: Implementation|Diagnosis,
  risk: Low|Medium|High,
  requires_human_review: bool,
  verify_commands: Vec<String>,   // 交给 runtime 执行,agent 不可伪造结果
  expected_files: Vec<String>,
}
```

- 调用时父节点处于 `Implement` 步、预算未耗尽;否则结构化错误拒绝。
- 每个子单元经 `compose_work_graph` 生成计划并强校验;任何单元校验失败 → 整个提案拒绝,
  不产生半持久化状态(原子性,对齐 `try_bind_root_cause_child` 的预留-绑定模式)。
- 提案消耗一次父级迭代预算(分解是昂贵的,防止用分解逃避重试)。

### 2. 子图执行与证据回流

- 子单元逐个串行执行(P4 不做并行;容量语义与 P1 一致);每份子图复用现有
  `run_work_graph` 管线,有自己的 `revision`、适配规则、审计流。
- 子图终态映射:
  - `Complete` → 父级 `specialist_reports` 追加一条 `Implement` 产出的结构化子报告
    (复用 `(producer, kind)` 去重);
  - `Escalate` / 预算耗尽 → 子单元失败,**不**自动拖垮父级;父级按既定规则决定重试该单元、
    放弃该单元(继续其余单元)或整体 Escalate。
- 全部单元终态落定后,父级回到自身锚点(compile → test → verify)。**父级锚点是唯一的最
  终裁决**;子图全绿但父级锚点红 → 父级按既有规则重试/升级。

### 3. 审计与父子绑定

- 复用并扩展 `GraphChildBinding`(现仅 RootCause)→ 通用化 `role: NodeType` 为
  `GraphChildBinding { node_id, attempt, role, child_agent_id, child_graph: ChildGraphRef, timestamp }`。
- `GraphAuditEvent` 增加 `parent_node_id: Option<NodeId>`,审计流构成树;渲染层
  (`render.rs` / `cli/org_graph.rs`)以缩进树展示分解层级与每层 revision。
- 重放语义:任一崩溃点恢复后,自根向下由持久化锚点状态确定性导出全部后续路由。

### 4. 预算分账

```text
parent.budget.max_iter = 保留量 + Σ child_allocation
child_allocation = max(MIN_CHILD_ITER(2), parent 剩余 / units)
```

- 子预算从父级切分时持久化;父级保留量低于 `MIN_PARENT_ITER(1)` → 拒绝分解提案。
- `token_used` 同口径累计上报父级;跨 checkpoint 恢复后分账账本随 `WorkState` 走。

### 5. 回滚与深度上限

- `rollback_node` 语义扩展:回滚到某节点 = 连同其整个子树(`inherit_for_new_turn` 已保留
  审计与绑定,子树产物随父节点 checkpoint 一并失效)。
- 初始深度上限 `MAX_DEPTH = 2`(父 → 子 → 孙);超深提案 → 结构化错误。上限可配置,但
  默认保守。

## Alternatives Considered

1. **平面动态图**(LLM 在单图内加节点/边)。否决:公理级张力,详见主设计文档 Alternatives #1
   ——语义新颖结构的生成器只能是 LLM,而 LLM 信号恰是锚点体系不信任的输入。
2. **子图结果直接作为父级成功信号**。否决:子图 `Complete` 只是提案通过了子级证据;父级
   产物必须整体活过父级锚点,否则「验证这个图」退化为循环论证。
3. **并行执行子单元**。Deferred:需要并发子代理 + 预算分账的线程安全设计;P4 先串行证明
   递归语义,并行是纯执行层优化。
4. **子图失败即父级失败**。否决:分解的意义就是隔离失败域;父级应保留「放弃单元、继续其
   余」的代码拥有选项。

## Error Handling and Recovery

- 分解提案非原子失败:预留-绑定两段式(对齐 RootCause 派发),失败时无残留状态。
- 子单元执行中崩溃:子图状态已随每次锚点 `capture_current_work_state()` 持久化;恢复后
  `next_step` 从最新锚点续跑,不重放 LLM 历史。
- 深度/单元数/预算任何一项越界:结构化错误 + 审计记录,节点保持可继续状态(Escalate 仅在
  预算耗尽或 BoundaryViolation 时发生)。
- 子代理 terminal 但未发布必需交接:复用 `escalate_current_work_graph` 的静态路由失败同
  步机制。

## Testing

- 提案校验:非法 `task_kind`/超 `MAX_UNITS`/超深/父预算不足 → 结构化拒绝,零持久化。
- 证据回流:子图 Complete → 父级锚点红 → 父级重试(证明父锚点是唯一裁决);子图 Escalate
  → 父级可选择继续其余单元。
- 预算分账:分账后父级剩余 = 保留量;子预算独立耗尽不污染兄弟单元;checkpoint 恢复后账本
  一致。
- 重放:随机崩溃点注入(kill after each anchor),恢复后全树终态与无崩溃运行逐字节一致
  (审计序列 + 最终产物)。
- 回滚:回滚父节点 → 子树审计保留、产物失效(与现有 `inherit_for_new_turn` 语义对齐)。
- 注入:子单元 goal 含恶意指令 → 最多影响该单元,父级锚点与兄弟单元不受影响(爆炸半径有
  界证明)。

## Rollout / Migration

1. 依赖 P1(模板注册表 + 组合器)合入。
2. 第一步:审计与绑定字段的向后兼容扩展(`#[serde(default)]`,旧 checkpoint 可读)。
3. 第二步:`decompose_node` 工具 + 串行子图执行,`MAX_DEPTH=2`、`MAX_UNITS=4` 默认开启,
   配置可降为 0(完全关闭递归,回退平面行为)。
4. 第三步:渲染层树状展示与 `prompts/base.md` 工具说明更新。
5. 观察期后评估:是否放开 `MAX_DEPTH`、是否引入并行子单元。

## Non-goals

- 单图内平面生成节点/边(永久否决)。
- 子图并行执行、跨子图预算借贷(远期)。
- LLM 指定子图拓扑、跳过子图锚点、直接裁决父级状态。
- 跨节点(非父子)图合并。
