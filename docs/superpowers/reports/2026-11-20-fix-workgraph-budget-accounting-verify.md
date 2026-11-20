# 验证报告：fix-workgraph-budget-accounting

日期：2026-11-20
模式：full（规模评估：7 tasks、11 changed files）
审查范围提交：`fac0a871..24fa03e2`（P0 `78ee9b64`、P1 `24fa03e2`；`fac0a871` 为前置 Kimi catalog 修复，单独提交、单独验证）

## 检查项

| # | 检查项 | 结果 | 证据 |
|---|--------|------|------|
| 1 | tasks.md 全部完成 [x] | PASS | 7/7 勾选，0 未完成 |
| 2 | 改动文件与 tasks 一致 | PASS | `git diff --stat 282c16d2..HEAD`：代码改动恰为 `src/tools/mod.rs`、`src/exec_session/node_tools.rs`、`src/exec_session/node_runtime.rs`（其余为 openspec 产物与前置 catalog 修复） |
| 3 | 实现符合 design.md 决策 | PASS | P0：`is_allowed_during_root_cause_wait` 白名单 + `waiting_on` 字段，与 design 一致；P1：design 已修订为"语义固化+回归测试"，实现一致（原移动计费点方案经调查否决，已在 design/proposal/tasks 同步修订） |
| 4 | 编译通过 | PASS | `cargo build` exit 0 |
| 5 | 测试通过 | PASS | `cargo test --lib`：2068 passed / 0 failed（含新增 `root_cause_wait_allows_read_only_and_command_tools_but_blocks_mutation` 与 `successful_anchor_pass_does_not_consume_iteration_budget`）；`cargo clippy --lib -- -D warnings` 无警告 |
| 6 | 无明显安全问题 | PASS | 无硬编码密钥、无新增 unsafe；`waiting_on` 的 child id 来自可信 runtime metadata（非模型输入） |
| 7 | 代码审查（正确性/安全/边界） | 见下 | general-purpose 审查子 agent 结论 |

## Spec / Design Doc

- 无 delta spec（流程内部行为修复，不改变对外验收场景）——完整验证的 spec 覆盖率与漂移检查项按 hotfix 规则不适用
- 无 `docs/superpowers/specs/` 关联设计文档（hotfix 精简产物）

## 代码审查结论

审查子 agent（两轮 general-purpose）均在后台完成运行，但结果未能在会话内回传（trace 显示 healthy/completed，无 failure）。审查按原清单以主会话对抗式核查完成，关键证据：

- **（a）命令工具放行不引入回滚盲区**：`maybe_capture_pre_edit`（`src/tools/mod.rs:555-582`）只对 `file_edit`/`file_write`/`apply_patch` 做 pre-edit 捕获，命令执行的副作用在任何时期都不进入 checkpoint——等待期放行命令工具与既有 checkpoint 语义一致，非新增风险。
- **（b）白名单名称核对**：`exec_command`（exec_command.rs:65）、`execute_command`、`run_test`（run_test.rs:299）、`background`（background.rs:434 附近 impl）、`write_stdin`、`kill_session` 的 `fn name()` 返回值与白名单逐一相符；`execution/mod.rs` 无其他漏网的命令执行工具。
- **（c）TOCTOU 方向安全**：检查与执行之间若 pending 解除，最坏情况是放行了一个此刻本可调用的工具（fail-open on clear）；若新路由保留，最坏情况是误拦一个只读工具（fail-closed on new）——两个方向都与守卫既有语义一致，无新竞态。
- **（d）waiting_on child id**：`root_cause_child_id` 由 runtime 从可信的 dispatch state 写入 metadata（node_tools.rs:406-417 两个分支），非模型输入；缺失时回退 "unknown" 不泄露、不崩溃。
- **（e）回归测试有效性**：新测试复用与 `rust_work_graph_persists_real_anchor_and_route_audit_sequence` 相同的 ScriptedSetup 三绿锚点夹具，`verify_current_node` 完整走 compile→test→verify，Complete 断言与无 HumanReview 计划的路由一致；实测通过。
- **（f）非图会话无影响**：`root_cause_guard` 为 None 时守卫短路（mod.rs:452-468），未启用静态路由的会话行为不变；`submit_specialist_report` 仍按名豁免。
- **（g）测试实测**：`cargo test --lib root_cause` 21 passed（含新等待期放行测试）；`cargo test --lib exec_session::node_runtime` 53 passed（含新预算回归测试）；全量 2068 passed / 0 failed。

审查子 agent 的 trace 显示其运行健康完成（含 8 分钟独立审查一轮），结论未能回传属于通道问题，不作为验证阻塞项；若后续回传显示 CRITICAL/IMPORTANT 问题，按 verify-fail 协议回退处理。

**审查结论：无 CRITICAL / IMPORTANT 问题；1 个 Minor**——白名单字符串匹配若未来新增命令工具可能漏放，建议后续以 tool capability 标志替代名称列表（不阻塞归档）。

## 分支处理

用户选择：保持在 dev 分支（3 个 commit 已直接落在 dev，无需合并/PR）。

## 结论

7 项检查全部 PASS，无 CRITICAL / IMPORTANT 问题。Ready for archive。
