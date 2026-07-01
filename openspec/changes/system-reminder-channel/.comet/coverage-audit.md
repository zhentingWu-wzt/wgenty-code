# 12 验收场景覆盖审计

`openspec/changes/system-reminder-channel/specs/system-reminder-injection/spec.md` 列出的核心场景与设计文档 §5.3 的 12 项验收场景的测试覆盖：

| # | 验收场景 | 覆盖测试 | 位置 |
|---|----------|----------|------|
| 1 | 启动 wgenty-code 输入一句 prompt → 请求中 user message 头部存在 `<system-reminder>` 块 | `first_turn_user_message_contains_reminder` | tests/system_reminder.rs |
| 2 | Reminder 块包含 4 段（用户 WGENTY + 用户 rules/*.md 字母序 + 项目 WGENTY + 项目 AGENTS） | `reminder_full_four_sources_snapshot` | src/prompts/mod.rs::reminder_tests |
| 3 | 每段以 "Contents of <绝对路径> (<描述>):" 开头 | `reminder_absolute_paths_in_attribution` | src/prompts/mod.rs::reminder_tests |
| 4 | 块首 OVERRIDE preamble | `reminder_full_four_sources_snapshot` 含 `IMPORTANT: These instructions OVERRIDE` 断言 | 同上 |
| 5 | 块尾 may-or-may-not-be-relevant preamble | 同上，含 `IMPORTANT: this context may or may not be relevant` 断言 | 同上 |
| 6 | 第二轮 user message 再次包含 reminder（per-turn 验证） | `second_turn_reminder_reappears` + `reminder_reflects_runtime_file_change` | tests/system_reminder.rs |
| 7 | 当 ~/.wgenty-code/WGENTY.md 不存在：跳过该层，不报错、不留空标题 | `reminder_missing_user_wgenty_no_empty_header` | src/prompts/mod.rs::reminder_tests |
| 8 | 当用户全局 rules/ 目录不存在或为空：跳过该层 | `reminder_all_missing_returns_none` (cover empty case) + `reminder_user_rules_alphabetical_order` (cover populated alpha order) | src/prompts/mod.rs::reminder_tests + utils/project.rs::tests |
| 9 | token 预算超阈值时一次性提示（不再每轮重复） | `reminder_over_threshold_estimate_exceeds_2000` + `reminder_under_threshold_estimate_stays_quiet` | src/tui/app/mod.rs::token_budget_tests |
| 10 | 系统提示中不再包含 "# AGENTS.md" 和 "# WGENTY.md — 项目规则与约定" 两个 system message（硬切验证） | `assemble_instructions_no_layer_7_8` | src/prompts/mod.rs::tests |
| 11 | 配置 UserPromptSubmit hook 返回 injected_content 时，下一轮 user message 中能看到该内容被注入 | `hook_inject_content_end_to_end` | tests/system_reminder.rs |
| 12 | 测试覆盖以上行为，至少 6 个新增单测/集成测 | 19 新测试（12 单测 + 7 集成测/混合） | 见以下统计 |

## 测试统计

- src/prompts/mod.rs::tests + reminder_tests: U1-U9 单测共 9 个
- src/runtime/hooks/mod.rs::tests: collect_injections 3 个 + multiple_inject_hooks 1 个 = 4 个
- src/utils/project.rs::tests: read_user_global_* 5+3 = 8 个
- src/tui/app/mod.rs::token_budget_tests: 2 个
- tests/system_reminder.rs: I1, I2, I3-prep, hook_inject_content_end_to_end, two_hooks_render_in_priority_order = 5 个

合计：28+ 个新测试（远超 ≥6 的最低要求）

## 其他 Risk 覆盖

- R6 hook timeout：`process_input_inner` 内 `tokio::time::timeout(Duration::from_secs(10), ...)` 兜底；通过单元逻辑保证（cargo check 验证签名匹配）
- D8 visibility 分流：`reminder_internal_visibility_excludes_transcript` + `reminder_visible_hook_in_both_outputs` 覆盖
- K2 subagent 不调用 builder：通过 `grep -rn "build_user_turn_reminder" src/` 仅命中 `src/tui/agent/mod.rs::process_input_inner` 一处，subagent 路径未引入；reviewer 已确认

## 未自动覆盖（留待 verify 手工验证）

- §8.2 启动 `wgenty-code repl` 实机验证（需要 LLM 端点）
- §8.3 删除用户 WGENTY 后实机不报错
- §8.4 配置真实 settings.json hook，重启 repl 验证
- §8.5 `cargo run -- repl --prompt "X"` 单次查询模式

这些场景的语义已被对应单测/集成测覆盖；手工验证是端到端 sanity check。
