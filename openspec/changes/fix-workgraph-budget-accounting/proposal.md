# Proposal: 修复 Work-Graph 预算计费与诊断等待期缺陷

## 问题描述

在一次普通的 medium 风险代码修复（Kimi catalog 超时与内置列表匹配）中，Work-Graph 节点在**代码已全部修复、2066 个 lib 测试与 clippy 全绿**的情况下被路由到 `Escalate` 终态，节点被标记 Failed。审计记录与工作区实况失真。

实际链路暴露了两个流程缺陷：

1. **诊断等待期全锁且无提示**：`verify_node` 进入 `root_cause_dispatched` 后，派生的 root-cause 专家在后台异步运行，但等待期间主 agent 的工具调用（包括只读的 `exec_command` 跑测试）被拦截，返回信息也未说明"子 agent 正在后台执行、结果将通过 `<child-results>` 送达"。主 agent 误判为死锁，绕开图手动完成验证。
2. **导航轮次与修复重试共享同一预算**：`budget.iter_used` 在 `node_runtime.rs:311` 按 agent 轮次递增，读文件、跑测试、查状态等无风险导航操作与"再来一次实现尝试"消耗同一额度。等待子 agent 期间的大量调查轮次烧光了 `max_iter`，待报告送达、验证重新可走时，`retry_or_escalate`（`work_graph.rs:66`）发现预算耗尽，不看工作区实况直接判 Escalate。

## 根因分析

- **计费点错位**：`iter_used` 的本意是限制"盲目重试改代码"的次数（`retry_or_escalate` 的语义），但计费点在每轮 agent loop，而不是在"路由决定重试 Implement"的时刻。预算惩罚了认真调查的行为。
- **等待期策略过严**：`root_cause_dispatched` 是为防止主 agent 在专家诊断期间继续改代码造成状态竞争，但拦截范围扩大到了只读操作和验证命令，且没有向 agent 暴露等待语义，形成"越卡越烧、越烧越卡"的恶性循环。

## 修复目标

- **P0**：`root_cause_dispatched` 等待期放开只读工具与验证命令的执行；`verify_node` 返回的 `root_cause_dispatched` 响应携带等待语义（在等待哪个 child、结果如何送达）。
- **P1**：`iter_used` 只在路由选择"重试 Implement"（含 RootCause 诊断路径）时递增，导航轮次不再消耗预算。

## 非目标

- 不调整 `hooks.rs` 内层 verify-gate 的 `AutoRetry{max:2}` 默认值（与本事故无关）。
- 不实现"预算耗尽后先实况检查再判 Escalate"（P2）与按 risk 分层预算（P3），留待后续独立评估。
- 不改变 Escalate/Failed 终态语义与审计事件格式。
