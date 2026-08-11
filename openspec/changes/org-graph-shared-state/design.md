## Context

Org-Graph 的静态能力契约层（`NodeRegistry` + 五维 `NodeContract`，已归档 `org-graph-node-contract`）已在运行时强制（`coordinator.rs:547` 的 `can_spawn`、`task.rs:1097` 的 `filter_allowed_tools`）。但节点之间传递工作产物时只有自由文本：父 agent 发 prompt → 子 agent 跑 LLM loop → 结果以 `content: String` 写进 `SubagentResultMailbox`（`subagent_mailbox.rs:187`）→ 父 agent 读回自由文本靠模型解析。编译结果、测试失败清单、diff 这些本应结构化的产物全埋在自然语言里。详见 `proposal.md - Why`。

现有可复用的基础设施：
- `exec_session` 的 turn 生命周期 + `CheckpointStore`（`checkpoint_store.rs`）文件级 capture/rewind——能捕获工作区文件，但不捕获「工作状态字段」。
- `ContractViolation` 机制（`ContractDimension` + `CoordinatorError::ContractViolation`）——字段级权限强制可直接复用同款报错路径。
- `NodeContract` / `NodeType` 已派生 `Serialize/Deserialize`。

约束：与在途的 `org-graph-dispatch-telemetry`（transcript schema 加列）正交，不改其 schema；不改节点契约五维语义；本期只 pilot 一个路由点。

## Goals / Non-Goals

**Goals:**

- 引入强类型 `WorkState` schema，承载 per-task 结构化工作产物（需求 / diff / 编译结果 / 测试结果 / 审核结论 / 预算消耗 / 步骤日志）。
- 节点按 `NodeType` 声明可读/可写的字段子集，越权写触发 `ContractViolation`（真强制，非 warning）。
- `WorkState` 锚定在 `exec_session` turn 上，随 CheckpointStore 续跑。
- 至少一个真实路由点（pilot）从解析文本改为读结构化字段，且该 pilot 必须含真实写字段场景以验证字段级强制。
- `WorkState` 与 `SessionState`（会话骨架）/ `AppState`（全局配置）三层职责分明，互不吞并。

**Non-Goals:**

- 不大规模迁移所有现有自由文本路由点——只 pilot 一个。
- 不引入独立 task 实例抽象（`WorkState` 锚在 turn，不新建 task 对象）。
- 不实现 Edge 一等公民、External-Anchor 节点——这些是后续 change（Shared-State 是它们的前置依赖）。
- 不改 `NodeContract` 五维语义、不改 dispatch 路径、不改 transcript store schema。
- 不强制一次性淘汰 `SubagentResultMailbox`——本期两者并存，mailbox 作为非 pilot 路径的兜底。

## Decisions

### D1：`WorkState` 是 per-task 结构化 schema，置于 `src/org_graph/work_state.rs`

纯数据 struct + 字段权限读写 API，与 `org_graph` 模块「纯数据 + 纯函数」风格一致。字段至少：`requirement`、`generated_diff`、`compile_result`、`test_result`、`human_review`、`budget`、`step_log`。派生 `Serialize/Deserialize` 支持 Checkpoint 持久化。

**备选**：把 `WorkState` 放进 `exec_session`。**否决**：`WorkState` 是 Org-Graph 子系统的契约（节点间工作产物），`exec_session` 是会话骨架；放 `org_graph` 保持子系统内聚，`exec_session` 只负责 turn 集成。

### D2：字段级权限用声明矩阵 + 复用 `ContractViolation` 强制

每个 `NodeType` 声明一个「可读字段集 + 可写字段集」。读写 API 在执行前校验调用节点的 `NodeType` 是否被授权；越权写直接返回 `ContractViolation { dimension: Permission }`（复用现有 `ContractDimension::Permission`，或按需新增 `State` 维度，design 阶段末尾定）。

**关键**：本期真强制，不做「声明 + warning」软路径——否则「可校验」承诺打折扣，违背引入 Shared-State 的初衷（治状态漂移）。

**备选 A**：用 Rust 类型系统（每个字段一个 wrapper type，编译期保证权限）。**否决**：节点类型是运行时值（模型输出映射），编译期无法知道调用方 NodeType。
**备选 B**：声明 + warning。**否决**：见上，形同虚设。

### D3：`WorkState` 生命周期锚定在 `exec_session` turn

每个 turn 持有自己的 `WorkState` 实例。turn 开始时创建（或从上一 turn 继承——design 阶段定继承策略），turn 结束/检查点时随 CheckpointStore 一并持久化。崩溃后从最近 turn 恢复，结构化字段不丢。

**理由**：复用现有 turn 生命周期 + CheckpointStore，零新基础设施；天然获得断点续跑；`WorkState` 与 `SessionState` 同属一个 turn 但职责正交（一个是工作产物，一个是会话骨架）。

**备选**：独立 task 实例对象。**否决**：需新引入 task 抽象层，工作量大，且 turn 已是合适的任务边界锚点。

### D4：三层状态职责分明

| 层 | 类型 | 职责 | 生命周期 |
|----|------|------|----------|
| 全局配置 | `AppState` | settings / daemon / 全局 | 进程级 |
| 会话骨架 | `exec_session::SessionState` | turn 链 / node 链 / 会话 status | 会话级 |
| 工作产物 | `org_graph::WorkState` | per-task 结构化工作字段 | turn 级（本期） |

`WorkState` 引入后，`SessionState` 不丢字段、不改语义；`AppState` 完全不动。

### D5：pilot 路由点 design 阶段定，但必须满足「真实写字段」约束

候选：编译失败→回到代码生成（读 `compile_result.ok`）/ 测试失败→回到代码生成（读 `test_result.pass`）。design 阶段需先核查现有代码是否真有这两个路由闭环（编译/测试结果是否真的回流驱动重试），再选定。

**硬约束**：pilot 必须包含真实写字段场景（如编译节点写 `compile_result`），否则 D2 的字段级强制无场景可拦、形同虚设。pilot 选定与 D2 强制互相验证。

## Risks / Trade-offs

- **[Risk] mailbox 与 WorkState 并存的过渡期数据双写** → 两者语义不同（自由文本 vs 结构化），pilot 路径只走 WorkState，非 pilot 路径维持 mailbox；不强制同步，避免回归。
- **[Risk] 字段级真强制可能打散现有自由文本写路径** → 强制只作用于 `WorkState` 字段写入 API；现有 mailbox 写 `content: String` 不经过该 API，不受影响。
- **[Risk] turn 级 `WorkState` 继承策略影响续跑正确性** → 若 turn N 继承 turn N-1 的状态，需明确哪些字段继承、哪些重置；design 阶段定，默认保守（只读字段继承，可写字段按 pilot 语义）。
- **[Trade-off] 真强制 vs 渐进迁移** → 选真强制换「可校验」承诺，代价是 pilot 必须精心选以提供强制验证场景；接受。
- **[Trade-off] 锚 turn 而非独立 task 实例** → 换零新基础设施 + 天然续跑，代价是未来若需跨 turn 的 task 实例语义要再抽象；本期可接受。

## Migration Plan

纯新增结构化层，无数据/配置迁移：
1. 落地 `WorkState` schema + 字段权限读写 API（D1/D2）。
2. 集成到 `exec_session` turn + CheckpointStore 持久化（D3）。
3. 选定并实现 pilot 路由点（D5），验证字段级强制（D2）。
4. mailbox 维持原状作为非 pilot 路径兜底；后续 change 逐步迁移。

旧版本用户不受影响；`WorkState` 不存在时（legacy turn）按空状态处理，向后兼容。

## Open Questions

- pilot 路由点最终选编译闭环还是测试闭环？（design 阶段先查现有路由是否真实存在再定，受 D5 硬约束）
- 字段级权限的 `ContractDimension`：复用 `Permission` 还是新增 `State` 维度？
- turn 间 `WorkState` 继承策略（哪些字段继承、哪些重置）？
