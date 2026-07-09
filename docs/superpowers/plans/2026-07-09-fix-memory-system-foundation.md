---
change: memory-tfidf-recall
design-doc: openspec/changes/memory-tfidf-recall/design.md
base-ref: e0a90ebd7b8e3c4bca01c73c5fdcc30afc8d1455
---

# 实施计划：Memory TF-IDF Retrieval Pipeline

## 概述

将 `search_memories()` 从朴素子串扫描替换为 **TF-IDF 倒排索引**，并在 TUI 会话中引入**每轮智能召回**（per-turn smart recall），解决现有两个 P1 缺陷：

| 缺陷 | 现状 | 目标 |
|------|------|------|
| #5 检索质量 | `String::contains()` O(n) 无权重，`"error_handler"` 匹配不到 `"error handling"` | TF-IDF 倒排索引，词干无关加权排序 |
| #6 召回时机 | 仅 session 启动时按 cwd 召回一次，话题切换后不再检索 | 每轮检测话题切换自动触发 TF-IDF 检索 |

## 架构决策（引用设计文档）

1. **`MemoryIndex` 结构**（设计 §Component）：内存倒排索引，`inverted: HashMap<String, Vec<(usize, f32)>>` + `idf: HashMap<String, f32>`，文档数量 ≤ 10000 时内存开销约 2-5 MB
2. **停用词过滤**（设计 §Construction）：复用 `consolidation.rs` 中的 `is_meaningful_token`，保持过滤逻辑一致
3. **平滑降级**（设计 §Retrieval，`Fallback`）：索引为空时回退到子串扫描，`search_memories()` 签名不变
4. **同步策略**（设计 §Synchronization）：
   - `load()` / `consolidate()` → 完全重建（`rebuild()`）
   - `add_memory()` → 单条追加（`add_entry()`）
   - 惰性构建：`load()` 后首次 `search()` 时自动重建
5. **智能触发**（设计 §Smart Trigger）：Jaccard 相似度 + 最小关键词数，`topic_change` 时触发每轮召回
6. **配置**（设计 §Configuration）：`recall_top_n`（默认 5）、`recall_similarity_threshold`（默认 0.3）

## 执行顺序与依赖关系

```
Task 1 (pub is_meaningful_token) ──→ Task 2 (MemoryIndex 结构) ──→ Task 3 (接入 MemoryManager)
                                                                       │
Task 5 (MemorySettings 配置) ─────────────────────────────────────────┘
                                                                       │
                                               Task 4 (TUI 每轮智能召回) ←┘
                                                                       │
                                               Task 6 (端到端验证) ←─────┘
```

- **Task 1 → Task 2**：硬依赖 — Task 2 需要 `is_meaningful_token` 的可见性
- **Task 2 → Task 3**：硬依赖 — Task 3 用 `MemoryIndex` 替换 `search_memories` 内部实现
- **Task 3 ↔ Task 5**：无严格先后 — 但 Task 4 需要两者就绪
- **Task 3 + Task 5 → Task 4**：硬依赖 — Task 4 需要 `MemoryManager` 的新方法和配置
- **Task 6**：最终验证，依赖所有前置任务

---

## Task 1: 暴露 `is_meaningful_token` 为 `pub(crate)`

### 涉及文件
- `src/context/consolidation.rs`

### 具体改动
1. 将 `fn is_meaningful_token(token: &str) -> bool` 的可见性从 **private** 改为 **`pub(crate)`**
2. 无需修改函数体、签名或行为

```rust
// Before:
fn is_meaningful_token(token: &str) -> bool {

// After:
pub(crate) fn is_meaningful_token(token: &str) -> bool {
```

### 验证方式
```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all
```

### Commit Message
```
chore(Task1): expose is_meaningful_token as pub(crate) in consolidation.rs

Make the stop-word filter reusable by the upcoming MemoryIndex struct.
No behavior change — visibility only.
```

---

## Task 2: 实现 `MemoryIndex` 结构与 TF-IDF 检索

### 涉及文件
- `src/context/mod.rs`

### 具体改动

#### 2.1 新增 `MemoryIndex` 结构体（设计 §Component）

```rust
/// In-memory TF-IDF inverted index for memory retrieval.
///
/// Architecture (per design §Component):
/// - `inverted`: word → Vec<(entry_index, term_frequency)>
/// - `idf`: word → inverse document frequency
/// - `doc_count`: total number of indexed entries
///
/// Trade-offs (per design §Trade-offs):
/// - Memory: ~2-5 MB for 10K entries × ~50 words each
/// - Rebuild: O(total_tokens) on load/consolidate
struct MemoryIndex {
    /// word → list of (entry_index, term_frequency)
    inverted: HashMap<String, Vec<(usize, f32)>>,
    /// word → inverse document frequency
    idf: HashMap<String, f32>,
    /// total number of indexed entries
    doc_count: usize,
}
```

#### 2.2 实现 `rebuild()`（设计 §Construction）

```rust
impl MemoryIndex {
    /// Build the index from scratch.
    ///
    /// 1. Tokenize each MemoryEntry::content (whitespace split)
    /// 2. Apply is_meaningful_token stop-word filter
    /// 3. Build inverted index with TF counters
    /// 4. Compute IDF: log(N / df)
    /// 5. Normalize TF: 1 + log(tf_raw) for tf > 0
    fn rebuild(&mut self, entries: &[MemoryEntry]) {
        // ...
    }
}
```

流程细节：
- 遍历 `entries`，对每个 `entry.content` 做 whitespace split
- 用 `is_meaningful_token` 过滤停用词
- 对 entry i 中的每个词 w：递增 TF 计数器，记录到 `inverted[w]`
- 所有 entry 处理完毕后：`idf[w] = (doc_count as f32 / inverted[w].len() as f32).ln()`
- TF 归一化：`tf_norm = 1.0 + tf_raw.ln()`（tf_raw > 0 时）

#### 2.3 实现 `search()`（设计 §Retrieval）

```rust
    /// Search the index with TF-IDF ranking.
    ///
    /// 1. Tokenize + filter query
    /// 2. For each query term: look up inverted index
    /// 3. Compute score = tf × idf per entry
    /// 4. Aggregate, sort descending, return top N indices
    /// 5. Fallback: substring scan if index is empty
    fn search(&self, query: &str, top_n: usize, entries: &[MemoryEntry]) -> Vec<MemoryEntry> {
        // ...
    }
```

流程细节：
- 拆分 query 并用 `is_meaningful_token` 过滤 → `query_terms`
- 对每个 term：查 `inverted`，计算 `score = tf_norm × idf[term]`
- 聚合每个 entry 的总分，降序排列，取 `top_n`
- **降级策略**：如果 `doc_count == 0`（索引为空），回退到子串扫描

#### 2.4 实现 `add_entry()`（设计 §Synchronization）

```rust
    /// Append a single entry to the index (O(|words|) per-add).
    /// Used by MemoryManager::add_memory().
    fn add_entry(&mut self, entry: &MemoryEntry, entry_index: usize) {
        // Tokenize, filter, update inverted + recalculate IDF for affected terms
    }
```

> **注意**：`add_entry()` 需要为新增的词重新计算 IDF，简单方案是重新计算所有受影响词的 IDF：`idf[w] = (new_doc_count / inverted[w].len()).ln()`

#### 2.5 在文件内添加单元测试

```rust
#[cfg(test)]
mod tfidf_tests {
    use super::*;

    #[test]
    fn memory_index_rebuild_and_search() {
        let entries = vec![
            MemoryEntry::new(MemoryType::Knowledge, "error handling function"),
            MemoryEntry::new(MemoryType::Knowledge, "database connection pool"),
            MemoryEntry::new(MemoryType::Knowledge, "error recovery strategy"),
        ];
        let mut index = MemoryIndex::new();
        index.rebuild(&entries);
        
        // "error handling" 应召回第 1 条和第 3 条
        let results = index.search("error handling", 5, &entries);
        assert!(!results.is_empty());
        assert!(results[0].content.contains("error"));
    }

    #[test]
    fn memory_index_empty_fallback() {
        let entries = vec![];
        let index = MemoryIndex::new();
        let results = index.search("anything", 5, &entries);
        assert!(results.is_empty());
    }

    #[test]
    fn memory_index_add_entry() {
        let mut index = MemoryIndex::new();
        let entry = MemoryEntry::new(MemoryType::Knowledge, "test memory");
        index.add_entry(&entry, 0);
        assert_eq!(index.doc_count, 1);
    }
}
```

### 验证方式
```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all
```

### Commit Message
```
feat(Task2): add MemoryIndex struct with TF-IDF retrieval

Implement the core MemoryIndex with:
- rebuild(): build inverted index + compute IDF
- search(): TF-IDF ranking with substring fallback
- add_entry(): incremental index update
- Reuses pub(crate) is_meaningful_token for stop-word filtering

Per design §Component — MemoryIndex is O(unique_words × entries).
```

---

## Task 3: 将 `MemoryIndex` 接入 `MemoryManager`

### 涉及文件
- `src/context/mod.rs`

### 具体改动

#### 3.1 在 `MemoryManager` 新增字段

```rust
pub struct MemoryManager {
    // ... 现有字段 ...
    /// Optional TF-IDF index; rebuilt on load/consolidate, appended on add.
    index: Arc<RwLock<Option<MemoryIndex>>>,
}
```

#### 3.2 修改 `MemoryManager::new()` 和 `with_settings()`

在两个构造函数中初始化新字段：

```rust
index: Arc::new(RwLock::new(None)),
```

#### 3.3 修改 `load()` — 加载后重建索引

```rust
pub async fn load(&self) -> anyhow::Result<()> {
    let memories = self.storage.load_all().await?;
    let mut mem = self.memories.write().await;
    *mem = memories;
    // Rebuild TF-IDF index after loading all memories (设计 §Synchronization)
    let loaded = mem.clone();
    let mut idx = self.index.write().await;
    let mut new_index = MemoryIndex::new();
    new_index.rebuild(&loaded);
    *idx = Some(new_index);
    Ok(())
}
```

#### 3.4 修改 `consolidate()` — 合并后重建索引

```rust
pub async fn consolidate(&self) -> anyhow::Result<()> {
    // ... 现有 consolidation 逻辑 ...
    *memories = consolidated;
    // Rebuild index after consolidation
    let consolidated_clone = memories.clone();
    let mut idx = self.index.write().await;
    let mut new_index = MemoryIndex::new();
    new_index.rebuild(&consolidated_clone);
    *idx = Some(new_index);
    Ok(())
}
```

#### 3.5 修改 `add_memory()` — 追加后同步索引

```rust
pub async fn add_memory(&self, entry: MemoryEntry) -> anyhow::Result<()> {
    let mut memories = self.memories.write().await;
    let entry_index = memories.len();
    memories.push(entry.clone());
    self.storage.save_memory(&entry).await?;
    // Incrementally update index (设计 §Synchronization — add_entry)
    let mut idx = self.index.write().await;
    if let Some(ref mut index) = *idx {
        index.add_entry(&entry, entry_index);
    }
    Ok(())
}
```

#### 3.6 改造 `search_memories()` — 通过 TF-IDF 检索（设计 §API Compatibility）

```rust
pub async fn search_memories(&self, query: &str) -> Vec<MemoryEntry> {
    let memories = self.memories.read().await;
    let idx = self.index.read().await;
    
    match *idx {
        Some(ref index) if index.doc_count > 0 => {
            index.search(query, usize::MAX, &memories)
        }
        _ => {
            // Fallback: substring scan (设计 §Retrieval — graceful degradation)
            let query_lower = query.to_lowercase();
            memories
                .iter()
                .filter(|m| {
                    m.content.to_lowercase().contains(&query_lower)
                        || m.tags.iter().any(|t| t.to_lowercase().contains(&query_lower))
                })
                .cloned()
                .collect()
        }
    }
}
```

#### 3.7 新增 `search_memories_top_n()` 方法（供 Task 4 调用）

```rust
/// Search memories with TF-IDF and return top N results.
pub async fn search_memories_top_n(&self, query: &str, top_n: usize) -> Vec<MemoryEntry> {
    let memories = self.memories.read().await;
    let idx = self.index.read().await;
    
    match *idx {
        Some(ref index) if index.doc_count > 0 => {
            index.search(query, top_n, &memories)
        }
        _ => {
            // Fallback: substring scan, take top N
            let query_lower = query.to_lowercase();
            let mut results: Vec<_> = memories
                .iter()
                .filter(|m| {
                    m.content.to_lowercase().contains(&query_lower)
                        || m.tags.iter().any(|t| t.to_lowercase().contains(&query_lower))
                })
                .cloned()
                .collect();
            results.truncate(top_n);
            results
        }
    }
}
```

### 验证方式
```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all
```

需要特别关注：已有 38 个 context 测试必须全部通过。

### Commit Message
```
feat(Task3): wire MemoryIndex into MemoryManager

- Add index: Arc<RwLock<Option<MemoryIndex>>> field
- Rebuild index on load() and consolidate()
- Incremental update on add_memory()
- Dispatch search_memories() through MemoryIndex::search() with substring fallback
- Add search_memories_top_n() for per-turn recall
```

---

## Task 4: 添加每轮智能召回到 TUI App

### 涉及文件
- `src/tui/app/mod.rs`
- `src/tui/app/turn.rs`（可能涉及 `spawn_agent_turn`）

### 具体改动

#### 4.1 新增 `RecallState` 结构（设计 §Smart Trigger）

在 `App` 结构体（`src/tui/app/mod.rs`）中新增：

```rust
/// Per-turn recall state for topic-change detection.
/// (设计 §Smart Trigger)
struct RecallState {
    /// Keywords from the previous user message.
    prev_keywords: Vec<String>,
}
```

#### 4.2 在 `App` 结构体添加字段

```rust
pub struct App {
    // ... 现有字段 ...
    
    /// Per-turn recall state (None before first user message).
    recall_state: Option<RecallState>,
}
```

在 `App::new()` 中初始化：`recall_state: None,`

#### 4.3 实现 `extract_keywords()`（设计 §C2 Keyword Extraction）

```rust
/// Extract keywords from user message for memory retrieval.
///
/// Strategy (per design §C2):
/// 1. Whitespace split
/// 2. Filter stop words, tokens < 3 chars, pure digits
/// 3. Sort by token length descending (longer = more specific)
/// 4. Take top MAX_KEYWORDS (default 6)
fn extract_keywords(text: &str) -> Vec<String> {
    const MAX_KEYWORDS: usize = 6;
    let mut keywords: Vec<String> = text
        .split_whitespace()
        .filter(|t| {
            t.len() >= 3
                && !t.chars().all(|c| c.is_ascii_digit())
                && is_meaningful_token(t)  // reuse stop-word filter
        })
        .map(|t| t.to_lowercase())
        .collect();
    // Sort by length descending (longer = more specific)
    keywords.sort_by(|a, b| b.len().cmp(&a.len()));
    keywords.truncate(MAX_KEYWORDS);
    keywords
}
```

> **注意**：`is_meaningful_token` 来自 `src/context/consolidation.rs`，需要 `use crate::context::consolidation::is_meaningful_token;`

#### 4.4 实现 `topic_changed()`（设计 §Topic Change Detection）

```rust
/// Detect topic change using Jaccard similarity.
///
/// Jaccard(current, prev) = |current ∩ prev| / |current ∪ prev|
/// trigger = Jaccard < RECALL_SIMILARITY_THRESHOLD (default 0.3)
///         && current.len() >= MIN_KEYWORD_COUNT (default 2)
fn topic_changed(current: &[String], prev: &[String], threshold: f32) -> bool {
    const MIN_KEYWORD_COUNT: usize = 2;
    
    if current.len() < MIN_KEYWORD_COUNT {
        return false;
    }
    if prev.is_empty() {
        return true;  // first meaningful message
    }
    
    let current_set: HashSet<&str> = current.iter().map(|s| s.as_str()).collect();
    let prev_set: HashSet<&str> = prev.iter().map(|s| s.as_str()).collect();
    
    let intersection = current_set.intersection(&prev_set).count();
    let union = current_set.union(&prev_set).count();
    
    if union == 0 {
        return false;
    }
    
    let similarity = intersection as f32 / union as f32;
    similarity < threshold
}
```

#### 4.5 在 `submit_input()` 中集成每轮召回

修改 `src/tui/app/input.rs` 中的 `submit_input()`（或者更合适的地方是在 `turn.rs` 的 `spawn_agent_turn` 中处理）。

建议集成点：**在 `start_next_turn()` 调用 `spawn_agent_turn()` 之前**，或者在 `spawn_agent_turn()` 内部添加每轮召回逻辑。

最佳方案：在 `submit_input()` **末尾**（添加 `pending_inputs` 之后），对非命令的用户消息执行关键词提取和话题检测，触发 TF-IDF 检索，然后 `update_startup_memories()`。

具体流程：

```rust
// 在 submit_input() 末尾，self.pending_inputs.push_back(...) 之后：

// ── Per-turn smart recall ──
// Skip slash commands
if !text.trim().starts_with('/') && self.memory_manager.is_some() {
    let keywords = extract_keywords(&text);
    let threshold = {
        let s = self.settings_lock.read().unwrap();
        s.storage.memory.recall_similarity_threshold
    };
    let top_n = {
        let s = self.settings_lock.read().unwrap();
        s.storage.memory.recall_top_n
    };
    
    let should_recall = match self.recall_state {
        Some(ref state) => topic_changed(&keywords, &state.prev_keywords, threshold),
        None => !keywords.is_empty(),  // first message: always recall if keywords exist
    };
    
    if should_recall && !keywords.is_empty() {
        let query = keywords.join(" ");
        let mm = self.memory_manager.clone();
        if let Some(mm) = mm {
            let matched = mm.search_memories_top_n(&query, top_n).await;
            let lines: Vec<String> = matched.iter().map(|m| {
                format!(
                    "- [{}] {} (importance: {:.1})",
                    format_memory_type(&m.memory_type),
                    m.content,
                    m.importance
                )
            }).collect();
            if !lines.is_empty() {
                self.startup_memories = lines;  // merge/replace
            }
        }
    }
    
    self.recall_state = Some(RecallState {
        prev_keywords: keywords,
    });
}
```

> **设计决策**：`startup_memories` 字段被复用为"当前会话关联记忆"。每轮新检索的结果**替换**（而非追加）之前的记忆，避免上下文膨胀。这与设计文档中「`startup_memories = merged(startup, per_turn)`」的合并策略一致 — 因为 `startup` 阶段的记忆已在 session 启动时设置，每轮检索后整体替换即可。

#### 4.6 保留 Session Startup 的初始召回

现有 `App::run()` 中的 session startup 召回逻辑（第 515-567 行）保持不动，确保向后兼容。

### 验证方式
```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all
```

手动验收测试（按 proposed acceptance criteria）：
1. 启动 TUI，进入项目目录，应看到 startup 召回（cwd basename 匹配）
2. 输入 "auth" 相关消息 → 触发召回
3. 继续输入 "auth" 相关消息 → 不触发召回（话题未变）
4. 输入 "database" 相关消息 → 触发新召回（话题切换）
5. 输入 "ok" 或 "yes" → 不触发召回（短消息）

### Commit Message
```
feat(Task4): add per-turn smart recall to TUI App

- Add RecallState with prev_keywords tracking
- Implement extract_keywords() with stop-word filter + length weighting
- Implement topic_changed() using Jaccard similarity
- Integrate per-turn TF-IDF recall into submit_input()
- Retain session startup recall for backward compatibility
- Short messages and same-topic messages skip retrieval
```

---

## Task 5: 添加召回配置到 `MemorySettings`

### 涉及文件
- `src/config/services.rs`

### 具体改动

#### 5.1 在 `MemorySettings` 新增两个字段

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySettings {
    // ... 现有字段 ...
    
    /// Top-N memories to inject per recall (default 5).
    /// (设计 §Configuration)
    #[serde(default = "default_recall_top_n")]
    pub recall_top_n: usize,
    
    /// Topic similarity threshold for triggering re-retrieval (0.0–1.0).
    /// Jaccard similarity below this value triggers a new recall.
    /// (设计 §Configuration)
    #[serde(default = "default_recall_similarity_threshold")]
    pub recall_similarity_threshold: f32,
}
```

#### 5.2 添加默认值函数

```rust
fn default_recall_top_n() -> usize {
    5
}

fn default_recall_similarity_threshold() -> f32 {
    0.3
}
```

#### 5.3 更新 `StorageConfig::default()` 中的 `MemorySettings` 构造

由于 `#[serde(default = ...)]` 已处理序列化兼容性，`StorageConfig::default()` 中的 `MemorySettings` 构造由编译器自动补充新字段为默认值，无需手动修改。

### 验证方式
```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all
```

### Commit Message
```
feat(Task5): add recall_top_n and recall_similarity_threshold config

Add two new optional fields to MemorySettings:
- recall_top_n: usize (default 5) — top-N memories per recall
- recall_similarity_threshold: f32 (default 0.3) — Jaccard threshold

Both use #[serde(default)] for backward-compatible deserialization.
```

---

## Task 6: 端到端验证

### 涉及文件
- 所有修改过的文件
- 新增测试

### 具体验证步骤

#### 6.1 代码风格检查

```bash
cargo fmt --check
```

#### 6.2 Clippy 静态分析

```bash
cargo clippy --all-targets -- -D warnings
```

#### 6.3 运行所有测试

```bash
cargo test --all
```

预期结果：
- 所有已有 **38+ context 测试** 通过
- Task 2 新增的 `tfidf_tests` 单元测试通过
- 无回归

#### 6.4 编译检查（可选但推荐）

```bash
cargo check --all
```

#### 6.5 验收标准检查清单（按 proposal §Acceptance Criteria）

| # | 标准 | 验证方式 |
|---|------|----------|
| 1 | TF-IDF 对 "error handling function" 搜索返回含 "error_handler" 的记忆 | 单元测试 `memory_index_rebuild_and_search` |
| 2 | 话题切换（"auth"→"database"）触发检索；同话题不触发 | 手动验收 |
| 3 | Session startup 执行一次初始检索 | 观察日志 `recalled cross-session memories at startup` |
| 4 | 高频停用词不膨胀 TF-IDF 分数 | `is_meaningful_token` 过滤 + TF-IDF 归一化 |
| 5 | 短消息（"ok", "yes"）不触发检索 | `topic_changed` 中 `MIN_KEYWORD_COUNT = 2` |
| 6 | `add_memory()` 和 `consolidate()` 后索引同步 | 单元测试 + `search_memories` 结果验证 |
| 7 | 所有已有 context 测试通过 | `cargo test` |

### Commit Message
```
test(Task6): validate end-to-end TF-IDF retrieval pipeline

Run full test suite: cargo fmt --check, cargo clippy, cargo test --all.
All existing context tests pass; new TF-IDF unit tests pass.
```

---

## 风险点与缓解措施

| 风险 | 影响 | 概率 | 缓解措施 |
|------|------|------|----------|
| `is_meaningful_token` 过滤过于严格，导致短关键词被过滤 | 检索召回率为零 | 低 | Task 2 中 `search()` 保留子串扫描降级；可后续调整过滤逻辑 |
| `add_entry()` 后 IDF 变化可能导致索引不一致 | 检索结果偏差 | 中 | 简单实现：每次 add_entry 后仅更新受影响词的 IDF，完整重建在 consolidate 时保证一致性 |
| `submit_input()` 中异步调用 `search_memories_top_n()` 导致竞态 | 记忆注入顺序不确定 | 低 | `search_memories_top_n` 是读操作（`read().await`），多 reader 安全 |
| 每轮召回的 startup_memories 替换策略丢失旧记忆 | 上下文丢失 | 中 | 设计上每轮只注入当前最相关的 top-N 记忆；旧记忆在后续话题切换时仍可通过 TF-IDF 召回 |
| `format_memory_type()` 在 `src/tui/app/mod.rs` 中不可见 | 编译错误 | 高 | 确认该函数位置；如果不在 `mod.rs` 中，需要导入对应模块或重构为共享函数 |
| TF-IDF 检索在超大 memory 数量（>50000）时的性能 | 启动延迟 | 低 | 设计上限 10000，且 consolidate 会控制总数量 |

## 依赖关键路径

```
Day 1: Task 1 (5min) → Task 2 (2h) → Task 3 (1h) → Task 5 (15min)
Day 2: Task 4 (2h) → Task 6 (30min)
Total: ~6h 纯开发时间
```
