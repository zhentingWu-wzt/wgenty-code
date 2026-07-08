---
change: tui-context-usage-indicator
design-doc: docs/superpowers/specs/2026-06-22-tui-context-usage-indicator-design.md
base-ref: 8c47a27c1eabcc1d2c7ccd809414e992b797dfbf
---

# 实施计划：TUI 上下文用量指示器

## 概述

在 TUI 模式标签栏右侧添加上下文占比进度条，使用 API 报告的 `prompt_tokens` 除以可配置的上下文窗口上限（默认 200000），以 `▓▓▓░░░░░░ 32%` 形式显示，颜色随使用率阈值变化（绿 <50%、黄 50-80%、红 >80%）。

**设计文档**: `docs/superpowers/specs/2026-06-22-tui-context-usage-indicator-design.md`

**任务边界**: `openspec/changes/tui-context-usage-indicator/tasks.md`

---

## 任务 1：TokenCounter 扩展

**目标**: 为 `TokenCounter` 新增 `last_prompt_tokens` 字段及其读写方法。

**文件**: `src/api/token_counter.rs`

### 步骤

- [x] 1.1 新增 `last_prompt_tokens: Arc<AtomicUsize>` 字段到 `TokenCounter` struct，在 `new()` 中初始化为 0
- [x] 1.2 实现 `set_prompt_tokens(&self, tokens: usize)` 方法，使用 `store(tokens, Ordering::Relaxed)`
- [x] 1.3 实现 `last_prompt_tokens(&self) -> usize` 方法，使用 `load(Ordering::Relaxed)`
- [x] 1.4 添加单元测试：`set_prompt_tokens` 正确存储值，`last_prompt_tokens` 正确读取，多次 set 覆盖旧值

### 验收标准

- `cargo test token_counter` 全部通过
- `cargo clippy -- -D warnings` 零 warning
- 现有 `used`/`turn_input`/`turn_output` 字段和方法不受影响

---

## 任务 2：API 用量记录

**目标**: 在 agent core 的 token accounting 处记录 `usage.prompt_tokens`。

**文件**: `src/tui/agent/core.rs`

### 步骤

- [x] 2.1 在 `core.rs:131-134` 的 `if let Some(ref usage) = result.usage` 块内，新增 `self.token_counter.set_prompt_tokens(usage.prompt_tokens);`
- [x] 2.2 确认 fallback 路径（无 usage 时）不设置 prompt_tokens，保持上次值或 0

### 验收标准

- `cargo build` 编译通过
- `cargo clippy -- -D warnings` 零 warning
- 现有 `add(usage.total_tokens)` 和 `add_output(usage.completion_tokens)` 调用不变

---

## 任务 3：上下文窗口配置

**目标**: 在 `ModelsConfig` 中新增 `context_window` 字段，默认 200000。

**文件**: `src/config/models.rs`

### 步骤

- [x] 3.1 在 `ModelsConfig` struct 新增 `#[serde(default = "default_context_window")] pub context_window: usize` 字段
- [x] 3.2 添加 `fn default_context_window() -> usize { 200_000 }` 函数
- [x] 3.3 在 `Default for ModelsConfig` impl 中设置 `context_window: 200_000`
- [x] 3.4 添加测试：`ModelsConfig::default().context_window == 200_000`，反序列化缺失字段时使用默认值

### 验收标准

- `cargo test models` 通过
- `cargo clippy -- -D warnings` 零 warning
- 现有 settings.json 反序列化兼容（缺失 context_window 字段时使用默认值）

---

## 任务 4：进度条组件

**目标**: 新建 `context_bar.rs` 组件，渲染 Unicode 进度条 + 百分比，颜色随阈值变化。

**文件**: `src/tui/components/context_bar.rs`（新建），`src/tui/components/mod.rs`

### 步骤

- [x] 4.1 新建 `src/tui/components/context_bar.rs`，实现 `pub fn spans(used: usize, max: usize) -> Vec<Span<'static>>`
- [x] 4.2 渲染 8 格进度条：`▓`（填充）+ `░`（空），计算 `filled = (ratio * 8).round()`
- [x] 4.3 百分比文字：`format!(" {} {}%", bar, pct)`
- [x] 4.4 实现 `fn color_for_ratio(ratio: f64) -> Color`：绿 <0.5，黄 0.5-0.8，红 ≥0.8
- [x] 4.5 在 `src/tui/components/mod.rs` 添加 `pub mod context_bar;`
- [x] 4.6 添加测试：颜色阈值边界（ratio 0.49->green, 0.50->yellow, 0.79->yellow, 0.80->red），0% 显示

### 验收标准

- `cargo test context_bar` 通过
- `cargo clippy -- -D warnings` 零 warning
- 进度条宽度固定 8 格，不随窗口大小变化

---

## 任务 5：模式标签栏集成

**目标**: 修改 `render_mode_label` 在模式标签右侧渲染上下文进度条。

**文件**: `src/tui/app/render.rs`

### 步骤

- [ ] 5.1 修改 `render_mode_label`：从单个 `Paragraph` 改为 `Line::from(vec![Span...])`
- [ ] 5.2 构建 spans：`[mode_label_span, spacer_span, context_bar::spans(used, max)...]`
- [ ] 5.3 从 `self.token_counter.last_prompt_tokens()` 获取 used
- [ ] 5.4 从 `self.settings_lock.read().unwrap().models.context_window` 获取 max
- [ ] 5.5 窄终端（`area.width < 40`）时仅渲染模式标签，跳过进度条

### 验收标准

- `cargo build` 编译通过
- `cargo clippy -- -D warnings` 零 warning
- `cargo run -- repl` 启动后模式标签栏显示进度条
- 窄终端时进度条自动隐藏

---

## 任务 6：集成测试与验证

**目标**: 全量构建和测试验证。

### 步骤

- [ ] 6.1 运行 `cargo test --all` 全部通过
- [ ] 6.2 运行 `cargo fmt -- --check` 格式检查
- [ ] 6.3 运行 `cargo clippy --all-targets -- -D warnings` 零 warning
- [ ] 6.4 运行 `cargo build --release` release 构建通过

### 验收标准

- 所有测试通过
- 零 warning
- Release 构建成功
