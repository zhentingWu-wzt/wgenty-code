## 1. NodeContract 类型基础

- [x] 1.1 新建 `src/org_graph/mod.rs` 模块，定义 `NodeType` 枚举（Explore / Plan / GeneralPurpose / Verification / WgentyCodeGuide）、`Capability`、`PermissionBoundary`、`ResourceBudget`、`IoSchema` 类型；定义 `NodeContract` struct 含五维字段，全部派生 `Serialize/Deserialize/Debug/Clone`
- [x] 1.2 在 `src/main.rs`（或 lib root）注册 `org_graph` 模块
- [x] 1.3 添加 `NodeContract` 序列化往返测试（五维字段齐全，序列化->反序列化断言相等）

## 2. NodeRegistry 与内置契约

- [x] 2.1 在 `src/org_graph/registry.rs` 实现 `NodeRegistry`（持有 5 个内置 `NodeContract`，`get(&NodeType) -> Option<&NodeContract>`）
- [x] 2.2 填充 5 个内置节点契约，capabilities/permissions/budget 照搬现有硬编码语义（explore/plan=leaf can_spawn=false；general-purpose 可 spawn；explore_readonly 作 can_mutate_fs 默认源；IO schema 声明态留占位）
- [x] 2.3 添加测试：5 个内置契约均可在 registry 查到；查询不存在类型返回 None；契约内容与现有硬编码语义对齐

## 3. SpawnChildRequest 扩展

- [x] 3.1 给 `SpawnChildRequest`（`agent/coordinator.rs`）加 `node_type: NodeType` 字段，带默认值 `GeneralPurpose`；`SpawnChildRequest::new` 签名向后兼容
- [x] 3.2 给 `AgentCoordinator` 加 `NodeRegistry` 引用（构造时注入或内部默认持有）
- [x] 3.3 测试：显式传 node_type 与默认值两种构造路径

## 4. coordinator 三维强制校验

- [x] 4.1 新增 `CoordinatorError::ContractViolation` 变体（携带维度+原因），与 `DepthLimitReached` 等 structural 错误区分
- [x] 4.2 在 `reserve_child` 加三维校验：能力（requested tools ⊆ capabilities）、权限边界（can_spawn/can_mutate_fs/can_exec）、资源预算（leaf 禁 spawn + per-node-type depth/concurrent/token 覆盖）；违反返回 `ContractViolation`
- [x] 4.3 衔接 `SubagentLimits` 作为 budget 全局默认：contract.budget 字段为 None 时回退全局值，Some 时覆盖
- [x] 4.4 测试：能力越纲拒绝、权限边界拒绝、budget 拒绝（leaf 禁 spawn）、合法派发放行、budget None 回退全局、budget Some 覆盖全局

## 5. task.rs 读契约

- [ ] 5.1 改 `execute_with_context`：从硬编码 `match _subagent_type` 分支改为 `NodeRegistry::get(&node_type)` 读 `NodeContract`；system_prompt / allowed_tools / budget 全来自契约
- [ ] 5.2 改 `filter_allowed_tools` 签名从 `(names, subagent_type, depth, max_depth, explore_readonly)` 改为读 `&NodeContract`；内部读 `permissions` + `capabilities`；`explore_readonly` 作 `can_mutate_fs` 全局默认源
- [ ] 5.3 模型 JSON 的 `subagent_type` 字符串经派发层映射为可信 `NodeType` 枚举（不直接注入 SpawnChildRequest）
- [ ] 5.4 测试：explore/plan/general-purpose 三种节点派发的 system_prompt + allowed_tools + budget 与变更前硬编码路径完全一致（无回归）

## 6. 其余调用点补 node_type

- [ ] 6.1 排查并更新 `reserve_child` / `reserve_child_in_group` 所有调用点（fallback.rs / rlm/pipeline.rs / daemon/handlers.rs / run_script.rs），补 node_type 或依赖默认值
- [ ] 6.2 确认 delegate（RLM）路径的 node_type 归属（Open Question 1：倾向统一 general-purpose），落实并在 design 阶段记录决策
- [ ] 6.3 确认契约违反回退策略（Open Question 2：倾向硬拒绝不触发 fallback），确认 fallback.rs 只认 structural 失败不认 ContractViolation

## 7. AgentDefinition 并存验证

- [ ] 7.1 确认 `AgentDefinition`/`AgentsService` 未被新派发路径引用（新路径只读 NodeContract）
- [ ] 7.2 测试：CLI `run_agent` 仍走 AgentsService 旧路径，行为不变；stress_tests 仍工作

## 8. 验证与收尾

- [ ] 8.1 `cargo fmt --check` + `cargo clippy --all-targets -- -D warnings` 零 warning
- [ ] 8.2 `cargo test -p wgenty-code` 全量通过（含 org_graph 模块测试、coordinator 校验测试、task.rs 无回归测试、现有 subagent 测试无回归）
- [ ] 8.3 手动：真实 `task`/`delegate` 派发 explore 与 general-purpose 节点，确认 system_prompt/工具集/budget 来自契约且行为与变更前一致
