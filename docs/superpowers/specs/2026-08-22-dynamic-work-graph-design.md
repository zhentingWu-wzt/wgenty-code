# Dynamic Work-Graph Design(代码拥有的图组合与锚点驱动适配)

## Goal

在保持「路由可复现、可审计、预算强制、LLM 永不拥有图结构」的前提下,把静态 Work-Graph v1(仅
`implementation-v1` / `diagnosis-v1` 两个硬编码模板)扩展为:

1. **代码拥有的模板组合**——`WorkGraphRequest` 携带更多封闭集合结构化事实,代码按规则组装出
   有界、经校验的图;
2. **锚点驱动的中途适配**——图 revision 只能由结构化外部锚点结果(compile/test/verify)触发
   的确定性规则产生,LLM 的自然语言声明永远不是适配信号;
3. **递归分解(P4)**——`implement` 节点可把自身分解为子工作图,每个子图仍套进代码拥有的
   有界模板,执行树随运行时发现生长。详见配套 spec
   `2026-08-22-recursive-work-graph-design.md`。

对应 v1 设计文档中被 Deferred 的备选方案:「Generate a task-specific graph dynamically」。

## 动态性光谱(定位)

本设计覆盖 L2–L4,并以 L4 为最终形态:

| 层级 | 含义 | 阶段 |
|---|---|---|
| L1 静态图 | 两个硬编码拓扑,选一个执行 | 现状 |
| L2 组合选择 | 图在节点开始时按事实从组合空间选出,节点生命周期内静态 | P1 |
| L3 有界修订 | 图结构执行中途可变,但仅经有限预定义规则、硬上限、锚点触发 | P2 |
| L4 递归生长 | 执行树随运行时发现无界生长;每个单元内部仍是代码拥有的静态图 | P4 |

平面式的"图在单个图内自由生长"被明确否决(见 Alternatives #1)——它与锚点体系存在公理级
张力;L4 的正确形态是递归/分层,而非平面生成。

## Non-negotiable Invariants(继承自 v1)

- 图结构(节点、边、绑定)只能由代码从结构化事实生成;LLM 只能填充封闭集合字段。
- 每次选择/适配都可从持久化的 `GraphAuditEvent` 序列完整重放。
- 图有硬上限(节点数 ≤ 8、适配次数 ≤ 2),超限即 Escalate。
- 所有产物字段继续走 `WorkState` 的 `NodeType` 字段权限契约。

## Scope

四个递增阶段,每阶段独立可发布、可回滚:

### P0 — 模板注册表(纯重构)

把 `select_work_graph()` 中硬编码的节点/边构造提取为 `GraphTemplateRegistry`:

```text
struct GraphTemplate {
    id: &'static str,
    stages: Vec<TemplateStage>,        // 每阶段: 角色 + 允许的入边/出边
    edges: Vec<(&'static str, &'static str)>,
}
```

- `select_work_graph` 行为不变,现有 3 个单测原样通过。
- 无任何行为变更,只为 P1 提供挂载点。

### P1 — 结构化事实维度 + 组合器

扩展 `WorkGraphRequest`(`work_graph_plan.rs`),全部为封闭集合,无自由文本:

| 字段 | 类型 | 作用 |
|---|---|---|
| `task_kind` | `Implementation \| Diagnosis`(现有) | 基础拓扑 |
| `requires_human_review` | `bool`(现有) | 终局 HumanReview 门 |
| `risk` | `Low \| Medium \| High` | High → 强制 splice 审查门;Low → 跳过预置 diagnose 节点 |
| `has_test_infra` | `bool` | false → 无 TestAnchor 阶段(纯 compile+verify 图) |
| `max_specialists` | `u8`(0..=3) | 诊断通道数上限,默认 1 |

新增 `compose_work_graph(request) -> WorkGraphPlan`:

- 从注册表按事实挑选阶段片段拼接,`template_id` 变为组合签名(如
  `impl+no-test+review+risk-high`);
- 组合后强制校验:角色边白名单(`permits_role_edge`)、除声明的 retry 环外无环、节点数上限、
  绑定注册表(`bind_registry`)成功;
- 校验失败返回 `WorkGraphPlanError`,绝不降级为"最接近的合法图"。
- `select_work_graph` 保留为 `compose_work_graph(request)` 的兼容别名。

### P2 — 锚点驱动中途适配(图 revision)

`WorkGraphPlan` 增加 `revision: u32`(默认 1);`WorkState` 增加
`set_selected_work_graph_revision`(仍 `pub(crate)`)。

新增纯函数规则集 `GraphAdaptationRule`,输入只有 `WorkState` 的锚点字段:

| 触发条件(结构化) | 适配动作 |
|---|---|
| 连续 2 次同一 TestAnchor 失败且预算未耗尽 | splice `diagnose` 节点到下一次 implement 之前(把今天隐式的 retry 循环变为显式 plan revision) |
| 预算耗尽 / `BoundaryViolation` | Escalate(现状不变,纳入规则表述) |
| `requires_human_review` 且 verify 成功 | 进入 AwaitHumanReview(现状不变) |

- 每次适配:`revision += 1`,追加 `GraphAuditEvent{kind: Adapted, reason: 结构化枚举}`,
  `capture_current_work_state()` 持久化;
- 适配次数超过 `max_adaptations`(默认 2)→ Escalate;
- `next_step()` 不变——它消费的还是锚点状态;适配只改"下一步允许哪些角色",通过既有的
  `require_plan_edge` 校验生效。

### P3 — begin_node 工具面暴露封闭字段

`node_tools.rs` 的 `begin_node` input schema 增加 P1 新字段(全部封闭集合,同
`task_kind` 现有校验模式:非法值 → 结构化错误,不猜测默认)。LLM 提交事实,代码决定图。

## Alternatives Considered

1. **LLM 自由生成图/边(平面动态图)**。否决——与锚点体系存在公理级张力,论证如下:

   **提案/证据分离公理**:`next_step()` 是纯函数,只消费结构化锚点结果。LLM 可以提案
   (代码、verify_commands),但提案必须活过外部执行的真实结果。终点门验证的是**产物**,
   图拓扑编码的是**过程要求**(何时验证、能否绕门、高风险必过人审)——过程没有 exit code。

   若允许平面动态图,最终产物的验证仍然有效(坏代码照样被锚点拦住),死掉的是门周围
   的一切承诺:

   - **循环论证**:「验证这个拓扑是否适合任务」没有 oracle——良构校验(无环、白名单、
     上限)只验证图的语法,验证不了语义。锚点对代码有效正因为命令就是 oracle。
   - **可重放性断裂**:现在「持久化状态 + 代码」唯一确定后续行为;图若由 LLM 生成,重启
     重规划可能给出不同拓扑,同一份锚点历史无法解释,审计序列从合法性证明降级为日记。
   - **注入爆炸半径**:现在注入最多搞坏一次工具调用;结构可生成意味着注入可改写验证计划
     本身——安全系统的形状变成攻击者可写的。
   - **预算语义失效**:`max_iter` 定义在已知循环上;拓扑未知时预算退化为节点计数。

   **根本张力的最简表述**:生成新拓扑的生成器只有两种——确定性代码(规则集可枚举,组合
   空间再大也是 L3)或 LLM 判断(能产生语义新颖性,但恰是锚点体系存在理由所要不信任的
   信号)。新颖结构 = 语义判断 = 只有 LLM 能提供 = 不可信任的输入。

   **解法是递归,不是平面生成**:体系其实已经信任 LLM 提案(代码 edit 就是提案,合法性来自
   必须活过锚点)。把拓扑提案放进同一道缝——LLM 拥有「分解成什么」,代码拥有「每个单元如何
   被验证」。判断负责提案,锚点负责裁决。见 P4 spec。
2. **GoA 式边加权/概率路由**。否决:不可重放;审计序列无法回答"为什么走这条边"。保留为远期
   Non-goal。
3. **并行实现节点 fan-out**。Deferred:需要子代理并发 + 预算分账,先在 P1 用
   `max_specialists` 表达"容量",执行仍串行。
4. **图间合并/跨节点图**。Non-goal:节点即有界单元,跨节点共享经 `inherit_for_new_turn` 的
   审计与绑定,不需要合并图对象。

## Error Handling and Recovery

- 组合校验失败:节点创建失败并返回结构化错误(不产生半持久化状态);
- 适配规则冲突(同一锚点命中多条):固定优先级序,取最高优先级一条,审计记录被跳过的候选;
- 崩溃恢复:revision 与适配审计随 `WorkState` checkpoint 持久化,重启后 `next_step` 依据
  最新持久化状态路由,无需重放 LLM 历史;
- 渲染层(`render.rs` + `cli/org_graph.rs`)显示 revision 与每次适配原因,人工可核对。

## Testing

- P0:现有 `work_graph_plan.rs` 3 测原样通过(等价重构证明)。
- P1:组合矩阵穷举测试(全部事实组合 × 合法性断言),包括节点数上限触发拒绝、无测试基础设施
  时 TestAnchor 缺席、`bind_registry` 对每个组合成功。
- P2:适配触发测试(第 2 次测试失败 → diagnose revision 出现;第 3 次 → 超限 Escalate);
  checkpoint 恢复后 revision 与审计完整;审计重放 `compose→adapt*` 序列得到相同终态。
- P3:`node_tools` schema 校验测试:非法 `risk`/`max_specialists`(如 4)→ 结构化错误。
- 集成:`node_runtime` 现有 work-graph 集成测试全部保持绿色(兼容别名保证零迁移)。

## Rollout / Migration

1. P0 合并 → 跑全量 `cargo test org_graph`(等价性);
2. P1 合并 → `select_work_graph` 变别名,持久化 plan 增加 `revision`(serde `#[serde(default
   = "one")]` 兼容旧 checkpoint);
3. P2 合并 → 适配规则默认全开,`max_adaptations=2` 可经配置降为 0(即回退纯静态行为);
4. P3 合并 → 更新 `prompts/base.md` 中 begin_node 工具说明。
5. P4 独立排期 → 按 `2026-08-22-recursive-work-graph-design.md` 执行,依赖 P1 的模板注册表。

## Non-goals

- LLM 在单个图内生成节点/边/权重的任何通路(平面生成;LLM 拥有的只有分解提案,见 P4)。
- 并行节点执行与预算分账(P1 仅表达容量)。
- 跨节点图合并、图模板热加载、用户自定义模板 DSL。
