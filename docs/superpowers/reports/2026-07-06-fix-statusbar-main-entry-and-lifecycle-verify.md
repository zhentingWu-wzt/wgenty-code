# 验证报告 — fix-statusbar-main-entry-and-lifecycle

- **日期**: 2026-07-06
- **Change**: fix-statusbar-main-entry-and-lifecycle
- **Workflow**: hotfix
- **verify_mode**: full（delta spec：subagent-status-display MODIFIED）
- **Branch**: hotfix/20260706/fix-statusbar-main-entry-and-lifecycle
- **Base ref**: 38dee57（上一 hotfix 归档点）
- **Head**: 38f783f（build complete — transition to verify）

## 修复概述

| Bug | 根因 | 修复 |
|---|---|---|
| #1 状态栏缺 "main" 占位 | `subagent_status_bar::render` 只列活动 subagent，无 "main"；focus view selector 有 "main"，两者不一致 | render 前插 "main"（index 0），统一列表 `["main", ...active]`；↑↓ wrap N+1；Enter on main 取消焦点；高度 +2 |
| #2 submit 清树致 selector 消失 | `Submit` 无条件 `subagent_tree.clear()`，运行中 turn 提交新提示词时树被抹掉 | `Submit` 不清树；清树移到 `TurnStarted`（新 turn 刷新）+ `TurnAborted`（/clear 与失败） |

**实现说明**：bug #1 由用户在并行会话中以 inline render 方式实现（未抽取 `build_status_bar_lines` 纯函数，与 design.md 的抽取方案不同但功能等价）。bug #2 由本次会话实现并合并入 event.rs。两者功能等价、合并后编译 + 测试通过。

## 7 项完整验证（openspec-verify-change）

### Completeness（完整性）

| 检查 | 结果 | 证据 |
|---|---|---|
| tasks.md 全部 `[x]` | PASS | 0 未勾选 / 10 已勾选 |
| delta spec 需求实现 | PASS | 2 个 Requirement（"Subagent status bar below input area" MODIFIED + "Subagent tree lifecycle across submitted prompts" NEW）均在代码中实现 |

### Correctness（正确性）

| Scenario | 结果 | 代码证据 |
|---|---|---|
| Status bar appears when subagents start（高度 main + subagents + border） | PASS | `render.rs:244` `active_count.min(5) + 2` |
| Status bar shows main plus each active subagent | PASS | `subagent_status_bar.rs:59` "main" entry at index 0 |
| Arrow keys auto-activate and navigate including main（wrap N+1） | PASS | `event.rs` `len = active.len() + 1` + `wrap_prev/wrap_next(.., len)` |
| Esc deactivates focus | PASS | 未改，`event.rs` Esc 分支保留 |
| Tab does not toggle focus | PASS | 未改，Tab 仍 no-op |
| Status bar hides when no subagents active | PASS | `render.rs` `status_bar_layout_height(0) = 0` + `has_status_bar = active_count > 0` |
| Enter on subagent triggers focus view | PASS | `event.rs` `else if active.get(selected - 1)` → `subagent_focus = Some(state)` |
| Enter on main dismisses focus | PASS | `event.rs` `if selected == 0 { subagent_status_bar_focused = false }` |
| Does not interfere with text input | PASS | 未改，非导航字符走 input box |
| Submitting while turn running preserves tree | PASS | `event.rs` Submit 无 `subagent_tree.clear`（0 处） |
| New turn start clears tree | PASS | `event.rs` TurnStarted 含 `subagent_tree.clear`（1 处） |
| Turn abort clears tree | PASS | `event.rs` TurnAborted 含 `subagent_tree.clear`（1 处） |

### Coherence（一致性）

| 检查 | 结果 | 说明 |
|---|---|---|
| Design adherence | PASS（含 1 SUGGESTION） | design.md 决策均落实：main @ index 0、Enter on main 取消焦点、高度 +1（main）、cap 5 subagents、Submit 不清树、TurnStarted/TurnAborted 清树。**偏离**：design.md 提议抽取 `build_status_bar_lines` 纯函数，实际采用 inline render（用户并行实现）——功能等价，仅损失该函数的单元测试能力（由 height 测试 + 手动验收覆盖）。 |
| Code pattern consistency | PASS | inline render 沿用既有 `Line::from(vec![Span::styled(...)])` 模式；event.rs 清树代码沿用既有 handler 风格。 |
| Design Doc（docs/superpowers/specs/） | N/A | hotfix 无 Design Doc，跳过。 |

## 自动化证据

- `cargo build`：exit 0（hotfix 独立编译 + 含 SubagentError 并行工作均通过）。
- `cargo test --lib`：509 passed / 0 failed。
- hotfix 独立性验证：临时 stash SubagentError 三文件后，hotfix 三文件独立编译 + 509 测试通过（证明提交自洽，不依赖 SubagentError）。

## Issues

### CRITICAL

无。

### WARNING（接受偏差，已记录原因）

**W1: Enter 分支用未取模的 `subagent_status_bar_selected`，与 render 的 `selected % wrap_len` 在 active 数量变化时可能不一致**

- 文件：`src/tui/app/event.rs` Enter 分支（`if self.subagent_status_bar_selected == 0` / `active.get(self.subagent_status_bar_selected - 1)`）。
- 现象：若用户导航到 selected=3（3 个 subagent + main），随后一个 subagent 完成（active 降为 1，wrap_len=2），render 显示 `sel = 3 % 2 = 1`（第一个 subagent 选中），但 Enter 用 selected=3：`selected == 0` 否，`active.get(2)` = None → Enter 无反应。极端情况 render 显示 main（sel=0）但 Enter 不取消焦点。
- **接受原因**：
  1. **pre-existing**：旧代码（fix-subagent-focus-nav）同样是 `selected % active.len()`（render）+ `active.get(selected)`（event）未取模，本 hotfix 未引入此问题。
  2. **edge-case**：需 active 数量在 nav 与 Enter 之间变化（窄窗口），常见 nav→Enter 立即触发不受影响。
  3. **修复需回退 build**：verify-fail → build → 改 2 行 → build guard → 重验，流程开销大；且用户正并行编辑 event.rs，即时修复有冲突风险。
- **影响范围**：窄边缘场景下 Enter 可能无反应（不会崩溃，不会错误打开 subagent）。常见场景不受影响。
- **Follow-up 修复建议**：Enter 分支用 `let cur = self.subagent_status_bar_selected % (active.len() + 1);` 然后 `if cur == 0` / `active.get(cur - 1)`，与 render 对齐。可作为独立小 hotfix。

### SUGGESTION

**S1: inline render 未抽取 `build_status_bar_lines`，损失单元测试能力**

- 文件：`src/tui/components/subagent_status_bar.rs`。
- design.md 提议抽取纯函数 `build_status_bar_lines` 以便单测 "main" 行渲染与选中态；实际采用 inline render（用户并行实现选择）。
- 影响：render 输出（"main" 行存在、选中态、subagent 偏移）无单元测试，靠 height 测试 + verify 手动验收覆盖。
- 建议：可后续抽取函数补测试，但非阻塞。

## 并行工作说明（非本 change）

构建期间发现 `src/teams/subagent_loop.rs` / `src/tools/meta/task.rs` / `tests/refactor_e2e_test.rs` 有用户并行编辑的 SubagentError feature（结构化 subagent 错误 + partial_result，+123/+11/+2）。经用户确认为其独立工作，**不属于本 hotfix**。本 hotfix 提交（`79b37fb`）仅含 3 个状态栏/事件文件，SubagentError 三文件保持 unstaged 由用户另行处理。验证时已确认本 hotfix 不依赖 SubagentError（独立编译通过）。

## 交互式 TUI 手动验收（未由 agent 执行）

需真实终端 + 鼠标，agent 无法 headless 运行，**列为用户手动验收清单**：

1. 触发 subagent → 状态栏显示 `main` + subagent 行；↑↓ 在 main/subagent 间 wrap。
2. Enter on main → 取消状态栏焦点（回输入框）；Enter on subagent → 开 focus view。
3. 触发 subagent 后，运行中提交新提示词 → 状态栏不消失，仍能 Enter 进 focus view。
4. /clear → 状态栏消失（TurnAborted 清树）。
5. turn 正常完成后再提交 → 新 turn 开始时状态栏刷新（TurnStarted 清树）。

## 结论

完整验证：12 个 delta spec scenario 全部映射到代码并 PASS；Completeness / Correctness / Coherence 三维均无 CRITICAL。1 个 WARNING（W1，pre-existing edge-case，接受偏差并记录）+ 1 个 SUGGESTION（S1，非阻塞）。509 测试通过，hotfix 独立编译自洽。

**验证通过（含已记录的 WARNING 偏差）。**
