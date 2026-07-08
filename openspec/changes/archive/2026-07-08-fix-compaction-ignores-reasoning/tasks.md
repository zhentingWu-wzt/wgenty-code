## 1. 扩展 needs_compaction 的字符统计

- [x] 1.1 在 `src/tui/agent/compaction.rs` 的 `needs_compaction` 中，将 `total_chars` 累加扩展为 `content + reasoning_content + tool_calls.arguments`（`reasoning_content` 与 `tool_calls` 均为 `Option`，需过滤空值；`tool_calls` 需遍历各 `function.arguments`）。
- [x] 1.2 更新 `needs_compaction` 单元测试：构造含 `reasoning_content` 与 `tool_calls` 的 history，断言这三部分都被计入触发判定。

## 2. 下调 MAX_ESTIMATED_TOKENS 阈值

- [x] 2.1 在 `src/tui/agent/mod.rs` 将 `pub(super) const MAX_ESTIMATED_TOKENS: usize = 800_000;` 改为 `80_000`，并更新行内注释说明取值依据（≈128K 窗口、留 ~33K 余量、计入 reasoning+tool_calls）。

## 3. 验证

- [x] 3.1 `cargo fmt` 无 diff。
- [x] 3.2 `cargo clippy -D warnings` 无警告。
- [x] 3.3 `cargo test` compaction 相关测试通过。
- [x] 3.4 `cargo build --release` 成功。
