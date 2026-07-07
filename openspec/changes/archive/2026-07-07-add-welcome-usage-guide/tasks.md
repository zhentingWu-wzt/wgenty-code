## 1. 添加使用指南文本

- [x] 1.1 在 `src/tui/components/welcome.rs` 的 "Model: {model_name}" 行后追加：空行 + Comet 工作流特色行（`Comet spec-driven workflow · open → design → build → verify → archive`，淡紫色 `Color::Rgb(160, 140, 200)`）+ 空行 + 交互提示行 + 命令速览行（暗灰色 `Color::Rgb(120, 120, 140)`）。
- [x] 1.2 将 `Layout` 约束 `Constraint::Length(11)` 调整为 `Constraint::Length(16)` 以容纳新增行。

## 2. 构建与验证

- [x] 2.1 `cargo build` 通过。
- [x] 2.2 `cargo clippy -- -D warnings` 零 warning。
- [x] 2.3 `cargo fmt --check` 通过。
