# Design: Work-Graph 预算计费与诊断等待期修复

## 方案总览

两个相互独立的修复，共用"等待期不惩罚调查行为"这一原则。

## P0：诊断等待期放开只读操作 + 明示等待语义

### 现状

- `verify_node` 返回 `{"status": "root_cause_dispatched", "root_cause_child_id": ...}` 后，节点运行时进入等待态，直到 child 调 `submit_specialist_report` 才解锁。
- 等待期间 node 工具层拦截主 agent 操作：`submit_specialist_report` / `rollback_node` 被拒符合预期，但 `exec_command` 等只读/验证类工具也被拦截（实测报错 `the static Work-Graph is waiting for its root-cause specialist report`）。
- 返回 payload 未说明等待机制，agent 无法区分"后台执行中"与"死锁"。

### 设计

1. **等待语义**：`verify_node` 在派生 root-cause 专家时，响应中增加 `waiting_on` 字段，说明 child id、报告提交方式、结果经 `<child-results>` 送达。例：

   ```json
   {
     "status": "root_cause_dispatched",
     "root_cause_child_id": "fe090ee2-...",
     "waiting_on": "root-cause specialist fe090ee2-... is running in the background; its report arrives via <child-results>. Read-only commands and tests remain available while waiting."
   }
   ```

2. **只读放行**：等待期的拦截逻辑按工具分类：
   - 放行：`exec_command` / `file_read` / `grep` / `glob` / `search` / `lsp` / `codegraph_explore` 等不修改工作区的工具。
   - 继续拦截：文件写入/编辑类工具（防止诊断期间工作区竞争）、`submit_specialist_report`（身份校验已有）、`rollback_node`、`begin_node` 等图状态变更工具。
   - 实现位置：node 工具拦截层（`node_tools.rs` 或产生 `waiting for its root-cause specialist report` 错误的位置），将"等待期一律拒绝"改为"按工具是否只读分流"。

### 备选方案（不采用）

- 等待期完全放开所有工具：破坏诊断期间的工作区一致性保证。
- 加超时自动跳过 RootCause：掩盖子 agent 真实故障，且与 fail-closed 设计冲突。

## P1：`iter_used` 只在重试 Implement 时计费

### 调查结论（修订）

实施时核查代码发现：计费点**已经**是失败锚点驱动——`consume_iteration_budget` 只在 compile/test/verify 锚点失败（`node_runtime.rs` 的 `record_compile_result` / `record_test_result` / `verify_for_work_graph` 失败分支）和整组 unit 失败时调用，agent 轮次本身不计费。本次事故的 Escalate 直接成因是等待期拦截导致修复无法在同一轮验证、两连败耗尽预算，而非"每轮扣预算"。因此 P1 从"移动计费点"修订为"固化语义 + 回归测试"。

### 设计

1. **语义固化**：`consume_iteration_budget` 文档注释明确——`iter_used` 只在锚点失败处计费；一次失败购买一次 Implement/RootCause 重试；导航轮次与成功锚点不消耗。
2. **回归测试**：新增 `successful_anchor_pass_does_not_consume_iteration_budget`——全绿锚点跑完后 `iter_used` 保持 0，防止未来有人把计费点移回每轮递增。既有测试（如 `audit_compile_failure_records_root_cause_route_and_consumed_budget`）已覆盖失败计费方向。

### 备选方案（已否决）

- 移动计费点到路由层：调查后确认现状已符合语义，移动反而引入双计费风险。
- 新增独立 `navigation_iters` 字段：持久化格式变更，无实际收益。

## 影响面

- `src/exec_session/node_runtime.rs`：计费点移动（P1）；等待语义无涉。
- `src/exec_session/node_tools.rs`：等待期只读放行与 `waiting_on` 字段（P0）。
- `src/exec_session/runtime_store.rs`：若计费移动涉及持久化写入点，同步调整。
- 相关单元测试更新。
