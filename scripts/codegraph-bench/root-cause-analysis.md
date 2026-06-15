# Codegraph 采纳率根因分析

> 基于 Phase 0 探针数据，分析 codegraph 工具采纳率仅有 0.05% 的根本原因。
> 日期: 2026-06-15 | 分支: `feature/20260615/codegraph-baseline-spike`

## 数据概览

- 71 个 session 中仅 1 个（1.4%）使用过 codegraph
- 1959 次 tool call 中仅 1 次是 `codegraph_node`（0.05%）
- `codegraph_explore` 从未被使用
- Top 工具：`file_read` (748), `grep` (184), `glob` (173), `file_edit` (178), `bash` (150)
- 唯一的 codegraph session 是探询性会话（"你有code graph吗"），一次调用后未再使用

---

## 根因 1: System Prompt 中 grep 被列为首选代码搜索工具

### 描述

Agent 系统 prompt（`src/prompts/base.md`）在整个代码搜索部分将 `grep` 置于 codegraph 之前，
且 `grep` 是查找函数定义等代码导航任务的**首选推荐**。Codegraph 工具在 prompt 中**完全没有被提及**
（包括工具列表、使用指南和横向对比表），Agent 因此不知道 codegraph 的存在，更不会主动调用。

### 证据

1. **`src/prompts/base.md:117-119`** — 「Search」工具段落的顺序：
   ```
   - **`grep`**: Regex-based code search. Fast, respects `.gitignore`. Use for finding function names, patterns, imports.
   - **`glob`**: Filename pattern matching. Use for finding files by name (`**/*.rs`, `*.toml`).
   - **`search`**: Full-text search across the codebase. Use for conceptual queries.
   ```
   `grep` 排在第 1 位，且明确写明 "Use for finding function names"——这正是 codegraph 最擅长的场景。
   Codegraph 工具（`codegraph_node`, `codegraph_explore`）**不存在于任何工具列表中**。

2. **`src/prompts/base.md:141-153`** — 「When to use each tool」横向对比表：
   ```
   | Goal | Tool |
   |------|------|
   | Find where a function is defined | `grep` or `lsp definition` |
   | Find all files matching a pattern | `glob` |
   | Read a file | `file_read` |
   ```
   "查找函数定义"的推荐工具是 `grep`（纯文本搜索）或 `lsp`（正则模式匹配），
   两者都是**纯文本级**的工具。`codegraph_node`（可以返回符号定义位置、签名、引用、调用者/被调用者）
   从没有被推荐给 Agent。

3. **探针数据**（`scripts/codegraph-bench/probe-session-schema.txt:78-108`）：
   - `grep` 调用 184 次 vs `codegraph_node` 1 次，差距达 184 倍
   - 即使与 `grep` 有部分重叠的 `lsp` 工具（也在定义查找推荐中）也从未出现在工具调用统计中
   - Agent 完全依赖纯文本搜索方式进行所有代码导航任务

### 影响面

- **所有代码导航类任务**（定义查找、引用查找、调用链分析）均使用低效的 grep 纯文本搜索
- 对于跨文件、跨模块的代码结构理解，Agent 逐文件 grep 消耗大量 token
- 工具调用排名中 `file_read` (748) + `grep` (184) + `glob` (173) 合计 1105 次，
  远超其他工具。用 codegraph 替代其中大量调用可显著降低 token 消耗。

### 建议修复

- 在 prompt 的「Search」段落中插入 codegraph 工具，并排在 grep 之前
- 在「When to use each tool」表中将「Find where a function is defined」的推荐改为 `codegraph_node`，
  `grep` 降为兜底选项
- 增加 codegraph 的使用场景描述，如定义查找、调用链分析、引用查找

### 归属

`codegraph-agent-adoption` (#1)

---

## 根因 2: 工具描述缺乏场景引导

### 描述

`codegraph_node` 和 `codegraph_explore` 的工具 description 以功能导向（"what it does"）为主，
缺乏场景导向（"when to use it"）的引导。Agent 无法判断"什么时候应该用 codegraph 而不是 grep"。
即使 Agent 偶然知道了 codegraph 的存在，也没有足够的上下文判断使用时机。

### 证据

1. **`src/tools/codegraph/tools.rs:61-63`** — `codegraph_node` 的 description：
   ```rust
   fn description(&self) -> &str {
       "Look up a Rust symbol by name. Returns definition location, signature, references, \
        and callers/callees. Requires a codegraph index (run `wgenty-code codegraph index` first)."
   }
   ```
   描述是纯功能性的——告诉 Agent "能做什么"但没有说"什么时候应该用"。
   没有对比性引导如 "PREFER this over grep for finding symbol definitions, callers, and implementors"。

2. **`src/tools/codegraph/tools.rs:175-177`** — `codegraph_explore` 的 description：
   ```rust
   fn description(&self) -> &str {
       "Explore code symbols and their relationships. Returns relevant symbols and call paths. \
        Requires a codegraph index (run `wgenty-code codegraph index` first)."
   }
   ```
   同理，"Explore" 和 "relationships" 对 Agent 来说太抽象。Agent 需要知道：
   什么场景应该用 `codegraph_explore` 而不是 `codegraph_node` 或 `grep`。

3. **`src/prompts/base.md:143`** 中的对比表提供了场景化的工具选择框架：
   ```
   | Goal | Tool |
   |------|------|
   | Find where a function is defined | `grep` or `lsp definition` |
   ```
   但这个表没有给 codegraph 任何条目。即便 tool description 不够场景化，
   如果对比表中列举了 codegraph，Agent 仍有选择依据。双重缺失导致 Agent 从无机会选用 codegraph。

4. **探针数据**（`scripts/codegraph-bench/probe-session-schema.txt:106-108`）：
   - `codegraph_explore` 零使用，说明即使 `codegraph_node` 被调用了 1 次，
     Agent 也从未尝试更强大的探索工具
   - 这符合"知道存在但不知道何时用更好"的行为模式

### 影响面

- **definition_lookup 类任务**：Agent 用 grep 全文搜索匹配，再 file_read 确认，
  平均需要 2-3 次 tool call。codegraph_node 一次调用即可返回精准结果。
- **call_chain 类任务**：grep 只能搜索文本匹配，无法判断调用关系。
  Agent 只能先 grep 找到可能的调用点，再 file_read 逐文件确认，效率极低。
- **reference_lookup 类任务**：codegraph_node 直接返回 `references` 字段，
  grep 需要手动搜索符号名再筛选噪音匹配。

### 建议修复

- 重写 tool description，添加 "PREFER FOR" 场景列表：
  - `codegraph_node`: 查找符号定义、查看函数签名、分析调用者/被调用者、
    查找引用（优先于 grep）
  - `codegraph_explore`: 理解模块结构、分析调用图、浏览代码库中的相关符号
- 在 prompt 对比表中明确列出 codegraph 对应场景
- 考虑在 description 中嵌入对比示例，帮助 Agent 建立使用直觉

### 归属

`codegraph-agent-adoption` (#1)

---

## 根因 3: 缺乏懒初始化成功反馈

### 描述

Codegraph 的懒初始化机制在首次使用时静默初始化，如果初始化慢或失败，
Agent 无法感知 codegraph 是否已就绪。没有"codegraph index ready"的显式反馈信号，
Agent 只能退回到 grep 搜索。同时，初始化失败的错误信息也没有被 prompt 约束引导到重试路径。

### 证据

1. **`src/tools/codegraph/tools.rs:10-34`** — 懒初始化代码结构：
   ```rust
   static ENGINE: OnceLock<Arc<QueryEngine>> = OnceLock::new();

   fn get_engine() -> Result<Arc<QueryEngine>, ToolError> {
       if let Some(engine) = ENGINE.get() {
           return Ok(engine.clone());
       }
       let cwd = std::env::current_dir()...;
       let db_path = cwd.join(".codegraph").join("index.db");
       if !db_path.exists() {
           return Err(ToolError {
               message: "No codegraph index found. Run `wgenty-code codegraph index` first."
                   .to_string(),
               code: Some("no_index".to_string()),
           });
       }
       // ...初始化成功或失败，无主动通知
       let _ = ENGINE.set(engine.clone()); // 静默，忽略竞争错误
       Ok(engine)
   }
   ```
   - 初始化成功：直接返回 `Ok(engine)`，没有任何"index ready"的信号
   - 初始化失败（index 不存在）：返回 ToolError，Agent 得到一条错误消息，
     但 prompt 没有约束"收到 codegraph 错误时应该尝试修复"的行为
   - `OnceLock` 的竞争写入被静默忽略（`let _ = ...`）

2. **`src/tools/codegraph/tools.rs:78-87`** — 执行时的错误处理：
   ```rust
   async fn execute(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
       let engine = get_engine()?;  // 错误直接 propagate
       let result = engine.codegraph_node(symbol).map_err(|e| ToolError {
           message: format!("codegraph_node query failed: {}", e),
           ...
       });
   ```
   Agent 看到的只是工具返回的通用错误，没有引导其"初始化 codegraph"或"重试"的操作建议。

3. **探针数据**（`scripts/codegraph-bench/probe-query-output.txt:106-108`）：
   - 唯一的 codegraph 调用发生在用户明确询问"你有code graph吗"的 session 中
   - 这说明代码库中没有任何东西**主动告知** Agent codegraph 的存在和就绪状态
   - Agent 需要用户提示才知道 codegraph 的存在

4. **对比其他工具**：
   - `grep`/`glob`/`file_read` 等工具不需要任何前置初始化，始终可用
   - `run_test` 虽然也需要环境准备，但 TUI 会显示测试进度和结果
   - Codegraph 作为唯一一个需要预索引的工具，既没有初始化的引导，也没有就绪的反馈

### 影响面

- **首次使用**：Agent 尝试 codegraph 时遇到"no index"错误 → 不知道如何处理 → 用 grep 兜底。后续 session 也不会再尝试。
- **大代码库 session**：如果 codegraph 初始化较慢，Agent 可能在前几次调用中遇到超时或延迟，认为"codegraph 不好用"而退回 grep。
- **用户感知**：唯一使用 codegraph 的 session 名称是"你有code graph吗"——用户（及通过用户提示才知道 codegraph 的 Agent）对 codegraph 的状态和可用性缺乏信心。

### 建议修复

- 初始化成功后，在 ToolOutput 的 metadata 中加入索引状态：
  ```json
  { "index_ready": true, "symbols_count": 12483 }
  ```
- 在 TUI 中显示 codegraph 索引状态（如 "codegraph: indexed 12,483 symbols"），在 index 就绪后主动通知
- 错误信息中加入可操作的建议："索引未找到。请在 wgenty-code 中运行 `codegraph index` 创建索引，然后重试"
- 考虑在 Agent 启动时（若 index 存在）主动输出一条系统级提示，告知 codegraph 可用

### 归属

`codegraph-agent-adoption` (#1) / `codegraph-query-and-explainability` (#2)

---

## 附加发现

### codegraph_explore 零使用

`codegraph_explore` 在 1959 次 tool call 中从未被调用。即使 `codegraph_node` 被使用了 1 次，
Agent 也从未探索更强大的 `codegraph_explore`。这可能是因为：
- `codegraph_explore` 的 description 中的 "Explore" 和 "relationships" 过于抽象
- Agent 没有场景化的指引来区分 `codegraph_node` 和 `codegraph_explore` 的适用场景

### glob 使用频繁（173 次）

Agent 使用 `glob` 探索目录结构和查找匹配文件。对于模块结构理解类的任务，
`codegraph_explore` 可以更高效地直接返回符号间的调用关系和模块层次结构。
`glob` 的频繁使用说明 Agent 倾向于通过文件路径/命名模式来推测代码结构，
这是一种基于文本命名约定的间接理解，远不如 codegraph 基于 AST 的直接分析精确。

### Read 使用极高（748 次）

`file_read` 以 748 次位居 tool call 榜首。Agent 倾向于直接读取文件内容来理解代码，
而不是先通过 codegraph 获取符号的结构化信息再决定是否深入阅读。
这种方式：
1. 消耗大量 token（读入完整文件内容）
2. 增加上下文压力（不需要的结构细节也被加载）
3. 降低效率（多文件阅读时，Agent 需要自己维护跨文件的关系推断）

### 整体归因总结

三个根因形成**螺旋式衰减**的恶性循环：

```
Prompt 不提及 codegraph
    → Agent 从不尝试
        → 工具 description 场景化不够，试了也不知道何时用
            → 没有成功反馈循环，偶尔试一次失败就放弃
                → 反馈给 Prompt 的数据中 codegraph 使用率为 0 → 无优化动力
```

打破这一循环需要从三个方向同时发力：
1. **Prompt 层**（根因 1）：让 codegraph 在 prompt 中可见、被推荐，且排在 grep 之前
2. **描述层**（根因 2）：让 Agent 理解何时用、为什么用 codegraph
3. **反馈层**（根因 3）：让 Agent 看到 codegraph 的存在和就绪状态

---

## Sources

- `src/prompts/base.md` (lines 117-119, 141-153): 系统 prompt 中 grep 的优先级和 codegraph 的缺失
- `src/tools/codegraph/tools.rs` (lines 10-34, 61-63, 78-87, 175-177): 懒初始化实现和工具描述
- `scripts/codegraph-bench/probe-session-schema.txt` (lines 6-108): 71 个 session 的统计分析
- `scripts/codegraph-bench/probe-query-output.txt` (lines 106-108): 唯一 codegraph session 的分析
