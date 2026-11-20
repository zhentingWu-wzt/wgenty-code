# Proposal: 修复 Work-Graph 预算计费与诊断等待期缺陷

## 问题描述

在一次普通的 medium 风险代码修复（Kimi catalog 超时与内置列表匹配）中，Work-Graph 节点在**代码已全部修复、2066 个 lib 测试与 clippy 全绿**的情况下被路由到 `Escalate` 终态，节点被标记 Failed。审计记录与工作区实况失真。

实际链路暴露了两个流程缺陷：

1. **诊断等待期全锁且无提示**：`verify_node` 进入 `root_cause_dispatched` 后，派生的 root-cause 专家在后台异步运行，但等待期间主 agent 的工具调用（包括只读的 `exec_command` 跑测试）被拦截，返回信息也未说明"子 agent 正在后台执行、结果将通过 `<child-results>` 送达"。主 agent 误判为死锁，绕开图手动完成验证。
2. **导航轮次与修复重试共享同一预算**：`budget.iter_used` 在 `node_runtime.rs:311` 按 agent 轮次递增，读文件、跑测试、查状态等无风险导航操作与"再来一次实现尝试"消耗同一额度。等待子 agent 期间的大量调查轮次烧光了 `max_iter`，待报告送达、验证重新可走时，`retry_or_escalate`（`work_graph.rs:66`）发现预算耗尽，不看工作区实况直接判 Escalate。

## 根因分析

- **等待期策略过严**：`root_cause_dispatched` 为防止主 agent 在专家诊断期间改代码造成状态竞争，拦截了所有非只读工具——包括 `exec_command`/`execute_command`/`run_test` 等验证类命令工具。等待期间主 agent 无法跑测试验证修复，也收不到"子 agent 在后台执行"的语义提示，误判死锁后绕开图手动验证；期间产生的失败锚点继续消耗 `iter_used`，最终两连败触发 Escalate。
- **计费点（已排除）**：调查初期怀疑 `iter_used` 按 agent 轮次计费；实施时核查确认计费点已是失败锚点驱动（仅 compile/test/verify 失败与整组 unit 失败时计费），与本次事故无因果关系。该语义此前只有隐式约定，本 change 将其文档化并加回归测试。

## 修复目标

- **P0**：`root_cause_dispatched` 等待期放行只读工具与验证类命令工具（exec_command/execute_command/run_test 等）；`verify_node` 返回的 `root_cause_dispatched` 响应携带等待语义（在等待哪个 child、结果如何送达、哪些工具仍可用）。
- **P1**：固化 `iter_used` 的失败锚点计费语义（文档注释）并补回归测试，防止计费点被移回按轮递增。

## 非目标

- 不调整 `hooks.rs` 内层 verify-gate 的 `AutoRetry{max:2}` 默认值（与本事故无关）。
- 不实现"预算耗尽后先实况检查再判 Escalate"（P2）与按 risk 分层预算（P3），留待后续独立评估。
- 不改变 Escalate/Failed 终态语义与审计事件格式。
