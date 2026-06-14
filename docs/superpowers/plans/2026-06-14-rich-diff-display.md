---
change: rich-diff-display
design-doc: docs/superpowers/specs/2026-06-14-rich-diff-display-design.md
base-ref: 6b14e50eea76a2f47d3cf09dc96b5cf904a7ba4a
---

# Rich Diff Display Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将项目现有的裸 `+`/`-` diff 渲染替换为标准的 unified diff 格式（含 hunk 头、行号 gutter、词级变更高亮、增减统计），对标 `git diff --unified` 和 Claude Code 的 diff 展示。

**Architecture:** 在 `src/tui/components/` 下新增 `diff.rs` 模块，封装完整的 rich diff 引擎：使用 `similar::TextDiff` 进行行级和词级 diff 计算，生成结构化的 `UnifiedDiff` 数据（包含 hunks、行号、segment 词级标记），然后通过 `render_unified()` 渲染为 ratatui `Line` 列表。提供两个公开入口：`render()`（独立 Paragraph 视图，带行号 gutter）和 `diff_to_lines()`（内联聊天模式，无 gutter）。旧代码完整迁移到新模块，`chat.rs` 中删除所有内联 diff 逻辑。

**Tech Stack:** Rust, ratatui, similar (2.5, diff 引擎，已在 Cargo.toml 中存在), syntect (5.2, 可用但未集成语法高亮)

---

## 文件结构

### 修改文件

| 文件 | 改动类型 | 责任 |
|------|---------|------|
| `src/tui/components/diff.rs` | **新建** | 完整 rich diff 引擎 + 渲染（~416 行） |
| `src/tui/components/mod.rs` | 修改 | 新增 `pub mod diff;` 注册 |
| `src/tui/components/chat.rs` | 修改 | 删除旧内联 `diff_to_lines()`，改为调用 `diff::diff_to_lines()` |

### 文档文件

| 文件 | 说明 |
|------|------|
| `docs/superpowers/specs/2026-06-14-rich-diff-display-design.md` | 技术设计文档 |
| `openspec/changes/rich-diff-display/tasks.md` | OpenSpec 任务清单 |

---

### Task 1: 核心 Diff 数据结构与生成引擎

**文件:**
- Create: `src/tui/components/diff.rs:1-227`
- Test: `src/tui/components/diff.rs:349-398`

- [x] **Step 1: 定义颜色常量和配置常量**

```rust
const ADD_COLOR: Color = Color::Rgb(80, 200, 120);
const ADD_WORD_COLOR: Color = Color::Rgb(40, 255, 100);
const DEL_COLOR: Color = Color::Rgb(240, 100, 100);
const DEL_WORD_COLOR: Color = Color::Rgb(255, 70, 70);
const CTX_COLOR: Color = Color::Rgb(100, 100, 110);
const HUNK_COLOR: Color = Color::Rgb(60, 180, 180);
const HEADER_COLOR: Color = Color::Rgb(180, 180, 200);
const CONTEXT: usize = 3;
const MAX_STANDALONE: usize = 50;
const MAX_INLINE: usize = 25;
```

- [x] **Step 2: 定义核心数据结构**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineTag { Context, Delete, Insert }

struct Segment { changed: bool, text: String }
struct DiffLine { tag: LineTag, old_no: Option<usize>, new_no: Option<usize>, text: String, segments: Vec<Segment> }
struct Hunk { old_start: usize, old_count: usize, new_start: usize, new_count: usize, lines: Vec<DiffLine> }
struct UnifiedDiff { file_path: String, hunks: Vec<Hunk>, additions: usize, deletions: usize }
```

- [x] **Step 3: 实现 `generate_diff()` — 行级 diff + hunk 分组**

使用 `TextDiff::from_lines()` 和 `grouped_ops(CONTEXT)` 生成 hunks。对每个 group 提取 old_range/new_range 计算 hunk 头位置（1-based），逐行迭代 `iter_changes()` 生成 DiffLine，同步推进 old_line/new_line 计数。完成后调用 `compute_word_diffs()` 进行词级增强。

- [x] **Step 4: 实现 `compute_word_diffs()` — 词级 diff**

扫描 hunk 行，跳过 Context 行后收集连续 Delete 块和 Insert 块。对成对的 delete/insert 行调用 `TextDiff::from_words()`，通过 `segments_for_side()` 提取 changed/unchanged segment 标记。不等价的行跳过，只标记实际变更词。

- [x] **Step 5: 实现测试 — 空 diff、简单变更、纯新增/删除、词级检测、多 hunk**

```rust
#[test]
fn empty() {
    let d = generate_diff("a\nb\n", "a\nb\n", "f");
    assert!(d.hunks.is_empty());
}

#[test]
fn simple() {
    let d = generate_diff("a\nb\nc\n", "a\nX\nc\n", "f");
    assert_eq!(d.additions, 1);
    assert_eq!(d.deletions, 1);
    assert_eq!(d.hunks.len(), 1);
}

#[test]
fn add_only() { /* 验证新增行统计 */ }
#[test]
fn del_only() { /* 验证删除行统计 */ }
#[test]
fn word_parts() { /* 验证 segments 含 changed 标记 */ }
#[test]
fn multi_hunk() { /* 两处改动相距 > 2*CONTEXT，应生成 2 个 hunk */ }
```

验证结果: `cargo test -- diff` — 8 个测试全部通过。

---

### Task 2: 渲染输出 — 独立视图与内联视图

**文件:**
- Modify: `src/tui/components/diff.rs:228-345`

- [x] **Step 1: 实现行渲染 `render_line()` — 支持 gutter 切换**

`skip_gutter: bool` 参数控制是否渲染行号 gutter。Gutter 格式：`old_no marker new_no`（marker 为 ` `/`-`/`+`），全部使用 `CTX_COLOR`。行内容前缀 `  `/`- `/`+ ` 对应 Context/Delete/Insert。无 segments 时整行用 base 色，有 segments 时逐段渲染（changed 段用亮色）。超长行截断保护。

```rust
fn render_line(line: &DiffLine, gutter_w: usize, width: u16, skip_gutter: bool) -> Line<'static>
```

- [x] **Step 2: 计算 gutter 宽度**

扫描所有 hunks 的 old_no/new_no 最大值，计算数字字符串长度，格式为 `{old_digits} {marker} {new_digits} + 2`。

```rust
fn gutter_width(hunks: &[Hunk]) -> usize
```

- [x] **Step 3: 实现 hunk 头和统计行**

```rust
fn hunk_header(hunk: &Hunk) -> Line<'static>  // @@ -start,count +start,count @@
fn stats_line(path: &str, add: usize, del: usize) -> Line<'static>  // ▸ path  +N -M
```

- [x] **Step 4: 实现核心渲染函数 `render_unified()`**

```rust
fn render_unified(diff: &UnifiedDiff, width: u16, max_lines: usize, compact: bool) -> Vec<Line<'static>>
```

输出顺序：统计行 → hunks（hunk header + lines）→ 空 diff 提示。每渲染一行检查 `shown >= max_lines` 截断保护。截断时显示 `... (truncated)` 或 `... (N more lines)`。

- [x] **Step 5: 实现测试 — 渲染输出完整性、hunk 头格式**

```rust
#[test]
fn render_output() {
    let d = generate_diff("fn f() {\n  let x = 1;\n}\n", "fn f() {\n  let x = 2;\n}\n", "s.rs");
    let ls = render_unified(&d, 80, 50, true);
    assert!(ls.len() >= 5);
}

#[test]
fn hunk_fmt() {
    let d = generate_diff("a\nb\nc\n", "a\nX\nc\n", "f");
    let h = hunk_header(&d.hunks[0]);
    let t: String = h.spans.iter().map(|s| s.content.as_ref()).collect();
    assert!(t.starts_with("@@") && t.ends_with("@@"));
}
```

验证结果: `cargo test -- diff::tests::render_output diff::tests::hunk_fmt` — 通过。

---

### Task 3: 模块集成 — 注册与替换旧代码

**文件:**
- Modify: `src/tui/components/mod.rs:2`
- Modify: `src/tui/components/chat.rs:458-465`, `src/tui/components/chat.rs:601-657`

- [x] **Step 1: 在 `mod.rs` 注册新模块**

在现有 `pub mod chat;` 后添加 `pub mod diff;`。

```rust
pub mod chat;
pub mod diff;
pub mod input;
…
```

- [x] **Step 2: 导入 diff 模块**

在 `chat.rs` 顶部添加 `use crate::tui::components::diff;`。

- [x] **Step 3: 替换内联 `diff_to_lines()` 调用**

```rust
// 旧代码 (chat.rs:461-462):
let diff_lines = diff_to_lines(&diff.file_path, &diff.old_content, &diff.new_content, width);

// 新代码:
let diff_lines = diff::diff_to_lines(&diff.file_path, &diff.old_content, &diff.new_content, width);
```

- [x] **Step 4: 删除旧内联函数和颜色常量**

删除 `chat.rs` 中原有的约 80 行 `diff_to_lines()` 函数实现（长函数含旧 `iter_all_changes()` 逻辑）以及两个颜色常量 `ADD_COLOR`/`DEL_COLOR`。替换为简洁注释：

```rust
// Diff rendering is handled by `crate::tui::components::diff`.
// Use `diff::diff_to_lines()` for inline chat diff, or
// `diff::render()` for standalone diff views.
```

- [x] **Step 5: 公开 API — `render()` 和 `diff_to_lines()`**

```rust
/// Render a rich unified diff as a ratatui Paragraph in the given area.
pub fn render(f: &mut Frame, area: Rect, file_path: &str, old: &str, new: &str) -> u16

/// Convert diff data into ratatui Lines for inline rendering in chat.
pub fn diff_to_lines(file_path: &str, old: &str, new: &str, width: u16) -> Vec<Line<'static>>
```

两者内部共享 `generate_diff()` → `render_unified()` 流程，仅 `max_lines`（50 vs 25）和 `compact`（false vs true）参数不同。

---

### Task 4: 全项目测试验证

**文件:** 无代码改动

- [x] **Step 1: 运行 `cargo clippy --all-targets`**

验证无新增 warning（仅 1 个 `plugin_loading_test` 已有的 `len_zero` warning，非本变更引入）。

- [x] **Step 2: 运行 `cargo test` 全测试套件**

确认所有 155 个测试通过。验证 diff 模块 8 个专项测试通过：
- `empty` — 空 diff 场景，无 hunk
- `simple` — 1 增 1 删，单 hunk
- `add_only` — 纯新增行统计
- `del_only` — 纯删除行统计
- `word_parts` — 词级 diff segments 含 changed 标记
- `multi_hunk` — 间隔 8+ 行的两处改动生成 2 个独立 hunk
- `render_output` — 渲染输出行数验证
- `hunk_fmt` — hunk 头格式（`@@ -N,M +N,M @@`）

---

## 自检清单

**1. Spec 覆盖度**
- 数据结构 (`UnifiedDiff`, `Hunk`, `DiffLine`, `Segment`) — Task 1 已实现
- `generate_diff()` 使用 `grouped_ops(3)` + `iter_changes()` — Task 1 已实现
- 词级 diff `compute_word_diffs()` + `from_words()` — Task 1 已实现
- 独立视图（带 gutter）— Task 2 已实现
- 内联视图（无 gutter）— Task 2 已实现
- 截断保护（50/25 行）— Task 2 已实现
- 颜色常量 — Task 1 已实现，与设计文档完全一致
- 模块集成 — Task 3 已实现
- 8 个测试覆盖所有场景 — Task 1/2 已实现
- 旧代码删除 — Task 3 已实现

**2. 无占位符扫描** — 无任何 TBD/TODO/占位符，所有代码均为完整实现。

**3. 类型一致性** — `LineTag`、`Segment`、`DiffLine`、`Hunk`、`UnifiedDiff` 在各任务间类型签名一致。公开 API `render()` 和 `diff_to_lines()` 签名在 Task 3 和 diff.rs 定义一致。
