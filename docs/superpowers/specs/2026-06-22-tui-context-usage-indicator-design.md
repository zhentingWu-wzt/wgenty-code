---
comet_change: tui-context-usage-indicator
role: technical-design
canonical_spec: openspec
archived-with: 2026-07-08-tui-context-usage-indicator
status: final
---

# Design: TUI Context Usage Indicator

## Problem

当前 TUI 输入框下方的模式标签栏（`NORMAL`/`PLAN`/`ACCEPT EDIT`/`YOLO`）仅显示当前模式，用户无法直观感知当前对话消耗了多少上下文窗口。随着对话增长，用户可能在不知情的情况下接近上下文上限，导致模型"遗忘"早期内容或触发自动压缩。

## Solution

在模式标签栏右侧添加上下文占比进度条，使用 API 报告的 `prompt_tokens` 作为当前上下文大小，除以可配置的上下文窗口上限（默认 200000），以 `▓▓▓░░░░░░ 32%` 形式显示，颜色随使用率阈值变化。

## Architecture

```
Data Layer:      TokenCounter.last_prompt_tokens (AtomicUsize)
                 ↕ Arc shared between App and AgentLoop
Recording Layer: agent/core.rs:131  set_prompt_tokens(usage.prompt_tokens)
                 ↕
Config Layer:    ModelsConfig.context_window (serde default 200000)
                 ↕
Render Layer:    context_bar.rs (new) → render_mode_label (extended)
```

### Data Flow

1. 用户发送消息 → `AgentLoop::run_agent_loop` 调用 API
2. API 响应包含 `Usage { prompt_tokens, completion_tokens, total_tokens }`
3. `core.rs:131` 的 `if let Some(ref usage) = result.usage` 块内，调用 `token_counter.set_prompt_tokens(usage.prompt_tokens)`
4. TUI 渲染时，`render_mode_label` 读取 `token_counter.last_prompt_tokens()` 和 `settings.models.context_window`
5. `context_bar::render` 计算百分比并渲染进度条

## Components

### 1. TokenCounter Extension (`src/api/token_counter.rs`)

新增字段：
```rust
last_prompt_tokens: Arc<AtomicUsize>,
```

新增方法：
```rust
/// Record the prompt_tokens from the latest API response.
pub fn set_prompt_tokens(&self, tokens: usize) {
    self.last_prompt_tokens.store(tokens, Ordering::Relaxed);
}

/// Get the prompt_tokens from the latest API response.
pub fn last_prompt_tokens(&self) -> usize {
    self.last_prompt_tokens.load(Ordering::Relaxed)
}
```

现有 `used`/`turn_input`/`turn_output` 字段和方法不变。

### 2. API Usage Recording (`src/tui/agent/core.rs`)

在 `core.rs:131-134` 的 token accounting 块中增加一行：

```rust
if let Some(ref usage) = result.usage {
    self.token_counter.add(usage.total_tokens);
    self.token_counter.add_output(usage.completion_tokens);
    self.token_counter.set_prompt_tokens(usage.prompt_tokens);  // NEW
} else {
    // Fallback estimation unchanged
}
```

Fallback 路径不设置 `prompt_tokens`（无法估算准确值，保持上次值或 0）。

### 3. Config Extension (`src/config/models.rs`)

`ModelsConfig` 新增字段：

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelsConfig {
    #[serde(default)]
    pub transport: TransportConfig,
    pub main: ModelEndpoint,
    #[serde(default)]
    pub small: Option<ModelEndpoint>,
    #[serde(default)]
    pub planner: Option<ModelEndpoint>,
    #[serde(default = "default_context_window")]
    pub context_window: usize,  // NEW
}

fn default_context_window() -> usize {
    200_000
}
```

`Default` impl 中也设置 `context_window: 200_000`。

### 4. Context Bar Component (`src/tui/components/context_bar.rs`)

```rust
const BAR_WIDTH: usize = 8;

/// Render a context usage progress bar.
/// Returns a Vec<Span> that can be combined into a Line.
pub fn spans(used: usize, max: usize) -> Vec<Span<'static>> {
    let ratio = if max == 0 { 0.0 } else { used as f64 / max as f64 };
    let pct = (ratio * 100.0).round() as usize;
    let filled = ((ratio * BAR_WIDTH as f64).round() as usize).min(BAR_WIDTH);

    let color = color_for_ratio(ratio);
    let bar: String = "▓".repeat(filled) + &"░".repeat(BAR_WIDTH - filled);
    let text = format!(" {} {}%", bar, pct);

    vec![Span::styled(text, Style::default().fg(color))]
}

fn color_for_ratio(ratio: f64) -> Color {
    if ratio < 0.5 {
        Color::Rgb(80, 220, 120)   // green
    } else if ratio < 0.8 {
        Color::Rgb(255, 200, 80)   // yellow
    } else {
        Color::Rgb(255, 90, 90)    // red
    }
}
```

### 5. Mode Label Integration (`src/tui/app/render.rs`)

`render_mode_label` 从单个 `Paragraph` 改为 `Line::from(vec![Span...])`：

```rust
fn render_mode_label(&self, f: &mut Frame, area: Rect) {
    let color = self.mode.color();
    let mode_span = Span::styled(
        format!(" {} ", self.mode.label()),
        Style::default().fg(color),
    );

    let mut spans = vec![mode_span, Span::raw(" ")];

    // Context usage bar (hidden on narrow terminals)
    if area.width >= 40 {
        let used = self.token_counter.last_prompt_tokens();
        let max = self.settings_lock.read().unwrap().models.context_window;
        spans.extend(context_bar::spans(used, max));
    }

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}
```

## Edge Cases

| 场景 | 行为 |
|------|------|
| 首次 API 调用前 (prompt_tokens=0) | 显示 `░░░░░░░░ 0%`（灰色空条） |
| 窄终端 (width < 40) | 仅显示模式标签，隐藏进度条 |
| max=0 (context_window 配置为 0) | ratio=0，显示 0% |
| 多轮工具调用 | prompt_tokens 反映累积上下文，正确更新 |
| 上下文压缩后 | prompt_tokens 降低，进度条相应缩短 |

## Test Strategy

### Unit Tests

1. **TokenCounter** (`token_counter.rs`):
   - `set_prompt_tokens` 正确存储值
   - `last_prompt_tokens` 正确读取值
   - 多次 `set_prompt_tokens` 覆盖旧值

2. **Context bar color thresholds** (`context_bar.rs`):
   - ratio 0.49 -> green
   - ratio 0.50 -> yellow
   - ratio 0.79 -> yellow
   - ratio 0.80 -> red
   - ratio 0.0 -> green, 0% display

3. **Config default** (`models.rs`):
   - `ModelsConfig::default()` 的 `context_window` == 200000
   - 反序列化缺失 `context_window` 字段时使用默认值

### Manual Verification

- 启动 TUI，发送消息，观察进度条随对话增长
- 修改 `settings.json` 的 `context_window`，验证占比变化
- 缩小终端窗口，验证窄终端时进度条隐藏

## Risks & Mitigations

| 风险 | 缓解 |
|------|------|
| `prompt_tokens` 更新延迟 | 首次调用前显示 0%，可接受 |
| `settings_lock` 锁竞争 | 读取 context_window 很快，锁持有时间极短 |
| 进度条宽度固定 8 格 | 固定宽度避免布局抖动，窄终端时隐藏 |

## Open Questions

无。所有设计决策已在 brainstorming 阶段确认。
