# 验证报告 — fix-bg-subagent-tree-clear

- **日期**: 2026-07-06
- **Change**: fix-bg-subagent-tree-clear
- **Workflow**: hotfix
- **verify_mode**: full（delta spec：subagent-status-display MODIFIED）
- **Branch**: hotfix/20260706/fix-bg-subagent-tree-clear
- **Base ref**: 30448d7（W1 修复后）
- **Head**: de972fd（build complete — transition to verify）

## 修复概述

| Bug | 根因 | 修复 |
|---|---|---|
| 后台 subagent 运行中提交新提示词 → selector 消失 | `TurnStarted` 无条件 `subagent_tree.clear()` 抹掉跨 turn 存活的 background subagent（task 工具 background 模式 via `tokio::spawn`，主 turn 立即返回并完成） | 新增 `SubagentTree::clear_if_idle()`（仅当无活动 subagent 才清）；`TurnStarted` 改用它，且只在清树时重置 `completed_at`/`subagent_focus`/`selected` |

**根因调查**（systematic-debugging 四阶段）：
- Phase 1：二进制 mtime 16:27 > 修复 commit 14:17，排除 stale binary；全仓库仅 `event.rs:753`（TurnStarted）+ `:823`（TurnAborted）两处 `subagent_tree.clear()`，无其他清树路径；SubagentError 并行改动不触及 tree。
- Phase 2：前台（同步）subagent 阻塞主 turn → 提交入队、不开新 turn → 树保留（上次修复有效）；后台 subagent 不阻塞 → 主 turn 完成 → 提交开新 turn → TurnStarted 清树（未修）。
- Phase 3：假设「TurnStarted 无条件清树是根因」，代码追踪确认。
- Phase 4：4 个 `clear_if_idle` 失败/通过测试 + 实现修复。

## 4 项完整验证（openspec-verify-change）

### Completeness

| 检查 | 结果 | 证据 |
|---|---|---|
| tasks.md 全部 `[x]` | PASS | 0 未勾选 / 6 已勾选 |
| delta spec 需求实现 | PASS | MODIFIED "Subagent tree lifecycle across submitted prompts" 4 个 scenario 均落实 |

### Correctness（delta spec scenario → 代码）

| Scenario | 结果 | 代码证据 |
|---|---|---|
| Submitting while a turn is running preserves the tree | PASS | `event.rs` Submit 不清树（只 `submit_input`）；`submit_input` 在 `current_turn_handle.is_some()` 时入队 |
| **Submitting while a background subagent runs preserves the tree**（NEW） | PASS | `event.rs` TurnStarted `if self.subagent_tree.clear_if_idle() {…}`——有活动 subagent 时返回 false，不清树、不重置 focus/selected |
| New turn start clears the tree only when no subagents are active（MODIFIED） | PASS | `clear_if_idle`：`active_count() == 0` 才 `clear()` 返回 true；TurnStarted 仅在 true 时重置 `completed_at`/`focus`/`selected` |
| Turn abort clears the tree | PASS | `event.rs` TurnAborted 仍 `subagent_tree.clear()`（全清，1 处） |

### Coherence

| 检查 | 结果 | 说明 |
|---|---|---|
| Design adherence | PASS | design.md 决策均落实：`clear_if_idle()` 方法 + TurnStarted 条件清树 + TurnAborted 保持全清。 |
| Code pattern consistency | PASS | `clear_if_idle` 沿用 `active_count()`/`clear()` 既有方法；TurnStarted 条件块风格与既有 handler 一致。 |
| Design Doc | N/A | hotfix 无 Design Doc。 |

## 自动化证据

- `cargo build`：exit 0（含用户并行工作；独立 stash 并行工作后亦通过）。
- `cargo test --lib`：513 passed / 0 failed（509 + 4 新 `clear_if_idle` 测试）。
- `subagent_tree` 模块测试：15 passed。
- hotfix 独立性：临时 stash 用户 6 个并行文件后，fix 两文件独立编译 + 15 测试通过。

## 代码自审（correctness / security / boundaries）

- `clear_if_idle` 用 `active_count()`（走 `real_node_list`，排除 grouping 节点）→ 一个卡 Running 的 delegate 包装节点不会阻止清树。✓
- `clear_if_idle` 返回 false 时，`completed_at`/`subagent_focus`/`selected` 全部保留——background subagent 的 focus 视图与用户选择不被打断。✓
- 上一 turn 已完成 subagent 在保留时不清除，但被状态栏 active 过滤 + focus view completed-after-10s 过滤隐藏，UI 不受影响；`subagent_history` 快照已捕获上一 turn 状态，无历史丢失。轻微内存累积可接受。✓
- 无 `unsafe`、无密钥、无注入面。
- 边界：空树 `clear_if_idle` → true（清空，无害）；混合 Running+Completed → false（保留全部，不清一半）。✓

## 并行工作说明（非本 change）

工作区有用户并行编辑：`WGENTY.md`、`src/config/agent.rs`、`src/teams/subagent_loop.rs`、`src/tools/meta/task.rs`、`src/tui/agent/mod.rs`、`tests/refactor_e2e_test.rs`（SubagentError feature 等）。经确认为用户独立工作，**不属于本 hotfix**。本 fix 提交（`e495bf2`）仅含 `subagent_tree.rs` + `event.rs`，并行文件保持 unstaged。

## 交互式 TUI 手动验收（未由 agent 执行）

需真实终端，列为用户手动验收清单：

1. 触发一个 **background** subagent（task 工具 background 模式）→ 主 turn 立即返回并完成 → 状态栏显示 background subagent。
2. 在 background subagent 仍 Running 时提交新提示词 → **状态栏不消失**，仍能 Enter 进 background subagent 的 focus view。
3. background subagent 完成后 → 状态栏自然隐藏（active_count=0）；再提交新提示词 → TurnStarted 清树（空闲清）。
4. 前台 subagent 运行中提交 → 入队，状态栏不消失（上次修复，回归确认）。
5. /clear → TurnAborted 全清，状态栏消失。

## 结论

完整验证：4 个 delta spec scenario 全部映射到代码并 PASS；Completeness / Correctness / Coherence 三维均无 CRITICAL / WARNING。513 测试通过，fix 独立编译自洽。根因（TurnStarted 无条件清树）已消除。

**验证通过。**
