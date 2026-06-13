# Brainstorm Summary

- Change: reduce-tool-output-verbosity
- Date: 2026-06-14

## 确认的技术方案

**方案 A：参数驱动**

- grep: 新增 `files_with_matches: bool` 参数（默认 false），切换紧凑/详细模式
- grep: 行截断 200 chars，`…[truncated]` 后缀
- Read: `max_chars` 默认值 12000 → 6000
- Read: 行截断 300 chars，`…[truncated]` 后缀
- 截断阈值内置，不暴露为参数

## 关键取舍与风险

- `files_with_matches` 模式下用户看不到匹配内容 → 默认 false，用户显式 opt-in
- 行截断可能隐藏关键信息 → `…[truncated]` 标记提醒用户
- 默认 max_chars 降低可能让老用户意外 → 始终可通过显式 max_chars 覆盖

## 测试策略

- `cargo test --lib` — 所有现有测试通过
- `cargo build` — 编译通过
- `cargo clippy` — 无新增 warnings
- 手动验证 grep `files_with_matches: true` 输出格式

## Spec Patch

无
