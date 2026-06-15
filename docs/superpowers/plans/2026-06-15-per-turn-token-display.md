---
change: per-turn-token-display
design-doc: docs/superpowers/specs/2026-06-15-per-turn-token-display-design.md
base-ref: 3ff1acb68de0bdc5659c7d65f2c22f8bfc2c71d8
---

# Per-Turn Token Display 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** 在状态栏中按 Turn 分别显示 input tokens (`↑`) 和 output tokens (`↓`)，替代原有的累积 `used_tokens` 单一数值。

**Architecture:** TokenCounter 新增两个原子计数器 (`turn_input` / `turn_output`) 分别跟踪每轮输入和输出 token，在 AgentLoop 入口处 `reset_turn()`，输入估算和输出 API usage 分别累加到对应计数器，状态栏渲染时读取 turn 级计数并格式化为双向显示。

**Tech Stack:** Rust (stable), std::sync::atomic, ratatui (TUI)

---

## 文件结构

| 文件 | 职责 | 变更类型 |
|------|------|----------|
| `src/api/token_counter.rs` | TokenCounter 新增字段和方法，含单元测试 | 修改 |
| `src/tui/agent/mod.rs` | process_input 入口 reset_turn；process_input_inner 新增 add_input | 修改 |
| `src/tui/agent/core.rs` | run_agent_loop 新增 add_output | 修改 |
| `src/tui/components/status.rs` | render 签名改为 input_tokens/output_tokens；format_tokens 改双向格式 | 修改 |
| `src/tui/app/render.rs` | render_status 读取 turn_input_tokens / turn_output_tokens | 修改 |

---

### Task 1: TokenCounter 扩展 — 新增 turn 级字段与方法

**Files:**
- Modify: `src/api/token_counter.rs:12-16`（Struct 字段）
- Modify: `src/api/token_counter.rs:20-25`（构造函数）
- Modify: `src/api/token_counter.rs:29-72`（新增方法区域）
- Test: `src/api/token_counter.rs`（新增 `#[cfg(test)]` 模块）

- [x] **Step 1: 为 TokenCounter struct 新增 turn_input / turn_output 字段**

修改现有 struct：

```rust
/// Shared token usage tracker.
#[derive(Debug, Clone)]
pub struct TokenCounter {
    used: Arc<AtomicUsize>,
    budget: usize, // in thousands of tokens (0 = unlimited)

    // ── Per-turn counters (reset on each user input) ──
    turn_input: Arc<AtomicUsize>,
    turn_output: Arc<AtomicUsize>,
}
```

- [x] **Step 2: 构造函数初始化新字段为 0**

修改 `new` 方法：

```rust
pub fn new(budget_k: usize) -> Self {
    Self {
        used: Arc::new(AtomicUsize::new(0)),
        budget: budget_k * 1000,
        turn_input: Arc::new(AtomicUsize::new(0)),
        turn_output: Arc::new(AtomicUsize::new(0)),
    }
}
```

- [x] **Step 3: 实现 add_input、add_output、reset_turn 方法**

在 `impl TokenCounter` 块中，`is_exhausted` 方法之后新增：

```rust
/// Add `tokens` to the per-turn input counter.
pub fn add_input(&self, tokens: usize) {
    self.turn_input.fetch_add(tokens, Ordering::Relaxed);
}

/// Add `tokens` to the per-turn output counter.
pub fn add_output(&self, tokens: usize) {
    self.turn_output.fetch_add(tokens, Ordering::Relaxed);
}

/// Reset per-turn counters to zero (called at start of each turn).
pub fn reset_turn(&self) {
    self.turn_input.store(0, Ordering::Relaxed);
    self.turn_output.store(0, Ordering::Relaxed);
}

/// Current turn's input tokens.
pub fn turn_input_tokens(&self) -> usize {
    self.turn_input.load(Ordering::Relaxed)
}

/// Current turn's output tokens.
pub fn turn_output_tokens(&self) -> usize {
    self.turn_output.load(Ordering::Relaxed)
}
```

- [x] **Step 4: 运行编译检查**

Run: `cargo check`
Expected: 编译通过，无警告

- [x] **Step 5: 编写并运行单元测试**

在文件末尾（`impl TokenCounter` 块之后）新增：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_turn_counters_start_at_zero() {
        let counter = TokenCounter::new(10);
        assert_eq!(counter.turn_input_tokens(), 0);
        assert_eq!(counter.turn_output_tokens(), 0);
    }

    #[test]
    fn test_add_input_increments_turn_input() {
        let counter = TokenCounter::new(10);
        counter.add_input(100);
        assert_eq!(counter.turn_input_tokens(), 100);
        counter.add_input(50);
        assert_eq!(counter.turn_input_tokens(), 150);
    }

    #[test]
    fn test_add_output_increments_turn_output() {
        let counter = TokenCounter::new(10);
        counter.add_output(200);
        assert_eq!(counter.turn_output_tokens(), 200);
    }

    #[test]
    fn test_reset_turn_clears_both_counters() {
        let counter = TokenCounter::new(10);
        counter.add_input(100);
        counter.add_output(200);
        assert_eq!(counter.turn_input_tokens(), 100);
        assert_eq!(counter.turn_output_tokens(), 200);

        counter.reset_turn();
        assert_eq!(counter.turn_input_tokens(), 0);
        assert_eq!(counter.turn_output_tokens(), 0);
    }

    #[test]
    fn test_turn_counters_do_not_affect_used() {
        let counter = TokenCounter::new(10);
        counter.add_input(50);
        counter.add_output(50);
        // used_tokens should remain 0 — turn counters are independent
        assert_eq!(counter.used_tokens(), 0);
    }

    #[test]
    fn test_add_output_does_not_cross_budget() {
        // add_output is a relaxed fetch_add — it should never reject
        let counter = TokenCounter::new(1); // 1000 token budget
        counter.add_output(999);
        assert_eq!(counter.turn_output_tokens(), 999);
        counter.add_output(2); // would exceed budget, but turn counter doesn't care
        assert_eq!(counter.turn_output_tokens(), 1001);
    }
}
```

Run: `cargo test --lib -- api::token_counter::tests`
Expected: 7 passed, 0 failed

- [x] **Step 6: Commit**

```bash
git add src/api/token_counter.rs
git commit -m "feat: add per-turn token counters to TokenCounter

Add turn_input/turn_output atomic counters with add_input, add_output,
reset_turn, turn_input_tokens, and turn_output_tokens methods. These
counters are independent of the existing used/budget budget-control
path and use relaxed ordering.

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 2: AgentLoop 集成 — reset_turn / add_input / add_output

**Files:**
- Modify: `src/tui/agent/mod.rs:80-93`（process_input 入口）
- Modify: `src/tui/agent/mod.rs:114-123`（process_input_inner，用户输入推入历史前）
- Modify: `src/tui/agent/core.rs:58-77`（run_agent_loop，token 统计区域）

- [x] **Step 1: process_input 入口新增 reset_turn 调用**

修改 `src/tui/agent/mod.rs` 中 `process_input` 方法，在第 80 行函数体开头新增：

```rust
pub async fn process_input(&mut self, input: String) -> Result<(), String> {
    self.token_counter.reset_turn();  // ← 新增
    const AGENT_LOOP_TIMEOUT: Duration = Duration::from_secs(3600);
    // ... 后续不变
}
```

修改后该函数完整开头：

```rust
pub async fn process_input(&mut self, input: String) -> Result<(), String> {
    self.token_counter.reset_turn();  // reset per-turn counters for the new turn
    const AGENT_LOOP_TIMEOUT: Duration = Duration::from_secs(3600);
    // ... 后续保持不变
}
```

- [x] **Step 2: process_input_inner 中用户消息前估算并累加 input tokens**

修改 `src/tui/agent/mod.rs` 中 `process_input_inner` 方法，在 `history.push(ChatMessage::user(&input))` 之前新增：

```rust
async fn process_input_inner(&mut self, input: String) -> Result<(), String> {
    self.inject_background_results().await;

    {
        let mut history = self.conversation_history.lock().await;
        // Estimate input tokens (~4 chars per token) before pushing user message
        let input_tokens = input.len() / 4;
        self.token_counter.add_input(input_tokens);
        history.push(ChatMessage::user(&input));
    }

    self.run_agent_loop().await
}
```

- [x] **Step 3: run_agent_loop 中新增 add_output 调用**

修改 `src/tui/agent/core.rs`，在 `token_counter.add(usage.total_tokens)` 之后并行新增 `token_counter.add_output`。找到第 58-77 行的 token 统计区域，修改为：

```rust
            // Token accounting: prefer API-reported usage, fall back to character estimation.
            if let Some(ref usage) = result.usage {
                self.token_counter.add(usage.total_tokens);
                // Per-turn output tracking (display only, not budget)
                self.token_counter.add_output(usage.completion_tokens);
            } else {
                // Fallback: estimate from character count (~4 chars per token for English,
                // conservative so we don't undercount).
                let input_est: usize = messages
                    .iter()
                    .map(|m| m.content.as_deref().unwrap_or("").len())
                    .sum::<usize>()
                    / 4;
                let output_est: usize = (result.content.len()
                    + result
                        .tool_calls
                        .iter()
                        .map(|tc| tc.function.arguments.len())
                        .sum::<usize>())
                    / 4;
                self.token_counter.add(input_est + output_est);
                // Per-turn output tracking (display only)
                self.token_counter.add_output(output_est);
            }
```

注意：`add_input` 已在 Task2 Step2 中的 `process_input_inner` 处调用，不需要在 `run_agent_loop` 中重复添加。`run_agent_loop` 中的 fallback 路径已经用 `input_est` + `output_est` 调用 `add`，此处 `add_output` 只加 `output_est` 部分。

- [x] **Step 4: 编译检查并运行所有 token counter 相关测试**

Run: `cargo check`
Expected: 编译通过

Run: `cargo test --lib -- api::token_counter::tests`
Expected: 7 passed, 0 failed

- [x] **Step 5: Commit**

```bash
git add src/tui/agent/mod.rs src/tui/agent/core.rs
git commit -m "feat: integrate per-turn token tracking into AgentLoop

Reset turn counters at process_input entry, estimate input tokens
before pushing user message, and record per-turn output tokens from
API usage completion_tokens (with fallback estimation).

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 3: 状态栏渲染 — 双向 token 展示

**Files:**
- Modify: `src/tui/components/status.rs:21-30`（render 函数签名）
- Modify: `src/tui/components/status.rs:57-76`（meta 行构建逻辑）
- Modify: `src/tui/components/status.rs:168-174`（format_tokens 函数）

- [x] **Step 1: 修改 render 函数签名**

将 `tokens_used: usize` 替换为 `input_tokens: usize, output_tokens: usize`：

```rust
pub fn render(
    f: &mut Frame,
    area: Rect,
    phase: &AgentPhase,
    spinner_frame: u8,
    elapsed_secs: Option<u64>,
    input_tokens: usize,
    output_tokens: usize,
    mode_label: &str,
    subagent_tree: Option<&SubagentTree>,
) {
```

- [x] **Step 2: 修改 meta 行构建逻辑 — 使用 format_turn_tokens 替代 format_tokens**

将第 63-66 行：

```rust
    if tokens_used > 0 {
        meta_parts.push(format_tokens(tokens_used));
    }
```

替换为：

```rust
    let turn_token_str = format_turn_tokens(input_tokens, output_tokens);
    if !turn_token_str.is_empty() {
        meta_parts.push(turn_token_str);
    }
```

- [x] **Step 3: 保留原有 format_tokens、新增 format_turn_tokens**

将原有的 `format_tokens` 函数（第 168-174 行）替换为两个函数：

```rust
/// Format a single token count with k-suffix (e.g. "1.6k").
fn fmt_single(n: usize) -> String {
    if n >= 1000 {
        format!("{:.1}k", n as f64 / 1000.0)
    } else {
        format!("{} tokens", n)
    }
}

/// Format per-turn token display: "↑ N · ↓ Mk"
/// Omits input part if zero, omits output part if zero.
fn format_turn_tokens(input: usize, output: usize) -> String {
    let mut parts: Vec<String> = Vec::new();
    if input > 0 {
        parts.push(format!("↑ {}", fmt_single(input)));
    }
    if output > 0 {
        parts.push(format!("↓ {}", fmt_single(output)));
    }
    parts.join(" · ")
}
```

- [x] **Step 4: 编译检查**

Run: `cargo check`
Expected: 编译通过

注意：此时 `render_status` 中还在传 `used_tokens()`，会导致编译错误，这是预期中的。下一个 Task 会修复调用方。

- [x] **Step 5: Commit**

```bash
git add src/tui/components/status.rs
git commit -m "refactor: update status render to accept per-turn tokens

Replace single tokens_used param with input_tokens/output_tokens pair.
Add format_turn_tokens with ↑/↓ notation (e.g. ↑ 25 · ↓ 1.6k).
Hide tokens section entirely when both are zero.

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 4: App 渲染适配 — render_status 改用 turn 级计数器

**Files:**
- Modify: `src/tui/app/render.rs:114-128`（render_status 方法）

- [x] **Step 1: 修改 render_status 方法**

将：

```rust
    fn render_status(&self, f: &mut Frame, area: Rect) {
        let elapsed = self.turn_started_at.map(|t| t.elapsed().as_secs());
        let tokens = self.token_counter.used_tokens();
        let mode = self.mode.label();
        components::status::render(
            f,
            area,
            &self.phase,
            self.spinner_frame,
            elapsed,
            tokens,
            mode,
            Some(&self.subagent_tree),
        );
    }
```

修改为：

```rust
    fn render_status(&self, f: &mut Frame, area: Rect) {
        let elapsed = self.turn_started_at.map(|t| t.elapsed().as_secs());
        let input_tokens = self.token_counter.turn_input_tokens();
        let output_tokens = self.token_counter.turn_output_tokens();
        let mode = self.mode.label();
        components::status::render(
            f,
            area,
            &self.phase,
            self.spinner_frame,
            elapsed,
            input_tokens,
            output_tokens,
            mode,
            Some(&self.subagent_tree),
        );
    }
```

- [x] **Step 2: 编译检查并运行全量测试**

Run: `cargo check`
Expected: 编译通过，无警告

Run: `cargo test`
Expected: 全部测试通过

- [x] **Step 3: 最终提交**

```bash
git add src/tui/app/render.rs
git commit -m "feat: wire per-turn token display in App render_status

Replace used_tokens() call with turn_input_tokens() and
turn_output_tokens() for the status bar, enabling the new
bidirectional token display per turn.

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## 自检清单

### 1. Spec 覆盖
- TokenCounter 扩展：Task 1 覆盖 —— 新增 turn_input/turn_output 字段及 5 个方法
- AgentLoop reset_turn：Task 2.1 覆盖 —— process_input 入口调用
- AgentLoop 输入估算：Task 2.2 覆盖 —— process_input_inner 中 input.len()/4 估算并 add_input
- AgentLoop 输出累加：Task 2.3 覆盖 —— run_agent_loop 中 add_output(usage.completion_tokens) / 降级估算
- 状态栏渲染签名变更：Task 3.1 覆盖 —— tokens_used → (input_tokens, output_tokens)
- 双向格式展示：Task 3.2-3.3 覆盖 —— format_turn_tokens 用 ↑/↓ 格式
- App 渲染适配：Task 4.1 覆盖 —— render_status 读取 turn 级计数器

### 2. 占位符检查
- 所有步骤包含完整代码块，无 "TBD"、"TODO"、"implement later" 等占位符
- 所有命令包含预期的输入/输出说明

### 3. 类型一致性
- TokenCounter 方法名：add_input / add_output / reset_turn / turn_input_tokens / turn_output_tokens —— 各 task 间完全一致
- status::render 签名：第 3 参数从 `tokens_used: usize` 变为 `input_tokens: usize, output_tokens: usize` —— 调用方（render.rs）和定义方（status.rs）一致更新
- `format_turn_tokens` 在 status.rs 中定义和使用，无外部引用
