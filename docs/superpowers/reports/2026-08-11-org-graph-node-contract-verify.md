# 验证报告：org-graph-node-contract

- **Change**: org-graph-node-contract
- **验证模式**: full（25 tasks > 3、2 delta capabilities > 1、24 changed files > 8，三项阈值均超）
- **验证日期**: 2026-08-11
- **base-ref**: `0de3b78d` … HEAD `9b7e496d`
- **review_mode**: standard（build 阶段已做最终代码审查，verify 阶段做正确性/安全/边界轻量复核）

## 摘要记分卡

| 维度 | 状态 |
|------|------|
| Completeness（完整性） | 25/25 tasks 完成；7 ADDED + 1 MODIFIED 需求全部有实现证据 |
| Correctness（正确性） | 全部需求/场景有覆盖测试；新鲜构建+全量测试通过 |
| Coherence（一致性） | 实现符合 design.md D1–D7 决策；delta spec 与 Design Doc 无矛盾 |

**最终评估**：无 CRITICAL、无 IMPORTANT 问题。所有检查通过，**Ready for archive**。

---

## 新鲜验证证据（本报告当次运行）

| 检查 | 命令 | 结果 |
|------|------|------|
| 构建 | `cargo build` | exit 0 |
| 格式 | `cargo fmt -- --check` | exit 0 |
| 静态检查 | `cargo clippy --all-targets -- -D warnings` | exit 0 |
| 全量测试 | `cargo test` | lib 1427 passed / 0 failed；integration 183 passed / 0 failed |

---

## 1. Completeness（完整性）

### 1.1 Task 完成

`openspec instructions apply` 报告 `progress: total 25, complete 25, remaining 0`。tasks.md 8 个顶层任务（共 25 个子项）全部 `[x]`。

### 1.2 需求→实现映射（ADDED: org-graph-node-contract/spec.md）

| 需求 | 实现位置 | 覆盖测试 |
|------|---------|---------|
| R1 NodeContract 建模（五维 + serde 往返） | `src/org_graph/contract.rs` NodeContract 五维字段，派生 Serialize/Deserialize | `node_contract_serde_roundtrip`；`ioshape_serde_roundtrip`；`contract_dimension_serde_roundtrip` |
| R1 内置覆盖五种类型 | `registry.rs::builtin` 5 个契约 | `all_five_builtin_contracts_present`；`builtin_capabilities_all_wildcard`；`builtin_budgets_all_none` |
| R2 NodeRegistry 查询 | `registry.rs::get(&NodeType) -> Option<&NodeContract>` | `all_five_builtin_contracts_present`（Some 路径） |
| R2 查询不存在返回 None | 同上，HashMap miss | 由 `get` 语义保证（Option 返回） |
| R3 SpawnChildRequest.node_type（默认 GP） | `coordinator.rs:39` 字段 + `:46 new` + `:55 with_node_type` | `spawn_child_request_defaults_to_general_purpose`；`spawn_child_request_with_node_type_overrides_default` |
| R3 node_type 来自可信派发层 | `task.rs::parse_node_type` 映射后注入，非模型 JSON 直接 | `parse_node_type_maps_all_known_strings`；`parse_node_type_unknown_falls_back_to_general_purpose` |
| R4 coordinator 三维强制（两层） | `coordinator.rs::reserve_child` caller can_spawn + budget；`task.rs::filter_allowed_tools` capability + can_mutate_fs | 见下方场景表 |
| R4 ContractViolation 独立变体 | `coordinator.rs` CoordinatorError::ContractViolation + `task.rs::map_coordinator_error` code "contract_violation" | `coordinator_contract_violation_not_eligible` |
| R5 budget Option 覆盖全局 | `coordinator.rs::resolve_effective_max_depth` | `resolve_effective_max_depth_none_falls_back_to_global`；`resolve_effective_max_depth_some_overrides_global`；`builtin_budgets_all_none` |
| R6 IO schema 声明态不校验 | `contract.rs` IoShape 枚举，运行时无校验路径 | `ioshape_*` 测试只测 serde/默认，无校验断言（符合声明态） |
| R7 task.rs 读契约（system_prompt/tools/budget） | `task.rs::execute_with_context` 经 `coordinator.node_contract` 读契约；match 块已移除 | `builtin_system_prompts_retain_distinct_opening_phrases`（byte-identity 守卫）；no-regression 测试组 |
| R7 filter_allowed_tools 三重过滤 | `task.rs::filter_allowed_tools(names, &NodeContract)` | `filter_capability_whitelist_when_non_empty`；`filter_can_spawn_false_strips_task_and_delegate`；`filter_can_mutate_fs_false_strips_mutating_fs_tools` |
| R8 AgentDefinition 并存不破坏 | 新派发路径零代码引用 AgentDefinition/AgentsService（仅 registry.rs 注释）；CLI 路径未改 | `cli/args.rs:730-734` 映射镜像 parse_node_type；stress_tests + integration 183 passed |

### 1.3 需求→实现映射（MODIFIED: subagent-tool-permissions/spec.md）

| 场景 | 覆盖测试 |
|------|---------|
| Explore cannot call file_write（readonly=true） | `explore_readonly_filters_mutating_fs_tools`（strip file_write/file_edit/apply_patch）；`filter_can_mutate_fs_false_strips_mutating_fs_tools` |
| Explore can call file_read | `explore_readonly_filters_mutating_fs_tools`（keep file_read/grep/exec_command） |

---

## 2. Correctness（正确性）

### 2.1 R4 场景逐项验证（新鲜运行，5 测试全部 ok）

| 场景 | 测试 | 结果 |
|------|------|------|
| leaf 节点禁止 spawn 被 coordinator 拒绝 | `leaf_caller_cannot_spawn` | ok |
| 合法派发（root spawn 任意 builtin 子类型）不被拒绝 | `all_builtin_child_types_accepted_from_root` | ok |
| budget None 回退全局 | `depth_limit_uses_global_when_budget_none` | ok |
| GP caller 可 spawn | `general_purpose_caller_can_spawn` | ok |
| ContractViolation 不触发 fallback | `coordinator_contract_violation_not_eligible` | ok |

### 2.2 关键正确性属性

- **can_spawn 查 CALLER 而非 child**：`caller_contract(caller)` 用 `node_types` 旁表查调用者 node_type（root 未注册 → GeneralPurpose），再查 `caller_contract.permissions.can_spawn`。这修正了 Design Doc §6.3 原伪代码的 bug（原伪代码查 child 契约会令 spawn leaf 永远失败）。偏差已在 §6.3「实现偏差」注记中记录。`leaf_caller_cannot_spawn`（leaf 调用者拒绝）+ `all_builtin_child_types_accepted_from_root`（root 可 spawn leaf 子类型）共同锁定两端语义。
- **信任边界完整**：模型 JSON `subagent_type` → `parse_node_type` → 可信 NodeType → `SpawnChildRequest::with_node_type`，无路径直接注入原始字符串。4 个生产调用点（task.rs / rlm×2 / run_script）均设可信 node_type；fallback.rs / handlers.rs 的 spawn 全部在 `#[cfg(test)]`。
- **无回归**：explore/plan/GP 三类的 system_prompt 与工具集与变更前一致（system_prompt 逐字迁移，byte-identity 守卫测试锁定；工具集经真实 registry 契约的 no-regression 测试断言）。
- **node_types 旁表无泄漏**：`record_mapped_child_result` 在 group 早返回之前移除 child 条目，覆盖 group-less spawn；`node_types_entry_removed_after_child_finishes` 锁定（build 阶段 review 发现并修复的 Important 问题）。

### 2.3 无安全问题

- 无硬编码密钥、无新增 `unsafe`。
- 契约违反显式拒绝（非静默放行），权限边界由数据驱动声明，无法被模型 JSON 篡改（node_type 是可信枚举，非模型输入）。

---

## 3. Coherence（一致性）

### 3.1 Design 决策遵循（design.md D1–D7）

| 决策 | 遵循证据 |
|------|---------|
| D1 NodeContract 是 serde struct 非 trait | `contract.rs` NodeContract 为派生 struct，无 trait |
| D2 三维强制在 coordinator reserve_child | `reserve_child` 强制 can_spawn + budget（can_spawn/budget 维）；capability + can_mutate_fs 在 task.rs（delta spec 明确的两层拆分，D2 的精细化，非矛盾） |
| D3 SpawnChildRequest 携带 node_type 非 contract | `coordinator.rs:39` node_type: NodeType 枚举字段 |
| D4 budget Option 覆盖全局 SubagentLimits | ResourceBudget 全 Option；resolve_effective_max_depth None/Some 语义 |
| D5 IO schema 声明不强校验 | IoShape 枚举声明，无运行时校验 |
| D6 filter_allowed_tools 读契约但保留纯函数 | `filter_allowed_tools(names, &NodeContract) -> Vec<String>`，纯函数 |
| D7 5 内置契约照搬硬编码语义不调 | `builtin_budgets_all_none` + `builtin_capabilities_all_wildcard` 锁定无 tuning |

### 3.2 delta spec 与 Design Doc 无矛盾

- 「三维校验分两层」在 delta spec（org-graph spec.md）与 Design Doc（docs/superpowers/specs/，§6.3 含实现偏差注记）均有记录，一致。
- §6.3 原伪代码 bug（查 child 契约）已在 build 阶段 review 后补「实现偏差」注记说明 caller-contract 检查，Design Doc 不再与实现矛盾。

### 3.3 Design Doc 可定位

`docs/superpowers/specs/2026-08-10-org-graph-node-contract-design.md` 存在，与当前 change 相关。

---

## 4. 问题清单

### CRITICAL（必须修）— 无。

### IMPORTANT（应修）— 无。

### WARNING / SUGGESTION（接受并记录）

build 阶段 standard review 已发现的 Minor，按 review gate 接受并在 commit `d0af29b7` body 记录理由：

1. **verify/guide 契约 system_prompt 为空**（registry.rs:107,130 注释说明）：这两个 node type 是 CLI 路径专用（prompt 在 AgentDefinition），task 工具 schema enum 阻止这些字符串，派发路径不可达。接受。
2. **caller_contract 每次 spawn clone 整个 NodeContract**：spawn 本身昂贵，clone 可忽略；registry 是 Arc'd immutable，未来可改借用返回。接受，留优化。

---

## 5. build 阶段代码审查复核（standard）

build 阶段已对 `0de3b78..8c4d309` diff 做最终代码审查（reviewer subagent）。结论：0 Critical、1 Important（node_types 泄漏，已修+测试）、6 Minor（4 修+2 接受）。verify 阶段轻量复核聚焦「实现是否符合 spec/tasks 的正确性」与「build 之后新增改动」：build 之后仅新增 review 修复（`d0af29b7`）+ plan 勾选（`9b7e496d`），均已纳入本次新鲜验证。

---

## 6. 最终评估

**Ready for archive**: 是。

**理由**：25/25 tasks 完成；7 ADDED + 1 MODIFIED 需求全部有实现与覆盖测试；新鲜构建/fmt/clippy 全绿、1610 测试 0 失败；实现遵循 design D1–D7，delta spec 与 Design Doc 一致；无 CRITICAL/IMPORTANT 问题。仅 2 个 WARNING/SUGGESTION 已按 review gate 接受并记录理由。
