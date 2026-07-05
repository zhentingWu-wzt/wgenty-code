# Verification Report — fix-subagent-focus-nav

- **Date:** 2026-07-05
- **Change:** fix-subagent-focus-nav
- **Verify mode:** full (45 tasks, 2 delta spec capabilities, 20 files)
- **Branch:** feature/20260705/fix-subagent-focus-nav
- **Base-ref:** 7568e2974f81459a78fe8ac47931302a7b830e9d
- **Head:** a552405 (after verify fix)

## Fresh Verification Evidence

| 检查 | 命令 | 结果 |
|------|------|------|
| 构建 | `cargo build` | exit 0，无错误 |
| 单元测试 | `cargo test --lib` | 501 passed; 0 failed; 0 ignored |
| Lint | `cargo clippy --lib` | 无 error/warning |
| OpenSpec 校验 | `openspec validate fix-subagent-focus-nav` | Change is valid |

## Summary Scorecard

| 维度 | 状态 |
|------|------|
| Completeness | 45/45 tasks `[x]`；2 delta spec requirements 全覆盖 |
| Correctness | 全部 spec scenarios 可追溯到代码；2 个 capability 的 requirement 实现一致 |
| Coherence | design.md D1–D12 全部实现；Design Doc 一致；无 delta spec vs design doc 矛盾 |

## Completeness

- **tasks.md**: 45/45 任务勾选（`grep -c '\- \[ \]'` = 0）。
- **delta spec requirements**:
  - `subagent-focus-view`: 2 个 MODIFIED requirement（navigation and exit / selector bar）→ 实现覆盖。
  - `subagent-status-display`: 1 个 MODIFIED requirement（status bar below input area）→ 实现覆盖。

## Correctness — Scenario → Code 映射

### subagent-focus-view

| Scenario | 代码位置 | 验证 |
|----------|---------|------|
| Arrow keys navigate the selector | `event.rs` KeyCode::Up/Down → `visible_node_ids` + `wrap_prev/wrap_next` | ✓ 代码 + 单测 |
| Enter switches subagent or exits to main | `event.rs` KeyCode::Enter → `selector_index==0` 退出，否则 `build(visible[idx-1])` | ✓ 代码 |
| Timeline scrolls only via mouse wheel | `event.rs` MouseScrolled → focus scroll；focus view 内无 PageUp/PageDn | ✓ 代码 |
| Fold toggle is always available | `event.rs` `KeyCode::Char('t')`（无 active_area 守卫） | ✓ 代码 |
| Tab is a no-op | focus view Tab 分支已删，`_ => return` 兜底 | ✓ 代码 |
| Exiting focus view returns to main | `event.rs` Esc → `subagent_focus = None` | ✓ 代码 |
| Selector shows main + real subagents only | `build_selector_lines` 用 `visible_node_ids`（`real_node_list` + 完成态过滤） | ✓ 代码 + 单测 |
| Grouping nodes excluded | `subagent_tree.rs` `is_grouping_node` + `real_node_list`；`active_node_ids` 走 `real_node_list` | ✓ 单测 |
| Cursor aligns with current subagent on entry | `FocusViewState::build` `selector_index = pos+1` | ✓ 单测 |
| Selector scrolls to keep cursor visible | `build_selector_lines` 滑动窗口 `scroll_start` | ✓ 单测（no-overflow + cursor-visible） |
| Selector wraps around including main | `wrap_prev/wrap_next` len = `visible.len()+1` | ✓ 代码 |
| Selector distinguishes cursor from current view | `▶` vs `●` marker | ✓ 代码 |
| Selector border indicates interactive area | `selector_border = active_border`，`timeline_border = inactive_border` | ✓ 代码 |
| Completed subagents are dimmed | `build_selector_lines` `label_color` dim gray for completed | ✓ 代码 |
| Completed subagents removed after delay | `visible_node_ids` 过滤 `elapsed > 10s` | ✓ 单测 |
| Currently viewed subagent exempt from removal | `visible_node_ids` `if id == current_node_id return true` | ✓ 单测 |

### subagent-status-display

| Scenario | 代码位置 | 验证 |
|----------|---------|------|
| Status bar appears when subagents start | `render.rs` `status_bar_height = active_count().min(5)` | ✓ 代码（既有） |
| Status bar shows each active subagent | `subagent_status_bar.rs` 渲染 | ✓ 既有 |
| Arrow keys auto-activate and navigate | `event.rs:310` Up/Down → `subagent_status_bar_focused = true` + 导航 | ✓ 代码（D10） |
| Esc deactivates status bar focus | `event.rs:337` Esc → `subagent_status_bar_focused = false` | ✓ 代码 |
| Tab does not toggle status bar focus | `event.rs` Tab arm → `return`（无状态变更） | ✓ 代码（verify 修复） |
| Status bar hides when no subagents active | `active_count()==0` → height 0 | ✓ 既有 |
| Status bar Enter triggers focus view | `event.rs:325` Enter → `FocusViewState::build` | ✓ 既有 |
| Status bar does not interfere with text input | 非 ↑↓/Esc/Enter/Tab 键 → disengage + 透传输入框 | ✓ 代码 |

## Coherence

- **design.md D1–D12**: 全部实现（D1 FocusArea 移除、D2 光标对齐、D3 滚动跟随、D4 高度 8、D5 键位、D6 边框、D7 task 包装移除、D8 完成态跟踪、D9 分组过滤、D10 状态栏自动激活、D11 主聊天滚动、D12 input.rs update_style）。
- **Design Doc**（`docs/superpowers/specs/2026-07-05-fix-subagent-focus-nav-design.md`）: 与 design.md 同源，决策一致。
- **delta spec vs design doc**: 无矛盾。两份 delta spec 的 scenario 与 design 决策一一对应。
- **proposal.md 目标**: 全部满足（键位重做、滚动跟随、光标对齐、完成态灰显+移除、包装节点移除、分组过滤、状态栏自动激活、主聊天滚动简化、输入框修复）。
- **代码模式一致性**: 新代码遵循既有 ratatui 渲染模式（Block/Borders/Style/Span）；`SubagentTree` 方法风格一致；测试用既有 `make_node`/`make_progress` helper。

## Issues Found

### WARNING（已修复）

1. **Tab 在状态栏非真 no-op**（subagent-status-display delta spec「Tab does not toggle status bar focus」）。
   - **位置**: `src/tui/app/event.rs` 状态栏 match 的 `_` 分支。
   - **问题**: 原实现中 Tab 落入 `_` 分支会 `subagent_status_bar_focused = false`（disengage），这对 focus 状态有影响，与 spec「Tab SHALL have no effect on status bar focus」不一致。
   - **修复**: 新增显式 `KeyCode::Tab => return,` 分支，消费 Tab 但不变更 focus 状态（commit `a552405`）。
   - **复验**: build + 501 tests pass。

### CRITICAL / IMPORTANT

无。

## 交互式 TUI 手动验收（deferred）

下列场景需要交互式 TUI 操作（启动 TUI、触发 subagent、按键核对），无法由自动化测试覆盖；代码路径已逐项核对（见上表），最终交互式验收建议在归档前由用户抽检：
- ↑↓ 导航选择器与 wrap、Enter 切换/退出、鼠标滚 timeline、't' 折叠、Tab no-op。
- 选择器滚动跟随（多 subagent）、完成态灰显 + 10s 后移除。
- task 包装节点不出现、delegate 分组节点不出现、active 计数正确。
- 状态栏 ↑↓ 自动激活、Esc 取消、字符输入透传。
- 主聊天 PageUp/PageDn + 鼠标滚轮、↑↓ 不滚聊天。
- 输入框在 subagent 频繁更新时不闪烁。

## Final Assessment

无 CRITICAL / IMPORTANT 问题。1 个 WARNING（Tab spec 一致性）已在 verify 阶段修复并复验通过。所有 delta spec scenario 可追溯到代码，关键逻辑有单测覆盖（501 passed）。design D1–D12 全部实现，无 spec/design 矛盾。

**结论：Ready for archive**（交互式 TUI 抽检建议在归档前由用户完成；若发现问题可回退 build 修复）。
