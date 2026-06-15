---
comet_change: per-turn-token-display
role: technical-design
canonical_spec: openspec
archived-with: 2026-06-15-per-turn-token-display
status: final
---

# Per-Turn Token Display — 技术设计

## 1. TokenCounter 扩展

在 `src/api/token_counter.rs` 中，`TokenCounter` 新增两个用于状态栏展示的原子计数器：

```rust
pub struct TokenCounter {
    // ── 预算控制（不变）──
    used: Arc<AtomicUsize>,
    budget: usize,

    // ── 状态栏展示（新增）──
    turn_input: Arc<AtomicUsize>,
    turn_output: Arc<AtomicUsize>,
}
```

新增方法签名（全部使用 `Ordering::Relaxed`，无需与 `used` 的 CAS 同步）：

| 方法 | 行为 |
|------|------|
| `add_input(tokens: usize)` | `turn_input.fetch_add(tokens, Relaxed)` |
| `add_output(tokens: usize)` | `turn_output.fetch_add(tokens, Relaxed)` |
| `reset_turn()` | 将 turn_input、turn_output 归零 |
| `turn_input_tokens() -> usize` | `turn_input.load(Relaxed)` |
| `turn_output_tokens() -> usize` | `turn_output.load(Relaxed)` |

构造函数中新增字段初始化为 0。

## 2. AgentLoop 集成

### 2.1 Turn 重置

`process_input` 入口（`src/tui/agent/mod.rs` 第 80 行）新增：

```rust
pub async fn process_input(&mut self, input: String) -> Result<(), String> {
    self.token_counter.reset_turn();  // ← 新增
    // ... 原有逻辑
}
```

### 2.2 用户输入估算

`process_input_inner`（`src/tui/agent/mod.rs` 第 114 行）中，在 `history.push(ChatMessage::user(&input))` 之前新增：

```rust
let input_tokens = input.len() / 4;
self.token_counter.add_input(input_tokens);
```

### 2.3 输出 token 累加

`run_agent_loop`（`src/tui/agent/core.rs` 第 59-77 行）中，在现有 `token_counter.add(usage.total_tokens)` 之后新增：

```rust
// 原有：预算控制（不动）
self.token_counter.add(usage.total_tokens);

// 新增：状态栏展示
// 有 API usage 时用 completion_tokens
if let Some(ref usage) = result.usage {
    self.token_counter.add_output(usage.completion_tokens);
} else {
    // 降级：用 chars/4 估算 output
    let output_est = (result.content.len()
        + result.tool_calls.iter().map(|tc| tc.function.arguments.len()).sum::<usize>()) / 4;
    self.token_counter.add_output(output_est);
}
```

## 3. 状态栏渲染

### 3.1 签名变更

`status::render`（`src/tui/components/status.rs`）签名：

```rust
// Before
pub fn render(f, area, phase, spinner_frame, elapsed_secs, tokens_used, mode_label, subagent_tree)

// After
pub fn render(f, area, phase, spinner_frame, elapsed_secs, input_tokens, output_tokens, mode_label, subagent_tree)
```

### 3.2 显示逻辑

```rust
fn format_turn_tokens(input: usize, output: usize) -> String {
    fn fmt_single(n: usize) -> String {
        if n >= 1000 {
            format!("{:.1}k", n as f64 / 1000.0)
        } else {
            format!("{} tokens", n)
        }
    }
    let mut parts = Vec::new();
    if input > 0 {
        parts.push(format!("↑ {}", fmt_single(input)));
    }
    if output > 0 {
        parts.push(format!("↓ {}", fmt_single(output)));
    }
    parts.join(" · ")
}
```

meta 行示例：

```
(5s · ↑ 25 · ↓ 1.6k · NORMAL)     ← 有 input + output
(5s · ↓ 1.6k)                      ← 无 input
(5s · NORMAL)                      ← 都为 0，只显示模式
```

### 3.3 App 渲染适配

`App::render_status`（`src/tui/app/render.rs`）中：

```rust
// Before
let tokens = self.token_counter.used_tokens();

// After
let input_tokens = self.token_counter.turn_input_tokens();
let output_tokens = self.token_counter.turn_output_tokens();
```

## 4. 文件变更清单

| 文件 | 变更内容 |
|------|----------|
| `src/api/token_counter.rs` | 新增 turn_input/turn_output 字段及 5 个方法；构造函数适配 |
| `src/tui/agent/mod.rs` | process_input 新增 reset_turn；process_input_inner 新增 add_input |
| `src/tui/agent/core.rs` | run_agent_loop 新增 add_output（与 add 并行）|
| `src/tui/components/status.rs` | render 签名变；format_tokens 改为双向展示 |
| `src/tui/app/render.rs` | render_status 读取 turn_input/turn_output |
