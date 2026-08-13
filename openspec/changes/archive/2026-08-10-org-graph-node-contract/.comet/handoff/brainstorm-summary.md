# Brainstorm Summary

- Change: org-graph-node-contract
- Date: 2026-08-10

## 确认的技术方案

基于 open 阶段 7 个决策（D1-D7）的深度技术细化。核心：NodeContract 作为数据驱动 struct 建模五维，coordinator 在 reserve_child 强制三维校验，task.rs 从硬编码 match 改读契约。

三个 Open Question 已解决（均经代码验证 + 用户确认）：

1. **OQ1 RLM delegate node_type 归属**：统一用 `GeneralPurpose` 契约（默认值）。RLM 的 `SubTask` 无 type 字段（只有 prompt/use_small_model/depends_on），两处 `reserve_child`（pipeline.rs:276 普通 / :650 replan）只传 prompt。**不新建契约类型**。`use_small_model` 是 RLM 专有概念，**不进 NodeContract**，由 RLM 自己读控制模型选择。

2. **OQ2 契约违反回退策略**：**硬拒绝 + 回 ToolError 给父 agent**，与 `DepthLimitReached` 同模式。代码验证安全：`fallback_eligible_from_coordinator_error`（fallback.rs:26）是穷尽 match + `_ => None`，新 `ContractViolation` 变体自动落 `_ => None` 不触发 fallback。replan 路径 `reserve_child` 失败时 `continue`（pipeline.rs:660），不破坏 replan 循环。`task.rs map_coordinator_error` 加 `ContractViolation` 分支映射为 `{ code: "contract_violation", msg }`。

3. **OQ3 IO schema 类型表达**：**轻量 `IoShape` 枚举**（FreeText / StructuredJson / Report）。代码库无 schema 基础设施（无 TypeId/schemars），`ChildResult.summary` 全程 String。IoShape 可 serde 往返，声明不校验，后续 IO 强制 change 可加变体。

## 关键取舍与风险

- **ContractViolation 不触发 fallback**：结构上天然成立（穷尽 match `_ => None`），无需额外逻辑。风险：父 agent 收到 contract_violation 错误后需自行决定处理（改请求/换类型/放弃），模型有此能力（与 DepthLimitReached 同模式）。
- **SpawnChildRequest 多调用点**：reserve_child 有多调用点（task.rs / fallback.rs / rlm/pipeline.rs:276,:650 / daemon/handlers.rs / run_script.rs）。Mitigation：node_type 默认 GeneralPurpose，未显式传的调用点行为不变。
- **filter_allowed_tools 签名变更**：现有单测需同步改。Mitigation：新签名读 contract 更易构造测试用例。
- **AgentDefinition 与 NodeContract 两层并存**：Mitigation：文档标注各自用途，不删 AgentDefinition 避免破坏 CLI runner。
- **IoShape 声明不校验**：Mitigation：字段文档明确"声明态"，验收场景只测三维强制不测 IO。

## 测试策略

- 纯函数单测：NodeContract serde 往返、NodeRegistry 查询、IoShape 枚举往返、coordinator 三维校验（能力越纲/权限边界/budget 拒绝/合法放行）、budget None 回退全局/Some 覆盖、ContractViolation 不触发 fallback（fallback_eligible 返回 None）。
- task.rs 无回归：explore/plan/general-purpose 三种节点派发的 system_prompt + allowed_tools + budget 与变更前硬编码路径完全一致。
- AgentDefinition 并存：CLI run_agent + stress_tests 仍工作。
- 不写端到端 run_rlm_pipeline 单测（需 LLM+coordinator，现有测试也规避）。

## Spec Patch

无（三个 OQ 的解决均属实现细节，OpenSpec delta spec 的需求已涵盖 ContractViolation 拒绝、IoShape 声明不校验、RLM 走 GeneralPurpose 默认值的语义）。
