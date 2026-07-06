## 1. 修复字符边界安全截断

- [x] 1.1 在 `src/teams/subagent_loop.rs` `extract_params_summary` 将 `&s[..MAX_PARAMS_SUMMARY_LEN]` 改为 `&s[..s.floor_char_boundary(MAX_PARAMS_SUMMARY_LEN)]`（或等价手动回退，依工具链版本）。
- [x] 1.2 新增单元测试覆盖多字节字符截断（第 80 字节落在字符内部，不 panic）。

## 2. 构建与验证

- [x] 2.1 `cargo build` 通过。
- [x] 2.2 `cargo clippy --lib -- -D warnings` 零 warning。
- [x] 2.3 `cargo test --lib` 相关测试通过（515 passed, 0 failed；含 2 个新增测试）。
- [x] 2.4 根因消除检查：`subagent_loop.rs` 不再存在无边界检查的 `&s[..MAX_PARAMS_SUMMARY_LEN]` 字节切片。
