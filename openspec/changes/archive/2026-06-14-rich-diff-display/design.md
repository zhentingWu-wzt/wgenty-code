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
