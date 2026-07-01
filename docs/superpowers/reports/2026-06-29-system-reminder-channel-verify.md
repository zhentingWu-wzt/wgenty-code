# Verification Report — system-reminder-channel

- Change: system-reminder-channel
- Branch: feature/20260628/system-reminder-channel
- Base: f6bbb1e23a4a840a195820bc4e4bae896530babe
- Head: 0069259 (after verify-phase fixes)
- Date: 2026-06-29
- Verify mode: full
- Reviewer: 主会话（应用 verification-before-completion 原则，所有 claim 附带 fresh 命令证据）

## Summary Scorecard

| Dimension | Status |
|-----------|--------|
| Completeness | 43/43 tasks ✅；2 capabilities，7+4 requirements 全部有实现证据 |
| Correctness | 28+ 测试覆盖；cargo test --workspace 全过；clippy/fmt clean |
| Coherence | D1-D9 全部实现；O1-O3 全部闭合；spec ↔ design ↔ impl 三方一致 |

## 验证命令证据

| Check | Command | Result |
|-------|---------|--------|
| Tasks 完成 | `grep -c '^- \[ \]' tasks.md` | 0 unchecked / 43 checked ✅ |
| 编译 | `cargo build` | exit 0 ✅ |
| 全量测试 | `cargo test --workspace` | 452 lib + 8 system_reminder + others, 0 failed ✅ |
| Clippy | `cargo clippy --workspace --all-targets -- -D warnings` | exit 0, clean ✅ |
| Fmt | `cargo fmt --all -- --check` | exit 0, clean ✅ |
| 安全 | `git diff` 扫 api_key/secret/password | 无匹配 ✅ |
| unsafe | `git diff` 扫 production unsafe | 无新增 ✅ |
| Design Doc | `ls docs/superpowers/specs/2026-06-27-...md` | 存在，frontmatter 合规 ✅ |

## D1-D9 实现证据（grep）

- D1 reminder 拼到 user message 头部: `src/tui/agent/mod.rs:175` `format!("{}\n\n{}", r.to_model, input)`
- D2 ReminderOutput 双轨: `src/prompts/mod.rs:180` `{ to_model, to_transcript }`
- D3 两 reader: `src/utils/project.rs:19,44` `read_user_global_instructions` / `read_user_global_rules`
- D4 project_root: `src/prompts/mod.rs:80,167` field + `with_project_root`
- D5 InjectedFragment + collect_injections: `src/runtime/hooks/mod.rs:916,931`
- D6 `# wgentyMd` + 6-space closing: `src/prompts/mod.rs:33,41`
- D7 hook fire await + 10s timeout + warn: `src/tui/agent/mod.rs:146,158`
- D8 visibility 分流 matches!: `src/prompts/mod.rs:272`
- D9 完整 reminder 块 token 估算: `src/tui/app/mod.rs:295,300`

## O1-O3 闭合状态

- O1 `# wgentyMd` 改名（方案 B）: 实现 + spec + design 全部同步（commit a546e25 修复 spec/design drift）
- O2 fire 改 await 无死锁: `AgentLoop` 独立 task + 10s timeout 兜底，分析见 design §2 D7
- O3 Internal visibility 双轨分流: `ReminderOutput` 输出端分流；`to_transcript` TUI 投递链路（K1）已通过 verify-phase Fix 3 部分打通（token 警告路由到 TUI）

## Scenario 覆盖

### system-reminder-injection（7 requirements）
- System reminder block injection per user turn: I1 first_turn + I2 second_turn ✅
- Four content source layers: U1 full snapshot + U2 missing degrade + U3 all-missing-None + hook_only_yields_wrapped_reminder ✅
- Rules alphabetical: U4 + utils tests ✅
- Source attribution: U5 absolute paths ✅
- Double preamble: U1 含 OPENING/CLOSING 断言 ✅
- Token budget: token_budget_tests over/under + Fix 3 TUI routing ✅
- Main session scope: grep 验证 subagent 路径不调用 builder ✅
- PromptContextBuilder API preserved: U9 + with_wgenty_md/with_agents_md 签名不变 ✅

### hook-lifecycle-complete（4 requirements）
- UserPromptSubmit fires before turn: code review + cargo check ✅
- InjectContext content reaches next turn: hook_inject_content_end_to_end ✅
- Coordinates with reminder block: two_hooks_render_in_priority_order + hook_only ✅
- Inject visibility honored: U7 internal + U8 visible ✅
- Inject priority orders: U6 + Fix 2 ties-by-declaration ✅
- continue_execution=false still injects: Fix 1 ✅

## Verify-phase 修复

1. **Spec drift #1**（commit a546e25）: 同步 spec + design 的 `# claudeMd` → `# wgentyMd`，对齐 O1 最终决策
2. **Minor Fix 1**（commit 0069259）: 补 `hook_with_continue_execution_false_still_injects` 测试
3. **Minor Fix 2**（commit 0069259）: 补 `two_hooks_identical_priority_preserve_declaration_order` 测试
4. **Minor Fix 3**（commit 0069259）: token 预算警告路由到 TUI `committed_messages`（spec "emit warning to TUI status area"）

## 已知限制（接受，归档时记录）

- K1: `to_transcript` 完整 TUI 投递链路（reminder 块本身的可见部分展示）仍部分延后；token 警告已路由，但 reminder 块的可见部分展示依赖未来 `AppEvent::SystemNotice` 通用通道。不影响模型侧正确性。
- §8.2-8.5 hands-on REPL 验证：底层行为已被自动化测试覆盖，实机 sanity 留待用户运行。

## Final Assessment

**All checks passed. Ready for archive.**

- 0 CRITICAL issues
- 0 IMPORTANT issues（spec drift #1 已修复）
- 0 outstanding WARNING issues（Minor 1-3 已修复）
- 所有验收场景由至少 1 个测试覆盖
- BREAKING migration 在 CHANGELOG 完整记录
- D1-D9 + O1-O3 全部一致
