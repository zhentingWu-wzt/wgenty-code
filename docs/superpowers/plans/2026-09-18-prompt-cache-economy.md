---
change: prompt-cache-economy
design-doc: docs/superpowers/specs/2026-09-18-prompt-cache-economy-design.md
base-ref: ab0a20d3cdccdd282b1486d3df90b747e23e0825
---

# Implementation Plan: prompt-cache-economy

> 降级说明：subagent 计划生成失败，本计划由主会话依据 design doc + tasks.md 内联补写（comet-build Step 1 降级回退路径）。
> 进度注记：任务组 2 的 cached_tokens 贯通（含 StatusBar 徽标扩展）已先行实现并提交 `f182f64c`。

## 1. 记忆召回迁移（跨 turn 缓存解锁）— 改动收敛在 src/prompts/mod.rs

- [x] 1.1 `assemble_instructions`：删除 Layer 5b `<relevant_memories>` 块（Layer 5c global 保留）
- [x] 1.2 `build_user_turn_reminder`：新增 memories section（顺序 hooks → memories），复用 `ReminderOutput{to_model,to_transcript}`
- [x] 1.3 `dump_user_turn_reminder` "hook-only" 文案更新
- [x] 1.4 测试：连续两 turn 不同召回 → system 消息逐字节一致；无召回无 hook → reminder None；section 顺序断言
- [x] 1.5 验证：`cargo test --lib prompts` + `cargo test --lib daemon` + TUI 相关

## 2. cached_tokens 贯通 — ✅ 已完成（f182f64c）

- [x] 2.1 `Usage.prompt_tokens_details.cached_tokens`（OpenAI）+ Anthropic `cache_read_input_tokens` 双站点映射
- [x] 2.2 TokenCounter.last_cached_tokens + UsageUpdate 事件 + TurnContext usage JSON
- [x] 2.3 web sessionStore.cachedTokens + StatusBar `cache NN%` 徽标（null 隐藏）+ Inspector 面板（数字版后续小片）
- [x] 2.4 Rust 解析测试（有值/缺省）+ sessionRunner cached/clear/missing 三态测试
- [x] 2.5 `cargo test --lib api|runtime|daemon` + web typecheck/vitest 全绿

## 3. reasoning_content 剥离（默认剥离 + DeepSeek 例外）

- [x] 3.1 `src/api/mod.rs`：`strip_reasoning_content_for_replay(messages, provider)`，挂 `sanitize_tool_call_args_for_replay` 同一序列；settings `models.transport.strip_reasoning_content`（Option<bool>）兜底
- [x] 3.2 测试：GLM(OpenAI 兼容) 请求 JSON 无该字段；DeepSeek 保留；settings 覆盖生效
- [x] 3.3 验证：`cargo test --lib api` + clippy + fmt

## 4. 收尾

- [ ] 4.1 全量：`cargo test --lib`、web typecheck + vitest
- [ ] 4.2 CHANGELOG Unreleased 条目；勾选 tasks.md；comet-build 退出守卫

## 验证命令（每任务组后运行）

```bash
cargo fmt -- --check && cargo clippy --all-targets -- -D warnings
cargo test --lib
cd web && npm run typecheck && npm test
```
