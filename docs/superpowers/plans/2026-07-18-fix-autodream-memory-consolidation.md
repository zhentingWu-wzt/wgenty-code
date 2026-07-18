---
change: fix-autodream-memory-consolidation
design-doc: openspec/changes/fix-autodream-memory-consolidation/design.md
base-ref: bb0c340fd843d8539b78966d2659a5ff1a429a83
archived-with: 2026-07-18-fix-autodream-memory-consolidation
---

# 实施计划: 修复 AutoDream 记忆整合

> **关联设计文档**: `openspec/changes/fix-autodream-memory-consolidation/design.md`
> **关联任务清单**: `openspec/changes/fix-autodream-memory-consolidation/tasks.md`
> **执行模式**: direct + TDD

## 决策映射

| 决策 | 任务 | 文件 |
|------|------|------|
| D6: 移除未使用的 `_state` 参数 | Task 1 | `services/auto_dream.rs` + 5 调用方 |
| D2: 门控阈值 24h/5s -> 1h/1s | Task 2 | `services/auto_dream.rs` |
| D3: 移除 AutoDream 自管锁 | Task 2 | `services/auto_dream.rs` |
| D1: daemon 注入 mm + memory_add + AutoDream | Task 3 | `daemon/state.rs` |
| D4: 移除 TUI app 侧 AutoDream | Task 4 | `tui/app/mod.rs` |
| D4: headless 接入 AutoDream | Task 5 | `cli/headless_runtime.rs` |

## TDD 流程

- Task 1 (D6): 先更新测试调用方 (Red) -> 改签名+5调用方 (Green)
- Task 2 (D2/D3): 先写 `test_default_thresholds_are_relaxed` + `test_check_and_run_does_not_write_disk_lock` (Red) -> 改阈值+移除锁 (Green)
- Task 3-5: 直接实现 + `cargo check` 验证

## 验证结果

- `cargo check --lib` ✅
- `cargo test --lib services::auto_dream` ✅ 3/3
- `cargo test --lib memory_add` ✅ 6/6
- `cargo test --lib context` ✅ 100/100
- `cargo fmt --check` ✅
- `cargo clippy` - 3 errors, 全部 pre-existing (git stash 确认)
