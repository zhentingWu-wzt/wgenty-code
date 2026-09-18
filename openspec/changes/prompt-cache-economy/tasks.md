# Tasks: prompt-cache-economy

## 1. 记忆召回迁移到 user reminder（跨 turn 缓存解锁）

- [ ] 1.1 `src/prompts/mod.rs`：`build_user_turn_reminder` 新增 memories section（`<relevant_memories>` 块，含 to_model/to_transcript 语义与 dump 支持）
- [ ] 1.2 `src/daemon/run_loop.rs`：`run_session_turn` 记忆召回结果改走 reminder 通道，移除 `prompt_ctx.memories` 写入与 Layer 5b 依赖；TurnContext 的 recalled_memories 展示保持不变
- [ ] 1.3 TUI 侧对应注入点（`src/tui` turn 注入）同步迁移，保持双端口径一致
- [ ] 1.4 测试：新增「连续两 turn system 消息逐字节一致」断言（mock 两次不同召回）；无召回时 reminder 为 None 的退化断言；既有 Layer 5b 相关测试更新
- [ ] 1.5 验证：`cargo test --lib prompts`、`cargo test --lib daemon`、TUI 相关测试

## 2. cached_tokens 贯通到 Inspector

- [ ] 2.1 `src/api/types.rs`：OpenAI usage 增加 `prompt_tokens_details.cached_tokens`（serde default）；Anthropic 转换层映射 `cache_read_input_tokens` 到统一字段
- [ ] 2.2 token_counter 增加 `last_cached_tokens`；`run_loop.rs` TurnContext usage JSON 增加 `cached_tokens` 字段
- [ ] 2.3 web：`sessionStore.ts` `TurnContextUsage` 类型 + `InspectorPanel.tsx` TokensTab 显示 `cached / prompt (xx%)`（null 时显示 "—"）；聊天 StatusBar context 条旁新增 cache 命中徽标（`cache 80%`，null 时隐藏）
- [ ] 2.4 测试：Rust 侧 usage 解析（含字段缺失缺省）+ web Inspector 显示（有值/null 两态）
- [ ] 2.5 验证：`cargo test --lib api`、`cd web && npm test`

## 3. OpenAI 兼容路径剥离 reasoning_content

- [ ] 3.1 `src/api/mod.rs`：新增 `strip_reasoning_content_for_replay(messages, provider)`，挂接到 OpenAI 兼容请求组装；DeepSeek provider 保留；settings 增加覆盖开关（默认 auto）
- [ ] 3.2 测试：GLM/OpenAI 兼容序列化输出无 `reasoning_content` 字段；DeepSeek 保留；磁盘会话消息不受影响（仅请求边界剥离）
- [ ] 3.3 验证：`cargo test --lib api`、`cargo clippy --all-targets -- -D warnings`、`cargo fmt -- --check`

## 4. 收尾

- [ ] 4.1 全量验证：`cargo test --lib`、web `typecheck` + `npm test`
- [ ] 4.2 CHANGELOG.md Unreleased 增补条目；Inspector 截图核对缓存占比展示
