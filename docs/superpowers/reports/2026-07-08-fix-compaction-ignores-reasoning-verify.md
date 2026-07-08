# 验证报告：fix-compaction-ignores-reasoning

- **Change**: fix-compaction-ignores-reasoning
- **Workflow**: hotfix
- **Verify mode**: light（手动覆盖；见下方说明）
- **日期**: 2026-07-08
- **分支**: hotfix/20260708/fix-compaction-ignores-reasoning -> 合并回 main（fast-forward），分支已删除
- **结论**: ✅ PASS

## 验证模式说明

`comet-state scale` 自动判为 `full`（任务数 7、变更文件 7），但属误判：7 个"任务"是 3 个任务组下的子项（1.1/1.2/2.1/3.1-3.4），7 个"变更文件"含 5 个 OpenSpec 产物（非代码）。真实代码改动 = 2 文件、单模块、无 delta spec、无架构/API 变更，明确属 light 范畴。按 comet-verify 覆盖机制手动设为 `light`。

## 6 项轻量检查

| # | 检查项 | 结果 | 证据 |
|---|--------|------|------|
| 1 | tasks.md 全部 `[x]` | PASS | `grep -c '\- \[ \]'` = 0（无未勾） |
| 2 | 改动文件与 tasks 一致 | PASS | `git diff --stat 9bde37b..HEAD -- src/` = compaction.rs + mod.rs，2 文件，与 tasks 1/2 对应 |
| 3 | 编译通过 | PASS | `cargo build --release` exit 0 |
| 4 | 测试通过 | PASS | `cargo test --lib` = 529 passed; 0 failed；含 2 个新测试 `test_request_size_chars_*` |
| 5 | 无明显安全问题 | PASS | diff 无新增 `unsafe`/硬编码密钥；`request_size_chars` 仅做 `str::len`，无 panic 风险（全 `unwrap_or(0)` 兜底） |
| 6 | 简化代码审查 | PASS | requesting-code-review 子代理：Ready to merge = **Yes**，0 Critical / 0 Important / 3 Minor |

## 代码审查结论

审查范围：正确性、安全、边界条件。

- **正确性**：`request_size_chars` 正确遍历 `Option<Vec<ToolCall>>` 累加 `function.arguments`；`None`/`Some(vec![])`/空串均安全归 0，无 `unwrap`/`expect`/下标越界。
- **阈值算术**：实测首次失败点 content+reasoning+tc ≈ 429K 字符对应 ~128K tokens（~3.35 字符/token）。新阈值 80K 触发于 ~320K 字符 ≈95K 真实 tokens，留余量。关键：`needs_compaction` 在 loop top（`core.rs:63`）每轮发送**前**检查，故任何实际发出的请求都已 ≤320K 字符；超额历史会在发送前被压缩，余量无需吸收整轮增长。
- **安全**：无 `unsafe`、无密钥、无注入面。
- **测试**：2 个新测试为真实回归保护（content-only 回退下会失败）。

### Minor 项处理

1. **注释余量偏乐观**（mod.rs）：原注释称 ~33K 余量，未扣工具定义+序列化开销。**已采纳**，精修为 "~30K nominal (less tool-definition + serialization overhead)" 并注明 loop-top 发送前检查。提交 `bea1540`。
2. 多 tool_calls 单消息测试 / 阈值边界测试：审查者自评 low-value/optional，**跳过**（代码路径已由现有测试覆盖）。

## 根因消除确认

- 根因 1（`needs_compaction` 只计 content）：已消除，改用 `request_size_chars`（content + reasoning_content + tool_calls.arguments）。
- 根因 2（`MAX_ESTIMATED_TOKENS=800_000` 远超窗口）：已消除，降为 80_000。
- 无深层架构问题、无接口变更，保持 hotfix 范围。

## 分支处理

用户选择：合并回 main（本地）。fast-forward 合并（main 原在 base 9bde37b），合并后在 main 上重跑 `cargo test --lib` = 529 passed。hotfix 分支已删除。`branch_status: handled`。
