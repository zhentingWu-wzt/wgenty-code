# Comet Design Handoff

- Change: org-graph-shared-state
- Phase: design
- Mode: compact
- Context hash: 7e3cfb40e9e8437c3c81bab1a1232f8601ca378d6688d13ff4080cdaddf2aaa8

Generated-by: comet-handoff.sh

OpenSpec remains the canonical capability spec. This handoff is a deterministic, source-traceable context pack, not an agent-authored summary.

## openspec/changes/org-graph-shared-state/proposal.md

- Source: openspec/changes/org-graph-shared-state/proposal.md
- Lines: 1-58
- SHA256: 5634c3e035d0847cd84698d4171ea3b03a25c904adce7932fa48035e0c19fa05

```md
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

```

## openspec/changes/org-graph-shared-state/design.md

- Source: openspec/changes/org-graph-shared-state/design.md
- Lines: 1-93
- SHA256: 294f89ebacee909fe52a4fe6d4514eac31cd14774b6ccd4a8796ec6f9bdaed2d

[TRUNCATED]

```md
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


```

Full source: openspec/changes/org-graph-shared-state/design.md

## openspec/changes/org-graph-shared-state/tasks.md

- Source: openspec/changes/org-graph-shared-state/tasks.md
- Lines: 1-36
- SHA256: b2f13068f011595f2e2dfd75e78d7c2edbf50f9bc9b6703a87ec37d4c67fc903

```md
# Tasks

## 1. WorkState schema 与模块骨架

- [ ] 1.1 新建 `src/org_graph/work_state.rs`，定义 `WorkState` struct，至少含字段：`requirement` / `generated_diff` / `compile_result`（含 `ok: bool` 与 `stderr`）/ `test_result`（含 `pass: bool` 与 `failed_cases`）/ `human_review`（enum `Approve`/`Reject`）/ `budget`（含 `max_iter` / `iter_used` / `token_used`）/ `step_log`；派生 `Serialize/Deserialize/Clone/Debug`
- [ ] 1.2 在 `src/org_graph/mod.rs` 导出 `pub mod work_state;` 及相关公开类型
- [ ] 1.3 为 schema 加单测：序列化往返（serialize → deserialize 字段等价）、默认值符合预期、各结构化子字段类型不丢失

## 2. 字段级访问权限（真强制）

- [ ] 2.1 设计并定义每个 `NodeType` 的「可读字段集 + 可写字段集」声明矩阵（至少覆盖 5 个内置节点类型的 pilot 相关字段）
- [ ] 2.2 实现 `WorkState` 受权限约束的读写 API：调用方提供 `NodeType`，越权写直接返回 `ContractViolation`（复用 `ContractDimension`，复用 `Permission` 还是新增 `State` 维度见 design 阶段定）
- [ ] 2.3 为权限强制加单测：节点正常读写授权字段成功、越权写字段被拒绝并触发 `ContractViolation`、授权读不写审计日志而授权写记入 `step_log`

## 3. turn 集成与检查点持久化

- [ ] 3.1 把 `WorkState` 锚定到 `exec_session` 的 turn：turn 开始时创建/继承 `WorkState` 实例（turn 间继承策略见 design 阶段定，默认只读字段继承、可写字段按 pilot 语义）
- [ ] 3.2 随 `CheckpointStore` 持久化 `WorkState`：turn 检查点时一并落盘，崩溃后从最近 turn 快照恢复结构化字段
- [ ] 3.3 为 turn 集成加单测：写入结构化字段后崩溃→从检查点恢复→字段值完整；既有文件级 capture/rewind 行为零回归

## 4. pilot 路由点（读结构化字段 + 验证强制）

- [ ] 4.1 查证现有代码是否存在真实的编译闭环或测试闭环路由（编译/测试结果是否真的回流驱动重试），按查证结果与 design D5 硬约束（必须含真实写字段场景）选定 pilot 路由点
- [ ] 4.2 实现 pilot 路由点：从「解析节点自然语言输出」改为「读取 `WorkState` 结构化字段」（如读 `compile_result.ok` 或 `test_result.pass`）做下一跳判定
- [ ] 4.3 确保 pilot 路由点涉及的写字段场景受 2.x 字段级权限强制（节点须声明可写该字段），并提供验证该强制的测试

## 5. 三层状态分层与零回归

- [ ] 5.1 验证 `WorkState` 与 `SessionState`（会话骨架）/ `AppState`（全局配置）三层职责分明：`SessionState` 字段语义不变、`AppState` 完全不动
- [ ] 5.2 `SubagentResultMailbox` 在非 pilot 路径维持原状（与 `WorkState` 并存，不强制同步、不强制淘汰）
- [ ] 5.3 与 `org-graph-dispatch-telemetry` 正交性验证：两者字段/schema 互不依赖、互不修改

## 6. 集成验证

- [ ] 6.1 `cargo build` 通过；`cargo test` 全绿（新增测试通过 + 既有节点契约 / 派发 / 契约强制 / transcript 测试零回归）
- [ ] 6.2 手动验证 pilot 路由点按结构化字段正确路由（成功路径 + 失败回流路径），且越权写字段被拦截

```

## openspec/changes/org-graph-shared-state/specs/org-graph-shared-state/spec.md

- Source: openspec/changes/org-graph-shared-state/specs/org-graph-shared-state/spec.md
- Lines: 1-81
- SHA256: 732a850ef6731fbfe81a5cdcfcd62757ddcebbb52451e9dbe015422ae70cf0a8

[TRUNCATED]

```md
## Purpose

为单个任务实例提供强类型、字段级权限的共享工作状态，让 Org-Graph 节点之间以结构化字段（任务需求 / 代码变更 / 编译结果 / 测试结果 / 预算消耗 / 步骤日志）传递工作产物，取代自由文本消息，并为路由判定与断点续跑提供可读的结构化事实源。

## ADDED Requirements

### Requirement: 强类型共享工作状态

系统 SHALL 为每个任务实例维护一个结构化的共享工作状态（`WorkState`），承载节点之间传递的工作产物。状态 SHALL 至少包含以下结构化字段：任务原始需求、生成的代码变更、编译结果（含成功标志与错误输出）、测试结果（含是否通过与失败用例清单）、人工审核结论、预算消耗、以及步骤审计日志。

#### Scenario: 工作产物以结构化字段流转

- **WHEN** 一个节点完成其工作（例如执行编译或运行测试）并把结果写入共享工作状态
- **THEN** 后续节点能从该状态读取对应的结构化字段（如编译成功标志、测试失败用例清单），而不是解析自然语言文本

#### Scenario: 状态字段强类型可序列化

- **WHEN** 共享工作状态被序列化（用于持久化或检查点）
- **THEN** 序列化结果可被反序列化回等价的结构化状态，字段类型不丢失

### Requirement: 节点对状态字段的访问受权限约束

系统 SHALL 为每种节点类型声明其可读与可写的共享工作状态字段子集。一个节点 SHALL NOT 写入其未声明可写的状态字段。

#### Scenario: 节点越权写字段被拒绝

- **WHEN** 一个节点尝试写入其节点类型未被授权写的状态字段
- **THEN** 系统拒绝该写入并报契约违规（与节点权限边界强制同款机制），状态保持写入前的值

#### Scenario: 节点正常读写授权字段

- **WHEN** 一个节点读写其节点类型声明可读/可写的状态字段
- **THEN** 读写成功完成，且该操作被记入步骤审计日志

### Requirement: 工作状态与既有状态层分层不吞并

系统 SHALL 把共享工作状态与既有的会话状态（会话骨架、turn 链、节点链）和全局应用配置三者职责分层。共享工作状态 SHALL NOT 取代或吞并会话状态或应用配置的既有职责。

#### Scenario: 三层状态互不吞并

- **WHEN** 共享工作状态被引入系统
- **THEN** 会话状态仍承担会话骨架职责（turn 链 / 节点链 / 会话状态），全局应用配置仍承担全局配置职责，共享工作状态只承载 per-task 工作产物
- **AND** 既有的会话状态与应用配置字段语义不发生改变

### Requirement: 工作状态随 turn 检查点持久化可续跑

系统 SHALL 把共享工作状态的生命周期锚定在会话 turn 上，并随既有 turn 检查点机制一并持久化。任务中途崩溃后，系统 SHALL 能从最近 turn 的共享工作状态快照恢复结构化工作产物，而非丢失在自由文本消息中。

#### Scenario: 崩溃后从检查点恢复结构化工作产物

- **WHEN** 一个任务在写入结构化工作状态后崩溃，随后从最近 turn 检查点恢复
- **THEN** 恢复后的共享工作状态包含崩溃前写入的结构化字段（如编译结果、测试失败用例清单）
- **AND** 已捕获的普通文件状态恢复行为不受影响（零回归）

### Requirement: 路由判定读取结构化字段而非解析文本

系统 SHALL 让至少一个真实存在的路由判定（本期 pilot）从「解析节点自然语言输出」改为「读取共享工作状态的结构化字段」。该 pilot 路由点 SHALL 包含一个真实的结构化字段写入场景，使字段级权限强制能被实际验证。

#### Scenario: pilot 路由点读结构化字段做判定

- **WHEN** pilot 路由点需要判定工作状态（例如编译是否通过、测试是否通过）以决定下一跳
- **THEN** 判定读取共享工作状态的结构化布尔字段，而不是解析节点的自然语言文本输出

#### Scenario: pilot 字段写入受权限强制验证

- **WHEN** pilot 路由点涉及的结构化字段被对应节点写入
- **THEN** 该写入受字段级访问权限约束（节点须声明可写该字段），使本期字段级真强制存在可被验证的真实场景

### Requirement: 零回归与正交性

系统 SHALL NOT 改变既有节点契约的五维语义、节点派发行为、契约强制逻辑，也 SHALL NOT 改变既有 transcript store 的 schema。共享工作状态与在途的运行时派发遥测能力 SHALL 保持正交、可并行推进。

#### Scenario: 既有节点契约与派发强制零回归

- **WHEN** 引入共享工作状态后运行既有节点契约、派发与契约强制测试套件
- **THEN** 全部通过，无回归

#### Scenario: 与派发遥测能力正交

- **WHEN** 共享工作状态与运行时派发遥测同时存在于系统

```

Full source: openspec/changes/org-graph-shared-state/specs/org-graph-shared-state/spec.md
