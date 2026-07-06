# 验证报告 — fix-statusbar-height-mouse-scroll

- **日期**: 2026-07-06
- **Change**: fix-statusbar-height-mouse-scroll
- **Workflow**: hotfix
- **verify_mode**: light（手动覆盖；见下方「规模评估」说明）
- **Branch**: hotfix/20260706/fix-statusbar-height-mouse-scroll
- **Base ref**: 66531727c86bdb0ceb264950aaa016622f6ffd4b
- **Head**: b61db6479be3167e22e8ea899795b351169d5a29

## 规模评估

`comet-state scale` 自动判定为 `full`（tasks=9、files=7）。手动覆盖为 `light`，理由：

- **真实源码改动 2 个文件**：`src/cli/args.rs`(+8)、`src/tui/app/render.rs`(+64)。7 个文件中有 5 个是每次 change 必备的 openspec 产物（proposal/design/tasks/.comet.yaml/.openspec.yaml），不属于实现规模。
- **0 delta spec**（低于阈值 1）。
- **9 tasks 是 3 个逻辑段下的细粒度子项**（1.1–1.3 / 2.1–2.3 / 3.1–3.3），实际为 2 个实现任务 + 1 个验证段。
- 无架构 / 接口 / schema 变更，单层 TUI 改动。
- 符合 skill 的覆盖机制：「如 agent 认为自动评估结果不合适，可手动覆盖」。

## 修复概述

| Bug | 根因 | 修复 |
|---|---|---|
| #1 主窗口看不到 selector 项 | `render.rs` 状态栏高度 `active_count().min(5)` 未计入 `Borders::TOP` 占用的 1 行，N 项只显示 N-1（N=1 → 0 可见） | 抽取 `status_bar_layout_height(active_count) = active_count.min(5) + 1`，0 时返回 0 |
| #2 似乎需要 Tab 才能聚焦 | #1 的表象：`event.rs:306-355` 的 ↑↓ 自动激活逻辑本就正确，但状态栏被裁切到 0 行，用户看不到 ▶ 移动 | 无单独修复；#1 修复后 ↑↓ 可见，Tab 仍是 no-op（符合 spec） |
| #3 聊天区鼠标滚动失效 | `args.rs` 终端初始化从未 `EnableMouseCapture`，crossterm 不投递 `Event::Mouse`，`AppEvent::MouseScrolled` 永不触发 | `EnterAlternateScreen` 后启用 `EnableMouseCapture`；正常退出 + panic hook 中配对 `DisableMouseCapture` |

## 6 项轻量验证

| # | 检查项 | 结果 | 证据 |
|---|---|---|---|
| 1 | tasks.md 全部 `[x]` | PASS | `grep -c '^\- \[ \]'` = 0 未勾选 / 9 已勾选 |
| 2 | 改动文件与 tasks 一致 | PASS | `git diff --stat base..HEAD -- src/` = `args.rs`(+8) + `render.rs`(+64)，对应 Task 1 / Task 2 |
| 3 | 编译通过 | PASS | `cargo build` exit 0 |
| 4 | 相关测试通过 | PASS | `cargo test --lib` = 509 passed / 0 failed（含 6 个新 `status_bar_layout_height` 测试） |
| 5 | 无明显安全问题 | PASS | diff 无 `unsafe` / `secret` / `password` / `api_key` / `panic!`；仅终端控制序列 |
| 6 | 简化代码审查 | PASS | 见下方「代码审查结论」 |

## 代码审查结论

`requesting-code-review` 派发的审查 subagent 结论：**Assessment: Ready to proceed**。

### 优点（审查确认）

- `status_bar_layout_height` 纯函数 + 6 个单元测试覆盖 0/1/3/5/6/50 边界，封顶正确。
- 高度数学正确：`Borders::TOP` 占 1 行（`block.inner(area)` 返回 `height-1`），`active_count.min(5)+1` 使 inner 恰好容纳 N≤5 内容行。已对照 `subagent_status_bar.rs:45-50` 验证。
- `has_status_bar` 正确改为由 `active_count > 0` 派生（旧式 `status_bar_height > 0` 在新 helper 下会误判）。
- 鼠标捕获顺序正确：setup `EnterAlternateScreen → EnableMouseCapture → enable_raw_mode`；teardown（正常 + panic）逆序 `disable_raw_mode → DisableMouseCapture → LeaveAlternateScreen`。
- panic hook 用 `let _ =`（best-effort，`execute!` 返回 `io::Result` 不 panic，无重入风险）；正常路径用 `?`（捕获失败则启动中止，合理——鼠标滚动是 spec 规定的滚动机制）。
- Bug #2 确认无需改动：`event.rs:306-312` 已在 `subagent_focus.is_none()` 且 active 非空时按 ↑↓ 设 `subagent_status_bar_focused = true`。症状纯系 0 高度不可见。
- 无 `unsafe`、无密钥、无注入面。

### Minor 问题（均为既有、非本次引入，不阻塞验证）

1. **`subagent_status_bar::render` 与 `active_count()` 对 grouping 节点不一致**（`subagent_status_bar.rs:25-34` vs `subagent_tree.rs:60-63`）：render 按 `tree.nodes.values()` 过滤 Running|Pending，包含 grouping 节点；`active_count()` 走 `real_node_list()` 排除 grouping 节点。若一个 delegate 包装节点卡在 Running，render 画的行数 > 高度分配，最后一行仍可能被裁切。
   - **本次影响**：不恶化（+1 反而多一行余量）；D7 已移除 `task` 包装节点，common case 无 grouping 节点。
   - **为何不在本 hotfix 修**：属第 3 个文件的独立既有问题，超出用户报告的 3 个 bug 范围，且会触发 hotfix→full 升级条件（3+ 文件）。
   - **Follow-up 建议**：render 改用 `active_node_ids(tree).iter().filter_map(|id| tree.nodes.get(id))`，与高度计算对齐。

2. **`enable_raw_mode()?` 失败时终端状态泄漏**（`args.rs:160`）：`EnableMouseCapture` 成功后若 `enable_raw_mode` 返回 Err，函数直接返回，不跑 `DisableMouseCapture`/`LeaveAlternateScreen`。panic hook 只覆盖 panic，不覆盖 Err。
   - **本次影响**：边际恶化（`EnterAlternateScreen` 原本就有同样的泄漏窗口）。
   - **实际概率**：极低（同一进程内 daemon 终端初始化刚成功）。
   - **Follow-up 建议**：用 `scopeguard` / `Drop` 终端守卫消除整类泄漏（设计重构，非 hotfix 范围）。

### 边界条件验证（审查确认）

- `active_count == usize::MAX`：`min(5)+1 = 6`，fits `u16`，无溢出。
- `active_count == 0`：返回 0，`has_status_bar = false`，不渲染。
- `EnableMouseCapture` 正常路径失败：`?` 传播 Err 中止启动（合理）。
- 鼠标捕获不干扰键盘 / bracketed paste / `Event::Resize`（独立 crossterm 通道）。
- 启用鼠标捕获会接管终端模拟器原生滚轮回滚——全屏 TUI alternate screen 的预期行为。

## 自动化证据

- `cargo build`：exit 0。
- `cargo test --lib`：509 passed / 0 failed（503 既有 + 6 新增 `status_bar_layout_height` 测试）。
- `cargo test`（含集成）：全部通过（lib 509 + 集成 30 + doc 0）。
- `cargo fmt --check`：本次改动文件 `render.rs` / `args.rs` 无 diff（仓内 `daemon/state.rs`、`teams/subagent_trace.rs` 既有未格式化，非本次引入，不在范围）。

## 交互式 TUI 手动验收（未由 agent 执行）

以下场景需要真实终端 + 鼠标，agent 无法在 headless 环境运行，**列为用户手动验收清单**：

1. **Bug #1**：触发 1 个 subagent，确认状态栏显示 1 行（原 0 行）；再触发 5 个 / 6+ 个，确认 ≤5 全显示、6+ 封顶 5 项。
2. **Bug #2**：状态栏可见时按 ↑↓，确认 ▶ 即时移动、无需 Tab；Tab 无效果；Esc 取消焦点。
3. **Bug #3 主聊天**：鼠标滚轮向上看旧内容、向下回新内容，回到底部后恢复 auto-scroll。
4. **Bug #3 focus view**：Enter 进 focus view，滚轮滚动 timeline 双向，回到底部 `auto_scroll` 重激活（`event.rs:463-465`）。
5. **panic hook**：临时 `panic!("test")` 触发，确认终端恢复（鼠标捕获关、alternate screen 退出、raw mode 关）后移除。

## 结论

6 项轻量验证全部 PASS，代码审查 Ready to proceed，无 CRITICAL / IMPORTANT 问题。2 个 Minor 既有问题记为 follow-up，不阻塞本次归档。自动化证据充分；交互式手动验收移交用户。

**验证通过。**
