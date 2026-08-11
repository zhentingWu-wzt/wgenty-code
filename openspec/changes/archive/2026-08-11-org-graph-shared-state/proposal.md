# Org-Graph Shared-State（共享工作状态）

> **路线定位**：Org-Graph 多层演进的**第三步**。
> 第一步 `org-graph-node-contract`（已归档）落地了静态能力契约层；第二步 `org-graph-contract-viewer`（已归档）落地了只读渲染；姊妹 change `org-graph-dispatch-telemetry`（在途）落地运行时派发遥测。本 change 引入**强类型共享工作状态**，是节点之间结构化传递工作产物的契约层，闭合 Rohit Graph-Engineering 框架中「Shared-State」这一核心原语。

## Why

Org-Graph 的节点池（`NodeRegistry`）和五维契约（`NodeContract`）已经落地并在运行时强制（`coordinator.rs:547` 的 `can_spawn` / `task.rs:1097` 的 `filter_allowed_tools`）。但**节点之间传递工作产物时，没有任何强类型、字段级权限的共享状态**——

当前数据流是自由文本的：父 agent 把 prompt 写给子 agent → 子 agent 跑自己的 LLM loop → 结果以 `content: String` 形式写进 `SubagentResultMailbox`（`subagent_mailbox.rs:187`）→ 父 agent 读回自由文本字符串，靠模型自己解析。编译输出、测试结果、diff 这些**本应结构化**的工作产物，全部以自然语言字符串在 agent 之间流动。

这恰好命中 Rohit Graph-Engineering 框架指出的三大生产环境病症之首——**状态漂移**：

- 每个子 agent 维护自己的私有上下文，各 agent 看到的信息不一致；
- 没有统一、可校验的共享状态字段；
- 路由判断无法读取结构化布尔（如 `state.test_result.pass`），只能解析大模型自然语言文本。

后果：边的路由判定权实质上交给了模型自由决策（违背 Rohit「路由优先交给代码」原则），且无法做断点续跑——中途崩溃后，结构化工作产物（编译是否通过、哪些测试失败）已丢失在自由文本里，无法恢复。`exec_session` 已有文件级 `CheckpointStore`（`checkpoint_store.rs`）能捕获工作区文件，但它捕获的是**文件**，不是**工作状态字段**——编译结果、测试失败清单这些结构化产物在 checkpoint 里无对应。

**本 change 引入强类型 `WorkState`**：把任务需求、diff、编译结果、测试结果、预算消耗、步骤日志等结构化为一张有字段权限的共享 schema，节点按声明的读/写权限访问字段，路由判定读取结构化字段而非解析文本。这是后续 Edge 一等公民、External-Anchor 治理的前置依赖——没有它，Edge 没东西可读，Anchor 没地方可挂。

## What Changes

- **新增 `WorkState` 强类型 schema**（`src/org_graph/work_state.rs`）：承载单个任务实例的结构化工作状态。字段至少包含：
  - `requirement: String` —— 任务原始需求（任务开始时写入，只读）
  - `generated_diff: Option<String>` —— 代码生成节点写入，校验节点读取
  - `compile_result: Option<CompileResult>` —— `{ ok: bool, stderr: String }`，编译节点写入
  - `test_result: Option<TestResult>` —— `{ pass: bool, failed_cases: Vec<String> }`，测试节点写入
  - `human_review: Option<ReviewDecision>` —— `enum { Approve, Reject }`，人工审批节点写入
  - `budget: BudgetUsage` —— `{ max_iter, iter_used, token_used }`，由边代码强制截断
  - `step_log: Vec<StepRecord>` —— 审计轨迹（谁在何时读写哪个字段）
- **字段级访问权限**：每个 `NodeType` 在契约里声明可读/可写的 `WorkState` 字段子集（如代码生成节点可写 `generated_diff`、只读 `requirement`；校验节点可写 `compile_result`/`test_result`、只读 `generated_diff`）。节点越权写字段时拒绝（`ContractViolation` 同款机制）。
- **`WorkState` 生命周期锚定在 `exec_session` 的 turn 上**：复用现有 turn 生命周期与 `CheckpointStore` 文件级 capture/rewind（`checkpoint_store.rs`），天然获得断点续跑——中途崩溃可从最近 turn 的 `WorkState` 快照恢复，不丢结构化工作产物。`WorkState` 是 per-task 工作产物 schema，`SessionState` 保持会话骨架职责（turn 链 / node 链 / status），两者分层不互相吞并。
- **字段级访问权限本期真强制**：每个 `NodeType` 声明可读/可写的 `WorkState` 字段子集，**越权写字段直接拒绝并触发 `ContractViolation`**（同款机制，与 `reserve_child` 的 `can_spawn` 强制对齐）。本期不做「声明 + warning」软路径——要么真拦截，要么不声明该字段。
- **路由判定从文本改为读字段**：本期聚焦 schema 与读写 API 落地 + **一个 pilot 路由点**改读字段；大规模迁移留后续 change。
- **不改变 dispatch 行为与契约强制**：纯新增只读+结构化层，不改 `NodeContract` 五维语义、不改 `reserve_child` 强制逻辑、不改 transcript store schema（与 dispatch-telemetry 正交）。

## Capabilities

### New Capabilities

- `org-graph-shared-state`: 为单个任务实例提供强类型、字段级权限的共享工作状态 `WorkState`，让节点之间以结构化字段（diff / 编译结果 / 测试结果 / 预算消耗 / 步骤日志）传递工作产物，取代自由文本消息，并为路由判定与断点续跑提供可读的结构化事实源。

### Modified Capabilities

<!-- 无。本 change 纯新增只读+结构化层；不改 org-graph-node-contract 的五维契约语义，不与 org-graph-dispatch-telemetry 的 transcript schema 迁移重叠。字段级权限复用现有 ContractViolation 机制，不修改既有需求。 -->

## Impact

- **新增代码**：`src/org_graph/work_state.rs`（schema + 字段权限 API + 审计日志）；`NodeContract` 可能新增一个声明态的「字段权限」字段（只声明、本期不强制，强制留后续 change）。
- **与现有状态分层**：`WorkState`（per-task 工作产物）与 `exec_session::SessionState`（会话骨架）/ `AppState`（全局配置）三者职责正交，互不吞并。
- **不改**：`reserve_child` 契约强制、`filter_allowed_tools`、transcript store schema、dispatch 路径。与 `org-graph-dispatch-telemetry` 正交（telemetry 观测「过去哪些 run 是什么节点」，WorkState 回答「任务进行中节点间如何结构化传递产物」），可并行推进。
- **回归风险**：低——纯新增结构化层；自由文本 mailbox 在本期保留为兜底（WorkState 与 mailbox 并存，pilot 路由点改读字段，其余维持原状），不强制一次性迁移。
- **依赖**：`NodeContract` / `NodeType` 已派生 `Serialize/Deserialize`；`WorkState` 同样派生 serde 以支持未来 Checkpoint 持久化。无新外部 crate。
- **Open Questions（留 design 阶段定）**：
  - ~~`WorkState` 的生命周期锚点~~ → **已定：锚定在 `exec_session` turn 上**，复用 CheckpointStore 续跑。
  - ~~字段级权限强制时机~~ → **已定：本期声明 + 拒绝越权写**（真强制，触发 `ContractViolation`）。
  - 哪个路由点作为 pilot 从「解析文本」迁移到「读字段」？**候选**：编译失败→回到代码生成 / 测试失败→回到代码生成。design 阶段需先核查现有代码是否存在真实路由闭环（编译/测试结果是否真的回流驱动重试），再选定 pilot。**约束**：pilot 必须能在 Q2 的字段级真强制下被验证——即 pilot 路由点必须包含一个真实「写字段」场景（如编译节点写 `compile_result`），否则 Q2 的强制无场景可拦、形同虚设。
