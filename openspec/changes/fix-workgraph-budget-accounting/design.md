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

### 现状

- `node_runtime.rs:311` 在每轮 agent loop 对 `budget.iter_used` 递增（`iter_used.saturating_add(1)`）。
- `work_graph.rs:66 retry_or_escalate` 用 `iter_used < max_iter` 判断是否还能重试。

### 设计

1. **移动计费点**：删除 agent loop 每轮的递增；改为在路由决定消耗一次重试时计费——即 `retry_or_escalate` 路径选择 `Implement` 或 `RootCause` 时，对 `budget.iter_used` +1。
2. **预算语义**：`max_iter` 表示"最多允许几轮 修改→锚点验证 循环"；编译/测试/验证锚点首次失败不额外计费（它触发的重试会计），导航轮次不计费。
3. **兼容性**：
   - 持久化格式不变（`Budget { max_iter, iter_used }` 字段不动），旧 checkpoint 语义近似兼容（旧数据 `iter_used` 偏高只会更早 Escalate，方向安全）。
   - `decompose.rs` 的预算拆分（`remaining = max_iter - iter_used`）语义随新计费点自动成立：拆分出去的是"剩余重试次数"。
4. **测试更新**：`node_runtime.rs` 中依赖"每轮递增"语义的夹具（如 `iter_used == 1` 的断言、以及 3380/3389/3398/3407/3416 行的逐轮递增断言组）改为模拟路由级重试后断言；`audit_*` 组测试中预算耗尽路径改为通过路由重试触发。

### 备选方案（不采用）

- 新增独立 `navigation_iters` 字段与重试预算分开统计：持久化格式变更，收益与 P1 相同但复杂度更高。
- 保留每轮计费、单纯调大 `max_iter`：只延迟问题，不解决计费语义错位。

## 影响面

- `src/exec_session/node_runtime.rs`：计费点移动（P1）；等待语义无涉。
- `src/exec_session/node_tools.rs`：等待期只读放行与 `waiting_on` 字段（P0）。
- `src/exec_session/runtime_store.rs`：若计费移动涉及持久化写入点，同步调整。
- 相关单元测试更新。
