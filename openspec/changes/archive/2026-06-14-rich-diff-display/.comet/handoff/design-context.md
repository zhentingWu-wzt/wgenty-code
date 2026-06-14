# Comet Design Handoff

- Change: rich-diff-display
- Phase: design
- Mode: compact
- Context hash: 5bb9a1fe7c45a656923d01cd6257d31f372fa99a9ee7c96d88216cb5e5b886ae

Generated-by: comet-handoff.sh

OpenSpec remains the canonical capability spec. This handoff is a deterministic, source-traceable context pack, not an agent-authored summary.

## openspec/changes/rich-diff-display/proposal.md

- Source: openspec/changes/rich-diff-display/proposal.md
- Lines: 1-31
- SHA256: ca3ae66c487d3df4d57b29c7fededbf9c383a52da9fe96cf73b219c6a37aadff

```md
## Why

目前 TUI 中的文件变更展示仅显示裸 `+`/`-` 行，缺少上下文、hunk 头、词级高亮和统计信息。对标 `git diff --unified` 和 Claude Code 的 diff 展示，用户需要更丰富、更可读的变更视图来快速理解代码修改。

## What Changes

- **重写 diff 渲染核心**：基于 `similar` crate 的 `grouped_ops()` 生成标准 unified diff 格式
- **Hunk 头**：`@@ -start,count +start,count @@` 精确标注变更位置
- **上下文行**：变更前后各保留 3 行不变上下文（可配置）
- **词级 Diff 高亮**：行内变更词用更亮的颜色标记（绿色/红色的深色和浅色变体）
- **统计摘要**：文件头显示 `▸ path/to/file  +N -M`
- **双模式渲染**：独立视图（带行号 gutter）和聊天内嵌（紧凑模式）
- **截断保护**：独立视图 50 行上限，内联 25 行上限
- **注册 diff 模块**：在 `components/mod.rs` 中声明并公开 API

## Capabilities

### New Capabilities
- `rich-diff-display`: 以 unified diff 格式渲染文件变更，包含 hunk 头、上下文行、词级高亮和统计信息

### Modified Capabilities
<!-- 无现有 capability 被修改，这是全新功能 -->

## Impact

| 影响范围 | 详情 |
|---------|------|
| 修改文件 | `src/tui/components/diff.rs` (重写), `src/tui/components/chat.rs` (适配新 API), `src/tui/components/mod.rs` (注册模块) |
| 依赖 | `similar` 2.5（已引入）— 无新增依赖 |
| API | 公开 `render()` 和 `diff_to_lines()` 两个函数 |
| 测试 | 8 个单元测试覆盖：空 diff、简单变更、纯增/删、词级 diff、多 hunk、渲染输出、hunk 格式 |
```

## openspec/changes/rich-diff-display/design.md

- Source: openspec/changes/rich-diff-display/design.md
- Lines: 1-54
- SHA256: 9594f50358a3b8ff7f4005cd51e63bad3de13f64ec742a575fb5efbc7e5298d0

```md
## Context

当前 `src/tui/components/diff.rs` 只做裸 `+`/`-` 行渲染，跳过所有上下文行，无 hunk 头、无词级 diff、无统计摘要。`chat.rs` 中的内联 `diff_to_lines()` 函数功能相同。

用户期望像 `git diff --unified` 或 Claude Code 那样看到完整的 diff 上下文。

## Goals / Non-Goals

**Goals:**
- 生成标准 unified diff 格式：hunk 头 + 上下文行 + `+`/`-` 标记
- 词级 diff：行内变更词用更亮的颜色高亮
- 统计摘要：文件路径 + 增删行计数
- 双模式：独立视图（带行号 gutter）+ 聊天内嵌（紧凑模式）
- 合理的截断上限（50 / 25 行）

**Non-Goals:**
- 语法高亮 diff 内容（syntect 已引入但本次不集成到 diff 渲染）
- 多文件 diff 列表导航
- 左右分栏对比视图

## Decisions

### 1. 使用 `similar::TextDiff::grouped_ops(N)` 生成 hunks
**Why**: `grouped_ops()` 自动按上下文分组变更，返回带范围的 `DiffOp`。比手动遍历 `iter_all_changes()` 并维护上下文缓冲区更简洁且不易出错。

**Alternatives considered**: 手动遍历 `iter_all_changes()` → 需要自己维护上下文缓冲区和 hunk 边界判断，代码量大且易产生边界 bug（如之前的无限循环）。

### 2. 使用 `TextDiff::from_words()` 做词级 diff
**Why**: `similar` 原生支持词级 diff，以 word boundary 切分，比字符级 diff 更干净。对单行 delete/insert 配对做词 diff，结果按 changed/unchanged 分段渲染。

**Alternatives considered**: 字符级 diff (`from_chars()`) → 过于细粒度，噪声多；行内全着色 → 丢失变更词的视觉焦点。

### 3. 双模式渲染（独立 vs 内联）
**Why**: 独立视图有完整终端宽度可用，适合带行号 gutter 的详细展示。聊天内嵌空间有限，需要紧凑模式（省略 gutter）避免折行。两个模式共用 `render_unified()` 核心，通过 `skip_gutter` 参数切换。

### 4. 配色方案
**Why**: 沿用项目已有的 diff 配色：
- 删除：`DEL_COLOR` (240,100,100) / 词级高亮 `DEL_WORD_COLOR` (255,70,70)
- 插入：`ADD_COLOR` (80,200,120) / 词级高亮 `ADD_WORD_COLOR` (40,255,100)
- 上下文：`CTX_COLOR` (100,100,110)
- Hunk 头：`HUNK_COLOR` (60,180,180)

### 5. 上下文行数 = 3
**Why**: 与 `git diff` 默认值一致，提供足够上下文又不致于冗长。硬编码为 `CONTEXT` 常量，方便后续调整为可配置项。

## Risks / Trade-offs

- **[Risk] `TextDiff::grouped_ops()` 偶发无限循环**: 第一次实现中因 `compute_word_diffs()` 未正确处理 Context-only 行导致死循环 → **Mitigation**: 在词级 diff 循环前添加 Context 跳过逻辑，8 个测试覆盖该场景。
- **[Risk] 词级 diff 在大量变更行时开销大**: 每个 delete+insert 对都运行 `from_words()` → **Mitigation**: 仅对配对行做词 diff，跳过相同文本。实际场景中单次 diff 最多几十行，性能可接受。
- **[Risk] syntect 未集成**: diff 内容是纯色，非语法高亮 → **Mitigation**: 记录为非目标，后续可加。`syntect` 依赖已引入，集成路径清晰。

## Open Questions

- 是否需要将 `CONTEXT`（上下文行数）和 `MAX_STANDALONE`/`MAX_INLINE`（截断上限）作为用户可配置项？→ 当前硬编码，用户反馈后再决定。
```

## openspec/changes/rich-diff-display/tasks.md

- Source: openspec/changes/rich-diff-display/tasks.md
- Lines: 1-32
- SHA256: 9e997aa254b0ebc48a3b46999df2196fd1e802edc3e98f077f96243f0c0d88e8

```md
## 1. 核心 Diff 引擎

- [x] 1.1 使用 `TextDiff::grouped_ops(3)` 生成 unified diff hunks
- [x] 1.2 计算 hunk 头的 old/new 起止行号和范围
- [x] 1.3 实现上下文行收集（前后各 3 行）
- [x] 1.4 实现词级 diff（`TextDiff::from_words()` 对配对 delete/insert 行）

## 2. 渲染输出

- [x] 2.1 渲染统计摘要行（`▸ path/file  +N -M`）
- [x] 2.2 渲染 hunk 头行（`@@ -start,count +start,count @@`）
- [x] 2.3 渲染带行号 gutter 的独立视图
- [x] 2.4 渲染无 gutter 的紧凑/内联视图
- [x] 2.5 词级 diff 分段渲染（changed/unchanged 不同颜色）
- [x] 2.6 行截断保护（独立 50 行 / 内联 25 行）

## 3. 模块集成

- [x] 3.1 在 `components/mod.rs` 注册 `pub mod diff`
- [x] 3.2 将 `chat.rs` 中的内联 `diff_to_lines()` 替换为 `diff::diff_to_lines()`
- [x] 3.3 公开 `render()` 和 `diff_to_lines()` API

## 4. 测试

- [x] 4.1 空 diff 测试（无变更场景）
- [x] 4.2 简单变更测试（1 增 1 删）
- [x] 4.3 纯新增/纯删除测试
- [x] 4.4 词级 diff segments 测试
- [x] 4.5 多 hunk 测试
- [x] 4.6 渲染输出完整性测试
- [x] 4.7 Hunk 头格式测试
- [x] 4.8 全项目测试套件验证（155 个测试通过）
```

## openspec/changes/rich-diff-display/specs/rich-diff-display/spec.md

- Source: openspec/changes/rich-diff-display/specs/rich-diff-display/spec.md
- Lines: 1-71
- SHA256: 4591b7b5f2b0a0f918bdc754fcf5b9f6bf1f734157f72a60f5958f6278cd63e7

```md
## ADDED Requirements

### Requirement: Render unified diff format
The system SHALL render file changes in standard unified diff (`git diff`) format, including hunk headers, context lines, and colored `+`/`-` markers.

#### Scenario: Simple change produces hunk with header
- **WHEN** a file is modified with a single-line change
- **THEN** the diff output SHALL contain a hunk header in `@@ -old_start,old_count +new_start,new_count @@` format
- **AND** the old_start, old_count, new_start, new_count SHALL accurately reflect the changed line ranges

#### Scenario: Context lines surround changes
- **WHEN** a change occurs within a file
- **THEN** up to 3 lines of unchanged context SHALL be shown before and after the changed lines

#### Scenario: Multiple changes far apart produce separate hunks
- **WHEN** two changes are separated by more than 6 unchanged lines (2 × context window)
- **THEN** the diff SHALL produce two separate hunks with independent hunk headers

### Requirement: Word-level diff highlighting
The system SHALL highlight changed words within delete/insert lines to make fine-grained modifications visible.

#### Scenario: Changed word highlighted in delete line
- **WHEN** a line is modified where only a single word changed
- **THEN** the unchanged portion of the delete line SHALL be rendered in the standard delete color
- **AND** the changed word portion SHALL be rendered in a brighter/higher-contrast delete color

#### Scenario: Changed word highlighted in insert line
- **WHEN** a line is modified where only a single word changed
- **THEN** the unchanged portion of the insert line SHALL be rendered in the standard insert color
- **AND** the changed word portion SHALL be rendered in a brighter/higher-contrast insert color

#### Scenario: Identical lines skip word diff
- **WHEN** a paired delete and insert line have identical content
- **THEN** no word-level segments SHALL be computed (standard line rendering used)

### Requirement: Diff statistics summary
The system SHALL display a summary line showing the file path and counts of additions and deletions.

#### Scenario: Stats line shows file path and change counts
- **WHEN** a diff is rendered for a file
- **THEN** the first line SHALL contain the file path prefixed by a chevron marker (▸)
- **AND** SHALL display the count of added lines after a `+` prefix
- **AND** SHALL display the count of deleted lines after a `-` prefix

#### Scenario: No changes shows empty diff indicator
- **WHEN** old and new content are identical
- **THEN** the diff output SHALL display "(no changes detected)"
- **AND** no hunk lines SHALL be rendered

### Requirement: Dual rendering modes
The system SHALL provide two rendering modes: standalone (with line-number gutter) and inline/compact (without gutter).

#### Scenario: Standalone mode shows line number gutter
- **WHEN** diff is rendered in standalone mode
- **THEN** each line SHALL include a gutter with old line number, diff marker (` `, `-`, `+`), and new line number

#### Scenario: Inline mode omits gutter for compact display
- **WHEN** diff is rendered in inline/chat mode
- **THEN** lines SHALL omit the line-number gutter
- **AND** use a `  ` / `- ` / `+ ` prefix directly before content

### Requirement: Line count truncation
The system SHALL enforce maximum line limits to prevent excessive output.

#### Scenario: Standalone view truncated at 50 lines
- **WHEN** a diff exceeds 50 lines in standalone mode
- **THEN** output SHALL be truncated with a "... (N more lines)" indicator

#### Scenario: Inline view truncated at 25 lines
- **WHEN** a diff exceeds 25 lines in inline/chat mode
- **THEN** output SHALL be truncated with a "... (truncated)" indicator
```

