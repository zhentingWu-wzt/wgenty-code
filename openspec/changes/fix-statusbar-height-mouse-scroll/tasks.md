# Tasks: 状态栏高度裁切 + 鼠标滚轮失效

## 修复 1：状态栏高度计入顶边框（问题 1 + 2）

- [ ] 1.1 `src/tui/app/render.rs`：将 `status_bar_height` 从 `active_count().min(5)` 改为 `visible_items + 1`（`visible_items = active_count().min(5)`），并确保 `has_status_bar` 仍由 `active_count > 0` 判定，避免空边框。
- [ ] 1.2 抽取纯函数 `status_bar_height(active_count) -> u16`（或等价可测结构），新增单元测试覆盖 `0→0`、`1→2`、`3→4`、`5→6`、`6→6`。
- [ ] 1.3 运行 `cargo test` 确认 `subagent_status_bar` / `render` 相关测试通过、无回归。

## 修复 2：启用鼠标捕获（问题 3）

- [ ] 2.1 `src/cli/args.rs`：在 `EnterAlternateScreen` 后添加 `execute!(stdout, EnableMouseCapture)`，导入 `crossterm::event::{EnableMouseCapture, DisableMouseCapture}`。
- [ ] 2.2 正常退出路径：在 `LeaveAlternateScreen` 前添加 `execute!(io::stdout(), DisableMouseCapture)`。
- [ ] 2.3 panic hook：在 `LeaveAlternateScreen` 前添加 `DisableMouseCapture`，保证崩溃恢复终端。

## 收尾

- [ ] 3.1 `cargo build` 全量编译通过。
- [ ] 3.2 `cargo test` 全量测试通过。
- [ ] 3.3 根因消除检查：确认 `render.rs` 不再有未计入边框的高度计算；确认 `args.rs` 含成对 `EnableMouseCapture` / `DisableMouseCapture`（含 panic hook）。
