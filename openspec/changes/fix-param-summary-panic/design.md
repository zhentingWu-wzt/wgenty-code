## 修复方案

### 方案：字符边界安全截断

将 `extract_params_summary` 第 81 行

```rust
format!("{}…", &s[..MAX_PARAMS_SUMMARY_LEN])
```

改为

```rust
format!("{}…", &s[..s.floor_char_boundary(MAX_PARAMS_SUMMARY_LEN)])
```

`str::floor_char_boundary`（Rust 1.80+ 稳定）返回 `≤ index` 的最大字符边界下标，避免在多字节字符中间切片。若工具链低于 1.80，改用手动回退：

```rust
let mut end = MAX_PARAMS_SUMMARY_LEN;
while end > 0 && !s.is_char_boundary(end) {
    end -= 1;
}
format!("{}…", &s[..end])
```

### 取舍

- **为何 `floor_char_boundary` 而非 `chars().take(N)`**：保留"字节预算 ≤ 80"的语义——参数摘要上限是字节长度（控制日志/显示宽度），仅下取整到字符边界。`chars().take(80)` 会取 80 个字符，多字节时实际字节数远超 80，违背上限初衷。
- **为何不调 `MAX_PARAMS_SUMMARY_LEN`**：80 字节上限是既有设计意图，本次只修 panic，不改预算。
- **为何不加 `char_indices` 手算**：`floor_char_boundary` 是标准库提供的精确语义，更清晰且零依赖。

### 验证

- 新增单元测试：构造含多字节字符（如中文）且第 80 字节落在字符内部的字符串，断言 `extract_params_summary` 返回截断到字符边界的结果，不 panic。
- `cargo build` / `cargo clippy --lib -- -D warnings` / `cargo test --lib` 通过。
- 根因消除：`subagent_loop.rs` 不再存在无字符边界检查的 `&s[..MAX_PARAMS_SUMMARY_LEN]` 切片。
