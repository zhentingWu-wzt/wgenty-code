# Task 2: API 用量记录 — prompt_tokens 写入 TokenCounter

Status: DONE
Summary: 在 `core.rs` 的 token accounting 块中，`add_output` 之后添加 `set_prompt_tokens(usage.prompt_tokens)`，将 API 报告的 `prompt_tokens` 写入 `TokenCounter`。

RED: 无新测试，此修改由编译验证
GREEN: `cargo build` — 编译成功，0 errors
`cargo clippy -- -D warnings` — zero warnings

Commit: 8c3272d30b0b23e08067080cdc6b7298ee73709f
Changed files: src/tui/agent/core.rs

Concerns: 无。
