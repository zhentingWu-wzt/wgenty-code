# Task 3: 上下文窗口配置 — context_window 字段

Status: DONE
Summary: 在 `src/config/models.rs` 的 `ModelsConfig` struct 中新增 `context_window: usize` 字段，默认值 200_000，带 `#[serde(default = "default_context_window")]` 保证缺失字段时反序列化兼容。

RED: `cargo test models` — 编译失败（no field `context_window`），预期 2 errors
GREEN: `cargo test models` — 4 passed, 0 failed
`cargo clippy -- -D warnings` — zero warnings

Commit: 4b123b1
Changed files: src/config/models.rs

Test summary:
- `test_default_context_window` — `ModelsConfig::default().context_window == 200_000` (PASS)
- `test_context_window_deserialize_default` — 反序列化不含 `context_window` 的 JSON，默认值为 200_000 (PASS)
