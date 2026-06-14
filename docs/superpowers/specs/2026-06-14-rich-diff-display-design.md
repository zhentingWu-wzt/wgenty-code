---
comet_change: rich-diff-display
role: technical-design
canonical_spec: openspec
---

# Rich Diff Display — 技术设计

## 概述

将项目现有的裸 `+`/`-` diff 渲染替换为标准 unified diff 格式，对标 `git diff --unified` 和 Claude Code 的 diff 展示。

## 架构

```
┌──────────────────────────────────────────────────────────────────┐
│                    Rich Diff 数据流                               │
└──────────────────────────────────────────────────────────────────┘

  file_write / apply_patch 工具 ──► old_content + new_content
                                           │
                                           ▼
  ┌────────────────────────────────────────────────────────────────┐
  │  generate_diff(old, new, file_path) → UnifiedDiff              │
  │                                                                │
  │  similar::TextDiff::from_lines()                               │
  │    └── grouped_ops(3) ──► Vec<Vec<DiffOp>>  # 含上下文的 hunks│
  │         └── iter_changes(op) ──► 逐行提取 old_no / new_no     │
  │              └── compute_word_diffs()                           │
  │                   └── TextDiff::from_words() ──► segments      │
  └──────────────────────┬─────────────────────────────────────────┘
                         │ UnifiedDiff { hunks, additions, deletions }
                         │
            ┌────────────┴────────────┐
            ▼                         ▼
      render()                   diff_to_lines()
      独立视图                     聊天内嵌
      ratatui Paragraph           Vec<Line>
      (带行号 gutter)             (紧凑模式, 无 gutter)
```

## 组件设计

### 1. 数据结构

```rust
struct UnifiedDiff {
    file_path: String,
    hunks: Vec<Hunk>,
    additions: usize,    // 统计
    deletions: usize,    // 统计
}

struct Hunk {
    old_start: usize,    // 1-based
    old_count: usize,
    new_start: usize,
    new_count: usize,
    lines: Vec<DiffLine>,
}

struct DiffLine {
    tag: LineTag,              // Context | Delete | Insert
    old_no: Option<usize>,     // 1-based 行号
    new_no: Option<usize>,
    text: String,              // 无前缀标记的文本
    segments: Vec<Segment>,    // 空 = 整行同色渲染
}

struct Segment {
    changed: bool,    // true = 词级变更部分（亮色）
    text: String,
}
```

### 2. Diff 生成 (`generate_diff`)

- 输入：`old: &str`, `new: &str`, `file_path: &str`
- 流程：
  1. `TextDiff::from_lines(old, new)` — 行级 token 化
  2. `.grouped_ops(CONTEXT)` — 自动按上下文 (3 行) 分组，返回 `Vec<Vec<DiffOp>>`
  3. 从 `DiffOp::old_range()` / `new_range()` 提取 1-based 行号
  4. `iter_changes(op)` 逐行生成 `DiffLine`，同步推进 old_no / new_no
  5. `compute_word_diffs()` 对配对 delete/insert 运行 `from_words()`

### 3. 词级 Diff (`compute_word_diffs`)

- 扫描 hunk 行，找出连续的 Delete → Insert 分组
- 对每个配对行调用 `TextDiff::from_words(del_text, ins_text)`
- Equal 词 → Segment { changed: false }；非 Equal → Segment { changed: true }
- Context-only 区域跳过（通过先跳过 Context 行的循环保证不无限循环）

### 4. 渲染 (`render_unified`)

双模式通过 `skip_gutter: bool` 切换：

**独立视图（`render`）**：
```
  ▸ src/main.rs                              +1 -1
@@ -1,4 +1,4 @@
   1   1   fn main() {
   2   2 -     let x = 1;
        3 +     let x = 2;
   3   4       println!("{}", x);
   4   5   }
```
- Gutter：`old_no` + marker(` `/`-`/`+`) + `new_no`，CTX_COLOR
- 词级变更词：DEL_WORD_COLOR / ADD_WORD_COLOR

**内联模式（`diff_to_lines`）**：
```
  ▸ src/main.rs  +1 -1
@@ -1,4 +1,4 @@
    fn main() {
  -   let x = 1;
  +   let x = 2;
      println!("{}", x);
    }
```

### 5. 颜色常量

| 常量 | 值 | 用途 |
|------|-----|------|
| `ADD_COLOR` | (80, 200, 120) | 插入行 |
| `ADD_WORD_COLOR` | (40, 255, 100) | 插入行内变更词 |
| `DEL_COLOR` | (240, 100, 100) | 删除行 |
| `DEL_WORD_COLOR` | (255, 70, 70) | 删除行内变更词 |
| `CTX_COLOR` | (100, 100, 110) | 上下文行 / gutter |
| `HUNK_COLOR` | (60, 180, 180) | hunk 头 |
| `HEADER_COLOR` | (180, 180, 200) | 文件统计头 |

### 6. 可配置项

| 常量 | 默认值 | 说明 |
|------|--------|------|
| `CONTEXT` | 3 | 上下文行数 |
| `MAX_STANDALONE` | 50 | 独立视图截断上限 |
| `MAX_INLINE` | 25 | 内联视图截断上限 |

## 文件清单

| 文件 | 改动类型 | 说明 |
|------|---------|------|
| `src/tui/components/diff.rs` | 重写 | 完整 rich diff 引擎 + 渲染 |
| `src/tui/components/chat.rs` | 适配 | 替换旧 `diff_to_lines()` 为 `diff::diff_to_lines()` |
| `src/tui/components/mod.rs` | 新增 | `pub mod diff;` |

## 依赖

无新增依赖。`similar` 2.5 和 `syntect` 5.2 已在 `Cargo.toml` 中声明。

## 非目标

- ❌ 语法高亮 diff 内容（syntect 可用但未集成）
- ❌ 多文件 diff 列表导航
- ❌ 左右分栏对比视图
- ❌ 用户可配置的上限/上下文行数（待反馈）
