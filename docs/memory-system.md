# Memory System Architecture

跨会话记忆系统的架构与流程说明。涵盖存储布局、写入（去重）、召回、整理（consolidation）四条主流程，以及单条记忆的生命周期。

> 代码入口：`src/context/mod.rs`（`MemoryManager`）、`src/context/storage.rs`、`src/context/consolidation.rs`、`src/context/inject.rs`、`src/agent/runtime/compactor.rs`、`src/services/auto_dream.rs`、`src/prompts/mod.rs`

## 1. 组件与存储布局

```
┌──────────────────────────── MemoryManager (context/mod.rs) ────────────────────────────┐
│                                                                                          │
│   memories: Arc<RwLock<Vec<MemoryEntry>>>      index: Arc<RwLock<MemoryIndex>>          │
│   (内存真源，load() 从盘恢复)                     (TF-IDF 倒排表，内存)                    │
│          │                                              │                                 │
│          │  add_memory / consolidate / search            │  rebuild/add_entry/replace     │
│          ▼                                              ▼                                 │
│   ┌─────────────┐    save_memory    ┌──────────────────────────┐                        │
│   │  Storage    │ ────────────────> │ ~/.wgenty-code/memory/   │  ← 记忆本体(每条一文件) │
│   │ (文件后端)  │ <────load_all──── │   {uuid}.json            │     原子写 tmp+rename   │
│   └─────────────┘                   └──────────────────────────┘                        │
│          │  reconcile(删除孤儿)                                                          │
│          │                                                                              │
│   ┌──────────────────┐    持有引用                                                      │
│   │ ConsolidationEng │  (相似度/合并/TTL)                                                │
│   └──────────────────┘                                                                 │
└──────────────────────────────────────────────────────────────────────────────────────────┘

  AutoDream 自有持久化 (services/auto_dream.rs):
    ~/.wgenty-code/.autodream_state.json     ← 门控状态(last_consolidated_at...)
    ~/.wgenty-code/memory/.consolidation.lock← consolidate 跨进程锁 (ConsolidationFileLock, 30min)
    ~/.wgenty-code/sessions/*.json           ← 门控 count_recent_sessions 扫这里
  注: AutoDream 不再自管锁 (D3), 跨进程互斥由 mm.consolidate() 内部 ConsolidationFileLock 统一保护
```

**组件职责**

| 组件 | 文件 | 职责 |
|------|------|------|
| `MemoryManager` | `context/mod.rs` | 编排：增/查/整理/加载，持有 memories Vec、index、storage、consolidation |
| `Storage` | `context/storage.rs` | 文件后端，`{id}.json` 原子写、`load_all`、`reconcile` 删孤儿 |
| `MemoryIndex` | `context/mod.rs` | 内存 TF-IDF 倒排索引，`search`/`rebuild`/`add_entry`/`replace_entry` |
| `ConsolidationEngine` | `context/consolidation.rs` | 相似度（Jaccard）、合并、TTL 衰减 |
| `MemoryContextInjector` | `context/inject.rs` | 召回：关键词提取 + 搜索 + 拼块 |
| `ApiCompactor` / daemon compactor | `agent/runtime/compactor.rs`, `tui/agent/adapters.rs` | 记忆生产者：压缩时让 LLM 提取 |
| `AutoDreamService` | `services/auto_dream.rs` | 整理触发者：daemon/headless 启动时按门控跑一次（TUI app 不再启动，D4） |

## 2. 写入流程（压缩提取 -> 去重落盘）

```
   上下文窗口满
        │
        ▼
 ┌───────────────┐    LLM 总结+提取     ┌──────────────────────────┐
 │   Compactor   │ ───────────────────> │ parse_compaction_response│
 │ (CLI/daemon   │   {summary,          │  -> Vec<MemoryEntry>     │
 │  两条路径)     │    memories[]}       │  (每条 new UUID)          │
 └───────────────┘                      └────────────┬─────────────┘
                                                     │  逐条
                                                     ▼
                                    ┌──────────────────────────────┐
                                    │      add_memory(entry)        │
                                    │   (context/mod.rs)            │
                                    └───────────────┬──────────────┘
                                                    │
                                       consolidating? ─yes─> spin wait
                                                    │ no
                                                    ▼
                                    ┌──────────────────────────────┐
                                    │ find_similar(entry, memories, │
                                    │   threshold=0.6, type-agn)   │
                                    │   (大小写不敏感 Jaccard)       │
                                    └───────────────┬──────────────┘
                                          命中?     │
                              ┌─────────yes─────────┴──────── no─────────┐
                              ▼                                          ▼
                  ┌──────────────────────┐                 ┌──────────────────────┐
                  │ merge_into(existing) │                 │  append 到 Vec        │
                  │ 保留 existing.id/type│                 │  save_memory (新文件) │
                  │ 取更丰富内容/importance│                │  index.add_entry      │
                  │ max, tags 并集        │                 └──────────────────────┘
                  └──────────┬───────────┘
                             │ save_memory 覆盖原文件
                             │ index.replace_entry (更新倒排)
                             ▼
                      (无孤儿副本)
```

**要点**

- 去重在 `add_memory` 入口完成：按内容相似度（阈值 0.6 + 子集捷径 + 跨 type + 大小写不敏感）合并到既有条目，覆盖原文件，不留重复。
- `MemoryEntry::new` 每次生成新 UUID，所以存储层按 id 去重对新提取记忆永远不生效；去重完全靠 `find_similar`。
- `consolidating` 标志保证整理期间 `add_memory` 自旋等待，不读到过渡态。

## 3. 召回流程（每轮 recall）

```
   用户发送消息 (每轮)
        │
        ▼
 ┌────────────────────────────────────────────────┐
 │ MemoryContextInjector::recall(input, top_n, 0.5)│  inject.rs
 └───────────────────────┬────────────────────────┘
                         ▼
        ┌────────────────────────────────┐
        │ extract_keywords(input)         │
        │  split -> 去停用词&<3字符        │
        │  -> lowercase -> 按词长降序      │
        │  -> dedup -> 截断 6 个           │
        └───────────────┬────────────────┘
                        │
            keywords < 2? ──yes──> 返回空 (不注入)
                        │ no
                        ▼
               query = keywords.join(" ")
                        │
                        ▼
        ┌────────────────────────────────┐
        │ search_memories(query)          │  mod.rs
        │  ① TF-IDF:                      │
        │     score = Σ tf_norm·idf       │
        │     tf_norm=1+ln(tf), idf=ln(N/df)│
        │     取 top-10 (硬编码)           │  ← score 之后丢弃
        │  ② 兜底: substring (整串contains,│
        │     几乎不命中)                  │
        └───────────────┬────────────────┘
                        ▼
        ┌────────────────────────────────┐
        │ filter importance >= 0.5        │
        │ sort by importance DESC  ◀── 最终按 importance 排序
        │ take(top_n = recall_top_n)      │
        └───────────────┬────────────────┘
                        │
             命中为空? ──yes──> 返回空
                        │ no
                        ▼
        ┌────────────────────────────────┐
        │ 拼 <memory-context> 块           │
        │  - [type] content (importance)  │
        └───────────────┬────────────────┘
                        │
          ┌─────────────┴──────────────┐
          ▼                            ▼
   ┌─────────────┐             ┌──────────────┐
   │  TUI 路径   │             │ Headless/CLI │
   │ 拆行->       │             │ inject()     │
   │ prompt_ctx  │             │ 前置到首条   │
   │ .memories   │             │ user message │
   └──────┬──────┘             └──────────────┘
          ▼
   system prompt Layer 5b
   <relevant_memories>…</relevant_memories>
   (prompts/mod.rs)
```

**要点**

- TF-IDF 只负责圈出最多 10 个候选；最终排序与截断由 `importance` 决定，相关度分数在 `search_memories` 后即丢弃。
- 候选池硬上限 10（`index.search(query, 10)`），与 `recall_top_n` 无关；`recall_top_n` 只在 importance 重排后截断。
- 关键词 < 2 或命中为空则该轮不注入。
- substring 兜底用 `keywords.join(" ")` 整串做 `contains`，多词查询几乎不会连续命中，索引冷启动时召回基本失效。

## 4. 整理流程（consolidate / AutoDream）

```
   触发源:
   ┌────────────────────────┐        ┌──────────────────────┐
   │ AutoDream (daemon/     │        │ 手动 memory dream /  │
   │  headless 启动时一次)  │        │ force_consolidation  │
   │ 门控: 1h ∧ 1session    │        │ (绕过门控)            │
   │ ∧ enabled              │        │                      │
   └───────────┬────────────┘        └──────────┬───────────┘
               └──────────┬──────────────────────┘
                          ▼
            ┌──────────────────────────────┐
            │ MemoryManager::consolidate()  │  mod.rs
            └──────────────┬───────────────┘
                           ▼
            ┌──────────────────────────────┐
            │ 1. ConsolidationFileLock      │  ← memory/.consolidation.lock
            │    (跨进程锁, 30min 过期)      │
            │ 2. consolidating = true       │  (add_memory 此时会 spin wait)
            │ 3. memories.write()           │
            └──────────────┬───────────────┘
                           ▼
            ┌──────────────────────────────┐
            │ ConsolidationEngine::consolidate│  consolidation.rs
            │  a. 按 importance 降序排       │
            │  b. should_keep? TTL 衰减:     │
            │     Knowledge/Preference 永留  │
            │     Error 半衰(age/2)          │
            │     其余 age < 24h             │
            │  c. is_similar_to_any?         │
            │     (同 type, Jaccard>0.8)     │
            │     命中->merge_memories(新id)  │
            │     否则->保留                  │
            └──────────────┬───────────────┘
                           ▼
            ┌──────────────────────────────┐
            │ 4. storage.reconcile(consolid)│  持久化 + 删孤儿文件
            │ 5. index.rebuild(consolid)    │  修复索引不同步
            │ 6. *memories = consolidated   │
            │ 7. consolidating = false      │
            └──────────────────────────────┘
```

**要点**

- consolidate 是唯一做 TTL 衰减和高阈值（0.8 + 同 type）合并的地方。
- `index.rebuild` 在替换 Vec 后重建 TF-IDF，避免整理后 positional idx 错位导致召回失效。
- AutoDream 的门控（时间 ∧ 会话数 ∧ enabled）状态存 `.autodream_state.json`，仅 `last_consolidated_at` 等持久化；`is_consolidating` 仅内存（D3 不再写磁盘锁）。

## 5. 单条记忆生命周期

```
   ┌─────────┐  压缩时 LLM 提取      ┌─────────┐
   │ (未存在) │ ───────────────────> │  出生   │  add_memory
   └─────────┘  MemoryEntry::new     └────┬────┘  落盘 {id}.json + 索引
                                              │
                                              ▼
                                    ┌─────────────────┐
                                    │   存活 / 召回    │
                                    │ 每轮 TF-IDF 命中 │<─────┐
                                    │ -> <relevant_     │      │ 下一轮用户输入
                                    │    memories> 注入 │      │ 再次召回
                                    └────────┬────────┘──────┘
                                             │
                                             ▼
                                    ┌─────────────────┐
                                    │  合并 / 衰减      │  consolidate
                                    │  - TTL 过期 -> 丢弃│  (AutoDream/手动)
                                    │  - 相似 -> merge  │
                                    └────────┬────────┘
                                             │
                                  ┌──────────┴──────────┐
                                  ▼                     ▼
                          ┌──────────────┐      ┌──────────────┐
                          │  被合并       │      │  TTL 丢弃     │
                          │ merge_memories│      │ should_keep= │
                          │ (新id, 记     │      │  false        │
                          │  录源 ids)    │      └──────┬───────┘
                          └──────┬───────┘             │
                                 │                     ▼
                                 │           ┌──────────────┐
                                 │           │ reconcile 删 │
                                 │           │ 孤儿 {id}.json│
                                 │           └──────────────┘
                                 ▼
                          (作为源被合并, 原文件删除)
```

## 6. 一句话串起来

- **写**：压缩提取 -> `add_memory` 去重（0.6 相似合并）-> 文件 + TF-IDF 索引
- **读**：每轮 `recall` -> 关键词 -> TF-IDF top-10 候选 -> importance≥0.5 过滤排序 -> 注入 prompt
- **整理**：AutoDream(daemon/headless 启动, 1h+1session) / 手动 -> `consolidate` -> TTL 衰减 + 0.8 合并 + 删孤儿 + 索引重建
- **存储**：记忆 `~/.wgenty-code/memory/{id}.json`；AutoDream 状态 `.autodream_state.json`；consolidation 锁统一在 `memory/.consolidation.lock`（D3 统一）

## 7. 已知限制与改进点

- **AutoDream 仅启动时跑一次**（daemon/headless 入口），无后台周期 tick；长会话内多次压缩靠 `add_memory` 去重兜底。
- ~~两把 consolidation 锁路径不一致~~（已修复，D3）：AutoDream 不再自管锁，统一由 `MemoryManager::consolidate()` 的 `ConsolidationFileLock` 保护。
- **召回按 importance 排序，非相关度**：TF-IDF 仅圈候选，相关度分数未参与最终排序。
- **候选池硬上限 10**：与 `recall_top_n` 无关，记忆库大时 10 之外候选永远进不来。
- **substring 兜底几乎无效**：整串 `contains` 对多词查询命中率极低。
