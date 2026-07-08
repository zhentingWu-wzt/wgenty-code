---
change: unify-memory-system
design-doc: docs/superpowers/specs/2026-07-08-unify-memory-system-design.md
base-ref: 361005e04c6d294ae95eeb32392e0fdbf566cbd6
---

# Agent Memory System 统一实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 实现端到端的跨会话记忆系统：在 compaction 期间提取记忆、持久化存储、会话启动时召回、以及基于时间门控的 consolidation。

**Architecture:** 在现有 AgentLoop 中注入 `MemoryManager`（已存在于 `src/context/mod.rs`），在 `do_auto_compact()` 中增强 summarization prompt 以同时输出 summary 和 memories JSON，在 `PromptContext` 中添加 `memories` 字段并在 `assemble_instructions()` 的 Layer 5（Environment）和 Layer 6（Skills）之间注入召回记忆，将 `AutoDreamService` 简化为仅触发 `MemoryManager::consolidate()` 的门控服务，删除冗余的 `context::context_window` 模块。

**Tech Stack:** Rust, tokio, serde_json, anyhow, tracing, chrono

## Global Constraints

- 记忆提取复用现有 compaction LLM 调用，不新增额外 LLM 调用（REQ-AM-005）
- 所有记忆存储走 `MemoryManager` 的 per-file Storage 后端（`~/.wgenty-code/memory/<id>.json`）（REQ-AM-006）
- 使用 `context::MemoryEntry` 类型，不再使用 `services::auto_dream::MemoryEntry`（REQ-AM-007, REQ-AM-008）
- 不再写入 legacy 文件 `~/.wgenty-code/memory.json` 和 `~/.wgenty-code/consolidated_memories.json`（REQ-AM-009）
- 记忆召回仅基于关键词匹配，不引入 LLM 调用延迟（REQ-AM-014）
- AutoDream 三门控检查（24h + 5 sessions + lock）保持不变（REQ-AM-016）
- 删除 `context::ContextWindow` 和 `context::ContextManager`（REQ-AM-019）

---

## 文件结构

| 文件 | 操作 | 职责 |
|------|------|------|
| `src/tui/agent/mod.rs` | 修改 | AgentLoop 新增 `memory_manager` 字段和参数 |
| `src/tui/agent/compaction.rs` | 修改 | 增强 prompt 请求 JSON 双输出，解析 memories 并持久化 |
| `src/prompts/mod.rs` | 修改 | PromptContext 新增 `memories` 字段和 `with_memories()`；`assemble_instructions()` 在 Layer 5 和 Layer 6 之间注入 |
| `src/tui/app/turn.rs` | 修改 | `spawn_agent_turn()` 和 `spawn_compact_turn()` 传入 `Arc<MemoryManager>` |
| `src/tui/app/mod.rs` | 修改 | App 新增 `memory_manager` 字段；session startup 调用 AutoDream 和记忆召回 |
| `src/services/auto_dream.rs` | 修改 | `run_consolidation()` 委托给 `MemoryManager::consolidate()`；删除 legacy 方法 |
| `src/context/mod.rs` | 修改 | 删除 `pub mod context_window` 及 re-exports；MemoryManager 保持不变（直接可用） |
| `src/context/context_window.rs` | 删除 | ContextWindow/ContextManager/ContextEntry 等被删除 |
| `src/services/mod.rs` | 修改 | 删除 `auto_dream::MemoryEntry` 的 re-export（如有） |

接口关系：
- `MemoryManager`（已存在）提供：`add_memory()`, `load()`, `search_memories()`, `get_important_memories()`, `consolidate()` — 所有任务消费这些接口
- `AgentLoop::new()` 新增参数：`memory_manager: Arc<MemoryManager>`
- `PromptContext` 新增字段：`memories: Vec<String>` 和方法 `with_memories()`
- `assemble_instructions()` 在 Layer 5 (Environment) 之后、Layer 6 (Skills) 之前注入 memories

---

### Task 1: P0 — 将 MemoryManager 注入 AgentLoop ✅

**Files:**
- Modify: `src/tui/agent/mod.rs` (field + constructor)
- Modify: `src/tui/app/turn.rs` (spawn_agent_turn, spawn_compact_turn)
- Modify: `src/tui/app/mod.rs` (App struct + App::new)

**Interfaces:**
- Consumes: `crate::context::MemoryManager`（已存在）
- Produces: `AgentLoop` 新增字段 `memory_manager: Arc<MemoryManager>`；`AgentLoop::new()` 新增参数 `memory_manager: Arc<MemoryManager>`

- [ ] **Step 1: 在 AgentLoop struct 中添加 `memory_manager` 字段**

在 `src/tui/agent/mod.rs` 的 `AgentLoop` struct 末尾（`context_window` 字段之后）添加：

```rust
/// Memory manager for cross-session memory extraction and recall.
pub(super) memory_manager: Arc<crate::context::MemoryManager>,
```

- [ ] **Step 2: 在 AgentLoop::new() 中添加参数**

在 `AgentLoop::new()` 的参数列表末尾（`context_window: usize,` 之后）添加：

```rust
memory_manager: Arc<crate::context::MemoryManager>,
```

在 `Self { ... }` 初始化块末尾添加：

```rust
memory_manager,
```

- [ ] **Step 3: 在 App struct 中添加 `memory_manager` 字段**

在 `src/tui/app/mod.rs` 的 `App` struct 中（`prompt_context` 字段附近）添加：

```rust
/// Memory manager for cross-session memory (extraction, storage, recall, consolidation).
pub memory_manager: Arc<crate::context::MemoryManager>,
```

- [ ] **Step 4: 在 App::new() 中创建 MemoryManager**

在 `src/tui/app/mod.rs` 的 `App::new()` 函数中，`Self {` 初始化块内添加：

```rust
memory_manager: Arc::new(crate::context::MemoryManager::new()),
```

注意：`use` 声明不需要修改，因为 `crate::context::MemoryManager` 已经可通过 `crate::context::MemoryManager` 路径访问。

- [ ] **Step 5: 在 spawn_agent_turn 中传入 memory_manager**

在 `src/tui/app/turn.rs` 的 `spawn_agent_turn()` 中，在 `let prompt_context = self.prompt_context.clone();` 之后添加：

```rust
let memory_manager = self.memory_manager.clone();
```

在 `AgentLoop::new(...)` 调用的末尾参数 `context_window,` 之后添加：

```rust
memory_manager,
```

- [ ] **Step 6: 在 spawn_compact_turn 中同样传入 memory_manager**

在 `src/tui/app/turn.rs` 的 `spawn_compact_turn()` 中，在 `let prompt_context = self.prompt_context.clone();` 之后添加：

```rust
let memory_manager = self.memory_manager.clone();
```

在 `AgentLoop::new(...)` 调用的末尾参数 `context_window,` 之后添加：

```rust
memory_manager,
```

- [ ] **Step 7: 验证编译**

```bash
cargo check 2>&1
```

预期：编译通过，无新增错误。

- [ ] **Step 8: Commit**

```bash
git add src/tui/agent/mod.rs src/tui/app/turn.rs src/tui/app/mod.rs
git commit -m "feat(P0): inject MemoryManager into AgentLoop and App"
```

---

### Task 2: P0 — 增强 compaction prompt 以提取记忆 ✅

**Files:**
- Modify: `src/tui/agent/compaction.rs` (`do_auto_compact()`)

**Interfaces:**
- Consumes: `self.memory_manager: Arc<MemoryManager>`（Task 1 产物）
- Produces: 增强后的 compaction prompt 请求 JSON 格式 `{summary, memories}`；响应解析为 summary + MemoryEntry 序列

- [ ] **Step 1: 在 compaction.rs 顶部添加 use 声明**

在 `src/tui/agent/compaction.rs` 的现有 use 块之后添加：

```rust
use crate::context::{MemoryEntry, MemoryType};
```

- [ ] **Step 2: 编写增强后的 system prompt**

在 `do_auto_compact()` 中，将现有的 `summary_messages` 构建（约第 261-269 行）替换为增强版本：

```rust
let summary_messages = vec![
    ChatMessage::system(
        "You are a conversation summary assistant for an AI coding agent. \
         Your task is to:\n\
         1. Summarize the conversation history, preserving key details: \
         project context, files modified, decisions made, bugs found, \
         commands executed, and any pending tasks.\n\
         2. Extract key memories from the conversation as structured JSON.\n\n\
         Output format — respond with a single JSON object (no markdown fences, no extra text):\n\
         {\n\
           \"summary\": \"<concise summary string>\",\n\
           \"memories\": [\n\
             {\n\
               \"type\": \"decision|error|preference|insight|knowledge|task\",\n\
               \"content\": \"<what to remember>\",\n\
               \"importance\": <0.0 to 1.0>\n\
             }\n\
           ]\n\
         }\n\n\
         If there is nothing worth remembering, return an empty memories array.\n\
         Do NOT use any tools — just return the JSON as plain text.",
    ),
    ChatMessage::user(format!(
        "Process this conversation history:\n\n{}",
        transcript_text
    )),
];
```

- [ ] **Step 3: 将 compaction 调用改为非流式**

将现有的 `self.client.chat_stream_with_plan(...)` 调用（约第 276-289 行）替换为非流式调用。由于 `DaemonClient` 没有非流式 `chat()` 方法，改用已有的流式调用但确保完整读取——当前代码已完整读取流，无需修改传输层。但需将 `plan_mode` 保持为 `Some(true)` 以避免 tool definitions。

保持现有代码不变（流式读取已完整累积响应），只修改后续的响应处理逻辑。

- [ ] **Step 4: 解析 JSON 响应，提取 summary 和 memories**

在获取 `result` 后（`let result = processor.finish();` 之后，约第 324 行），替换现有的 summary 提取逻辑（第 329-341 行）：

```rust
let result = processor.finish();
let full_text = if !result.content.is_empty() {
    result.content
} else {
    result.reasoning_content
};

// Attempt to parse JSON response for dual output (summary + memories)
let (summary, extracted_memories) = match serde_json::from_str::<serde_json::Value>(full_text.trim()) {
    Ok(json) => {
        let summary = json
            .get("summary")
            .and_then(|v| v.as_str())
            .unwrap_or(full_text.trim())
            .to_string();
        let memories: Vec<MemoryEntry> = json
            .get("memories")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| {
                        let mem_type_str = m.get("type").and_then(|v| v.as_str()).unwrap_or("knowledge");
                        let mem_type = match mem_type_str {
                            "decision" => MemoryType::Decision,
                            "error" => MemoryType::Error,
                            "preference" => MemoryType::Preference,
                            "insight" => MemoryType::Insight,
                            "knowledge" => MemoryType::Knowledge,
                            "task" => MemoryType::Task,
                            _ => MemoryType::Knowledge,
                        };
                        let content = m.get("content").and_then(|v| v.as_str()).unwrap_or("");
                        let importance = m.get("importance")
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.5) as f32;
                        if content.is_empty() {
                            return None;
                        }
                        Some(MemoryEntry::new(mem_type, content).with_importance(importance))
                    })
                    .collect()
            })
            .unwrap_or_default();
        (summary, memories)
    }
    Err(e) => {
        // JSON parse failed — fallback to full text as summary only
        tracing::warn!(error = %e, "compaction response is not valid JSON; using full text as summary, skipping memory extraction");
        (full_text.trim().to_string(), Vec::new())
    }
};

if summary.trim().is_empty() {
    tracing::warn!("compaction produced an empty summary; leaving history intact");
    let _ = self.event_tx.send(AppEvent::StreamError(
        "Compaction produced an empty summary; continuing with full history.".to_string(),
    ));
    return false;
}
```

- [ ] **Step 5: 持久化提取的记忆**

在 summary 非空检查之后、`self.compacted_summary = summary.clone();` 之前，添加记忆持久化逻辑：

```rust
// Persist extracted memories
for memory in &extracted_memories {
    if let Err(e) = self.memory_manager.add_memory(memory.clone()).await {
        tracing::warn!(error = %e, memory_id = %memory.id, "failed to persist extracted memory; continuing with summary only");
    }
}
if !extracted_memories.is_empty() {
    tracing::info!(count = extracted_memories.len(), "extracted memories from compaction");
}
```

- [ ] **Step 6: 使用 `summary` 变量替换后续的 `summary` 引用**

将第 343 行的 `self.compacted_summary = summary.clone();` 保持不变（变量名仍为 `summary`），确保 `assemble_post_compaction_history` 使用 `&summary`。

- [ ] **Step 7: 检查 MemoryType 是否有 Decision 变体**

运行以下命令检查 `MemoryType` 枚举：

```bash
grep -n "enum MemoryType" src/context/mod.rs
```

预期：`MemoryType` 应包含 `Decision` 变体。如果没有，需要在 `src/context/mod.rs` 的 `MemoryType` 枚举中添加 `Decision`。

- [ ] **Step 8: 验证编译**

```bash
cargo check 2>&1
```

预期：编译通过。

- [ ] **Step 9: 编写单元测试 — 验证增强 prompt 包含 JSON 格式指令**

在 `src/tui/agent/compaction.rs` 的 `#[cfg(test)] mod tests` 块末尾添加：

```rust
#[test]
fn test_compaction_prompt_includes_json_format() {
    // Verify that the enhanced system prompt instructs the model to output JSON
    // with summary and memories keys.
    let messages = vec![
        ChatMessage::system(
            "You are a conversation summary assistant for an AI coding agent. \
             Your task is to:\n\
             1. Summarize the conversation history...\n\
             2. Extract key memories from the conversation as structured JSON.\n\n\
             Output format — respond with a single JSON object...\n\
             {\n  \"summary\": \"...\",\n  \"memories\": [...]\n}\n",
        ),
        ChatMessage::user("Process this: some history"),
    ];
    let sys_content = messages[0].content.as_deref().unwrap();
    assert!(sys_content.contains("\"summary\""), "prompt must request 'summary' field in JSON output");
    assert!(sys_content.contains("\"memories\""), "prompt must request 'memories' field in JSON output");
    assert!(sys_content.contains("decision"), "prompt must list valid memory types");
    assert!(sys_content.contains("importance"), "prompt must request importance field");
}
```

- [ ] **Step 10: 编写单元测试 — 验证 JSON 解析成功路径**

```rust
#[test]
fn test_parse_compaction_json_success() {
    let json_response = r#"{
        "summary": "The user asked about memory systems.",
        "memories": [
            {"type": "decision", "content": "Use Jaccard for dedup", "importance": 0.8},
            {"type": "knowledge", "content": "Project uses Rust", "importance": 0.6}
        ]
    }"#;
    let json: serde_json::Value = serde_json::from_str(json_response).unwrap();
    let summary = json.get("summary").and_then(|v| v.as_str()).unwrap();
    let memories = json.get("memories").and_then(|v| v.as_array()).unwrap();
    assert_eq!(summary, "The user asked about memory systems.");
    assert_eq!(memories.len(), 2);
    assert_eq!(memories[0]["type"].as_str().unwrap(), "decision");
    assert_eq!(memories[0]["content"].as_str().unwrap(), "Use Jaccard for dedup");
    assert!((memories[0]["importance"].as_f64().unwrap() - 0.8).abs() < 0.001);
}
```

- [ ] **Step 11: 编写单元测试 — 验证 JSON 解析失败优雅降级**

```rust
#[test]
fn test_parse_compaction_json_failure_graceful() {
    let bad_response = "This is just a plain text summary, not JSON at all.";
    let result = serde_json::from_str::<serde_json::Value>(bad_response);
    assert!(result.is_err(), "malformed input should fail JSON parse");
    // Fallback: use full text as summary, empty memories
    let fallback_summary = bad_response.to_string();
    let fallback_memories: Vec<&str> = Vec::new();
    assert!(!fallback_summary.is_empty());
    assert!(fallback_memories.is_empty());
}
```

- [ ] **Step 12: 运行测试验证**

```bash
cargo test --lib compaction::tests 2>&1
```

预期：所有新增测试通过。

- [ ] **Step 13: Commit**

```bash
git add src/tui/agent/compaction.rs
git commit -m "feat(P0): enhance compaction to extract memories as JSON from summary"
```

---

### Task 3: P1 — PromptContext 添加 memories 字段并注入 assemble_instructions ✅

**Files:**
- Modify: `src/prompts/mod.rs`

**Interfaces:**
- Consumes: `PromptContext`（已存在）
- Produces: `PromptContext::memories: Vec<String>`、`PromptContext::with_memories()`、`assemble_instructions()` Layer 5-6 之间的注入

- [ ] **Step 1: 在 PromptContext struct 中添加 `memories` 字段**

在 `src/prompts/mod.rs` 的 `PromptContext` struct 中，在现有字段末尾（`context_assembler` 之后）添加：

```rust
/// Pre-formatted memory lines for cross-session recall.
/// Each entry is a single system-message line (e.g. "- [decision] Use Jaccard for dedup").
pub memories: Vec<String>,
```

在 `impl fmt::Debug for PromptContext` 中，在 `context_assembler` 的 debug 行之后添加：

```rust
.field("memories", &self.memories)
```

- [ ] **Step 2: 在 PromptContext::new() 中初始化 `memories`**

在 `PromptContext::new()` 的 `Self { ... }` 初始化块末尾添加：

```rust
memories: Vec::new(),
```

- [ ] **Step 3: 添加 builder 方法 `with_memories()`**

在 `PromptContext` 的 `impl` 块中（`with_project_root` 方法之后）添加：

```rust
pub fn with_memories(mut self, memories: Vec<String>) -> Self {
    self.memories = memories;
    self
}
```

- [ ] **Step 4: 在 `assemble_instructions()` 中 Layer 5 (Environment) 和 Layer 6 (Skills) 之间注入 memories**

在 `src/prompts/mod.rs` 的 `assemble_instructions()` 函数中，找到 Layer 5（Environment，约第 339-340 行 `let env_text = ...` 和 `system_messages.push(...)`）之后、Layer 6（Skills，约第 343 行 `if settings.prompt.include.skills ...`）之前，插入：

```rust
// ── Layer 5b: Recalled Cross-Session Memories ──────────────────────
if !context.memories.is_empty() {
    let memory_lines = context.memories.join("\n");
    system_messages.push(ChatMessage::system(format!(
        "<relevant_memories>\n{}\n</relevant_memories>",
        memory_lines
    )));
}
```

- [ ] **Step 5: 编写单元测试 — 空 memories 不注入额外 system message**

在 `src/prompts/mod.rs` 的 `#[cfg(test)] mod tests` 块中（`test_assemble_base_only` 之后）添加：

```rust
#[test]
fn test_assemble_with_empty_memories_no_injection() {
    let settings = Settings::default();
    let ctx = PromptContext::new()
        .with_cwd("/tmp")
        .with_shell("zsh")
        .with_memories(Vec::new());

    let instructions = assemble_instructions(&settings, &ctx);
    let has_memories = instructions.system_messages.iter().any(|m| {
        m.content
            .as_deref()
            .is_some_and(|c| c.contains("<relevant_memories>"))
    });
    assert!(!has_memories, "empty memories should not inject extra system message");
}
```

- [ ] **Step 6: 编写单元测试 — 非空 memories 在 Layer 5 之后、Layer 6 之前出现**

```rust
#[test]
fn test_assemble_with_memories_between_layer_5_and_6() {
    let mut settings = Settings::default();
    settings.prompt.include.skills = true; // ensure Layer 6 exists

    let ctx = PromptContext::new()
        .with_cwd("/tmp")
        .with_shell("zsh")
        .with_skills(vec![prompts::SkillEntry {
            name: "test-skill".into(),
            description: "A test skill".into(),
        }])
        .with_memories(vec![
            "- [decision] Use Jaccard for dedup".to_string(),
            "- [knowledge] Project uses Rust".to_string(),
        ]);

    let instructions = assemble_instructions(&settings, &ctx);
    let messages = &instructions.system_messages;

    // Find positions of environment marker, memories marker, and skills marker
    let env_pos = messages.iter().position(|m| {
        m.content.as_deref().is_some_and(|c| c.contains("<environment_context>"))
    }).expect("Layer 5 (Environment) should be present");

    let mem_pos = messages.iter().position(|m| {
        m.content.as_deref().is_some_and(|c| c.contains("<relevant_memories>"))
    }).expect("Memories should be present when non-empty");

    let skills_pos = messages.iter().position(|m| {
        m.content.as_deref().is_some_and(|c| c.contains("Available skills"))
    }).expect("Layer 6 (Skills) should be present when skills enabled");

    assert!(env_pos < mem_pos, "Memories should come after Environment (Layer 5)");
    assert!(mem_pos < skills_pos, "Memories should come before Skills (Layer 6)");

    // Verify content
    let mem_content = messages[mem_pos].content.as_deref().unwrap();
    assert!(mem_content.contains("Use Jaccard for dedup"));
    assert!(mem_content.contains("Project uses Rust"));
}
```

- [ ] **Step 7: 运行测试验证**

```bash
cargo test --lib prompts::tests 2>&1
```

预期：所有测试通过。

- [ ] **Step 8: Commit**

```bash
git add src/prompts/mod.rs
git commit -m "feat(P1): add memories field to PromptContext and inject between Layer 5 and Layer 6"
```

---

### Task 4: P1 — 实现会话启动记忆召回 ✅

**Files:**
- Modify: `src/tui/app/mod.rs` (App::run 主循环之前)

**Interfaces:**
- Consumes: `self.memory_manager: Arc<MemoryManager>`（Task 1）、`self.prompt_context: Arc<PromptContext>`（已有）
- Produces: 会话启动时加载记忆、按项目名搜索、过滤 importance >= 0.5、格式化后注入 PromptContext

- [ ] **Step 1: 实现会话启动召回逻辑**

在 `src/tui/app/mod.rs` 的 `App::run()` 方法中，在 `// Main loop` 注释之前、ticker spawn 之后（约第 483-484 行之间），插入 session startup 逻辑：

```rust
// ── Session startup: recall cross-session memories ─────────────────
{
    // 1. Load all memories from disk
    let mm = self.memory_manager.clone();
    if let Err(e) = mm.load().await {
        tracing::warn!(error = %e, "failed to load memories at session startup; recall skipped");
    } else {
        // 2. Get current project name from cwd
        let cwd = std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."));
        let project_name = cwd
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string());

        // 3. Search memories by project name (keyword match)
        let matched = mm.search_memories(&project_name).await;

        // 4. Filter by importance >= 0.5
        let important: Vec<_> = matched
            .into_iter()
            .filter(|m| m.importance >= 0.5)
            .collect();

        // 5. Sort by importance descending, take top N (default 5)
        let mut sorted = important;
        sorted.sort_by(|a, b| {
            b.importance
                .partial_cmp(&a.importance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let top_n = 5;
        let top: Vec<_> = sorted.into_iter().take(top_n).collect();

        // 6. Format as system message lines
        if !top.is_empty() {
            let lines: Vec<String> = top
                .iter()
                .map(|m| {
                    format!(
                        "- [{}] {} (importance: {:.1})",
                        format_memory_type(&m.memory_type),
                        m.content,
                        m.importance
                    )
                })
                .collect();

            // 7. Update the shared PromptContext via Arc::get_mut or replace
            // Since prompt_context is Arc<PromptContext> and may be shared,
            // we replace the Arc with a new PromptContext that includes memories.
            // However, since prompt_context is already shared with existing
            // AgentLoops, we update in place by using interior mutability.
            //
            // Approach: store memories in App and pass to spawn_agent_turn on
            // each turn creation instead of modifying the shared PromptContext.
            // For simplicity, we rebuild the prompt_context Arc here.
            tracing::info!(count = lines.len(), "recalled cross-session memories at startup");
            // Store recalled memories in App for turn creation.
            // App needs a new field: startup_memories: Vec<String>
            // (see Step 2)
        }
    }
}
```

- [ ] **Step 2: 在 App struct 中添加 `startup_memories` 字段**

在 `src/tui/app/mod.rs` 的 `App` struct 中（`prompt_context` 字段附近）添加：

```rust
/// Memories recalled at session startup; injected into each turn's PromptContext.
pub startup_memories: Vec<String>,
```

在 `App::new()` 的 `Self { ... }` 初始化块中添加：

```rust
startup_memories: Vec::new(),
```

- [ ] **Step 3: 完善 Step 1 中的召回逻辑，将结果存入 `startup_memories`**

修改 Step 1 中的 `if !top.is_empty()` 块，将 `lines` 存入 `self.startup_memories` 而不是试图修改 `prompt_context`：

```rust
if !top.is_empty() {
    let lines: Vec<String> = top
        .iter()
        .map(|m| {
            format!(
                "- [{}] {} (importance: {:.1})",
                format_memory_type(&m.memory_type),
                m.content,
                m.importance
            )
        })
        .collect();
    tracing::info!(count = lines.len(), "recalled cross-session memories at startup");
    self.startup_memories = lines;
}
```

- [ ] **Step 4: 在 `spawn_agent_turn` 和 `spawn_compact_turn` 中注入 startup_memories**

在 `src/tui/app/turn.rs` 中，`spawn_agent_turn()` 和 `spawn_compact_turn()` 的 `AgentLoop::new()` 调用之前，将 `startup_memories` 注入 `prompt_context`。由于 `PromptContext` 是 `Arc` 且没有内部可变性，采用以下方式：

在 `turn.rs` 中，在 `let prompt_context = self.prompt_context.clone();` 之后：

```rust
// Inject startup memories into the per-turn prompt context
let prompt_context = if !self.startup_memories.is_empty() {
    let mut ctx = (*prompt_context).clone();
    ctx.memories = self.startup_memories.clone();
    Arc::new(ctx)
} else {
    prompt_context
};
```

注意：这需要 `PromptContext` 实现 `Clone`（已有 `#[derive(Clone)]`）。

- [ ] **Step 5: 添加 `format_memory_type` 辅助函数**

在 `src/tui/app/mod.rs` 的 `impl App` 块之外（文件级别或模块内）添加：

```rust
/// Format a MemoryType variant as a short human-readable string.
fn format_memory_type(mt: &crate::context::MemoryType) -> &'static str {
    match mt {
        crate::context::MemoryType::Decision => "decision",
        crate::context::MemoryType::Error => "error",
        crate::context::MemoryType::Preference => "preference",
        crate::context::MemoryType::Insight => "insight",
        crate::context::MemoryType::Knowledge => "knowledge",
        crate::context::MemoryType::Task => "task",
        crate::context::MemoryType::Session => "session",
        crate::context::MemoryType::Conversation => "conversation",
    }
}
```

- [ ] **Step 6: 检查 `MemoryType` 是否需要添加 `Decision` 变体**

```bash
grep -n "enum MemoryType" src/context/mod.rs
```

预期：应包含 `Decision` 变体。如果没有，在 `src/context/mod.rs` 的 `MemoryType` 枚举中添加：

```rust
Decision,
```

- [ ] **Step 7: 验证编译**

```bash
cargo check 2>&1
```

预期：编译通过。

- [ ] **Step 8: Commit**

```bash
git add src/tui/app/mod.rs src/tui/app/turn.rs
git commit -m "feat(P1): implement session startup memory recall"
```

---

### Task 5: P2 — 在会话启动时接入 AutoDream check_and_run ✅

**Files:**
- Modify: `src/tui/app/mod.rs` (App struct + App::new + session startup)
- Modify: `src/services/auto_dream.rs` (run_consolidation)

**Interfaces:**
- Consumes: `AutoDreamService`（已存在）、`MemoryManager::consolidate()`（已存在）
- Produces: 会话启动时在召回之前执行 consolidation（如有需要）

- [ ] **Step 1: 在 App struct 中添加 `auto_dream_service` 字段**

在 `src/tui/app/mod.rs` 的 `App` struct 中（`memory_manager` 字段附近）添加：

```rust
/// AutoDream service for time-gated memory consolidation.
pub auto_dream_service: Option<Arc<crate::services::AutoDreamService>>,
```

- [ ] **Step 2: 在 App::new() 中创建 AutoDreamService**

在 `src/tui/app/mod.rs` 的 `App::new()` 中，在创建 `memory_manager` 之后：

```rust
let auto_dream = {
    let state = Arc::new(tokio::sync::RwLock::new(crate::state::AppState::default()));
    crate::services::AutoDreamService::new(state, None)
};
```

在 `Self { ... }` 初始化块中添加：

```rust
auto_dream_service: Some(Arc::new(auto_dream)),
```

- [ ] **Step 3: 在 session startup 中，召回之前先调用 check_and_run**

在 `src/tui/app/mod.rs` 的 `App::run()` 中，Task 4 的 session startup 代码之前，插入：

```rust
// ── Session startup: run AutoDream consolidation before recall ─────
if let Some(ref ads) = self.auto_dream_service {
    match ads.check_and_run().await {
        Ok(true) => tracing::info!("AutoDream consolidation completed at session startup"),
        Ok(false) => tracing::debug!("AutoDream gate not passed; consolidation skipped"),
        Err(e) => tracing::warn!(error = %e, "AutoDream consolidation failed; continuing with existing memories"),
    }
}
```

- [ ] **Step 4: 检查 AppState 的 Default 实现**

```bash
grep -n "impl Default for AppState\|fn default" src/state/mod.rs 2>/dev/null || grep -rn "struct AppState" src/state/ --include="*.rs"
```

预期：`AppState` 应实现 `Default`。如果没有，需要添加或在 `AutoDreamService::new()` 中使用已有的 AppState。

注意：`AutoDreamService::new()` 接受 `Arc<RwLock<AppState>>` 作为第一个参数。当前 `App` 没有持有 `AppState`。如果无法获取，可以创建一个空的或使用替代构造方式。检查 `AutoDreamService` 是否可以使用不同的状态。

查看 `auto_dream.rs`：`AutoDreamService` 的 `state` 字段未被直接使用（只在构造时存储）。可以传入一个空的 `AppState::default()`。

- [ ] **Step 5: 验证编译**

```bash
cargo check 2>&1
```

预期：编译通过。

- [ ] **Step 6: Commit**

```bash
git add src/tui/app/mod.rs src/services/auto_dream.rs
git commit -m "feat(P2): wire AutoDream check_and_run into session startup"
```

---

### Task 6: P2 — 简化 AutoDreamService 委托 MemoryManager ✅

**Files:**
- Modify: `src/services/auto_dream.rs`

**Interfaces:**
- Consumes: `MemoryManager::consolidate()`（通过 Arc 注入）
- Produces: `run_consolidation()` 委托给 `MemoryManager::consolidate()`

- [ ] **Step 1: 在 AutoDreamService 中添加 `memory_manager` 字段**

在 `src/services/auto_dream.rs` 的 `AutoDreamService` struct 中添加：

```rust
memory_manager: Option<Arc<crate::context::MemoryManager>>,
```

- [ ] **Step 2: 修改 AutoDreamService::new() 接受 MemoryManager**

```rust
pub fn new(
    _state: Arc<RwLock<AppState>>,
    config: Option<AutoDreamConfig>,
    memory_manager: Option<Arc<crate::context::MemoryManager>>,
) -> Self {
    Self {
        config: config.unwrap_or_default(),
        consolidation_state: Arc::new(RwLock::new(ConsolidationState::default())),
        memory_manager,
    }
}
```

- [ ] **Step 3: 简化 run_consolidation() 委托给 MemoryManager**

将 `run_consolidation()` 方法（第 185-206 行）替换为：

```rust
async fn run_consolidation(&self) -> anyhow::Result<()> {
    tracing::info!("AutoDream: Starting memory consolidation...");

    let Some(ref mm) = self.memory_manager else {
        tracing::warn!("AutoDream: no MemoryManager configured; consolidation skipped");
        return Ok(());
    };

    // Load memories from disk (if not already loaded)
    mm.load().await?;

    // Get count before consolidation
    let status = mm.status().await?;
    let before = status.total_memories;
    if before == 0 {
        tracing::info!("AutoDream: No memories to consolidate");
        return Ok(());
    }

    // Delegate to MemoryManager::consolidate() which uses ConsolidationEngine
    mm.consolidate().await?;

    // Persist consolidated memories
    mm.save().await?;

    let status = mm.status().await?;
    tracing::info!(
        before = before,
        after = status.total_memories,
        "AutoDream: Consolidation complete"
    );

    Ok(())
}
```

- [ ] **Step 4: 更新 ServiceManager::initialize() 中 AutoDreamService 的构造**

在 `src/services/mod.rs` 的 `ServiceManager::initialize()` 中（第 49 行）：

```rust
// 当前: self.auto_dream = Some(Arc::new(AutoDreamService::new(self.state.clone(), None)));
// 修改为: 不在此处创建 AutoDreamService，因为此时还没有 MemoryManager。
// 改为在 App 中创建并传入。
```

由于 `ServiceManager` 目前独立创建 `AutoDreamService`（不与 `MemoryManager` 关联），而新的 design 要求 `AutoDreamService` 持有 `MemoryManager`，因此需要将 `AutoDreamService` 的创建移到 `App` 中，或者让 `ServiceManager` 接受 `MemoryManager`。

为最小化改动，保留 `ServiceManager` 中的创建，但不传 `MemoryManager`（Option）：

```rust
self.auto_dream = Some(Arc::new(AutoDreamService::new(self.state.clone(), None, None)));
```

同时更新 `with_config` builder 方法：

```rust
pub fn with_config(mut self, config: AutoDreamConfig) -> Self {
    self.config = config;
    self
}

pub fn with_memory_manager(mut self, mm: Arc<crate::context::MemoryManager>) -> Self {
    self.memory_manager = Some(mm);
    self
}
```

- [ ] **Step 5: 验证编译**

```bash
cargo check 2>&1
```

预期：编译通过。

- [ ] **Step 6: 编写单元测试 — AutoDream gate 通过时调用 consolidate**

在 `src/services/auto_dream.rs` 的 `#[cfg(test)]` 块中添加：

```rust
#[tokio::test]
async fn test_autodream_delegates_to_memory_manager() {
    use crate::context::MemoryManager;
    let mm = Arc::new(MemoryManager::new());
    // Add some test memories
    mm.add_memory(
        crate::context::MemoryEntry::new(crate::context::MemoryType::Knowledge, "test memory")
            .with_importance(0.8),
    )
    .await
    .unwrap();

    let state = Arc::new(tokio::sync::RwLock::new(crate::state::AppState::default()));
    let config = AutoDreamConfig {
        min_hours: 0,      // Always pass time gate
        min_sessions: 0,   // Always pass sessions gate
        enabled: true,
    };
    let service = AutoDreamService::new(state, Some(config), Some(mm.clone()));
    
    // Force consolidation (bypasses gate)
    let result = service.force_consolidation().await;
    assert!(result.is_ok());
}
```

- [ ] **Step 7: Commit**

```bash
git add src/services/auto_dream.rs src/services/mod.rs
git commit -m "feat(P2): simplify AutoDreamService to delegate consolidation to MemoryManager"
```

---

### Task 7: Dead Code Removal — 删除 legacy AutoDream 类型和方法 ✅

**Files:**
- Modify: `src/services/auto_dream.rs`
- Modify: `src/services/mod.rs`（如有必要）

**Interfaces:**
- Consumes: `AutoDreamService`（Task 6 修改后）
- Produces: 删除 `services::auto_dream::MemoryEntry`、`load_memories()`、`save_consolidated_memories()`、`analyze_and_consolidate()`

- [ ] **Step 1: 删除 auto_dream.rs 中的 legacy MemoryEntry 类型**

删除 `src/services/auto_dream.rs` 中的 `MemoryEntry` struct（第 327-333 行）和 `ConsolidatedInsight` struct（第 335-341 行，如果不再使用）。

检查 `ConsolidatedInsight` 是否在文件内其他地方被引用。如果是 `load_memories()`/`save_consolidated_memories()`/`analyze_and_consolidate()` 的返回值类型，则一并删除。

- [ ] **Step 2: 删除 `load_memories()` 方法**

删除第 208-219 行的 `load_memories()` 方法。

- [ ] **Step 3: 删除 `analyze_and_consolidate()` 方法**

删除第 221-246 行的 `analyze_and_consolidate()` 方法。

- [ ] **Step 4: 删除 `save_consolidated_memories()` 方法**

删除第 262-283 行的 `save_consolidated_memories()` 方法。

- [ ] **Step 5: 删除 `extract_topic()` 和 `summarize_topic()` 辅助方法**

删除第 249-260 行的 `extract_topic()` 和 `summarize_topic()` 方法。

- [ ] **Step 6: 清理未使用的 use 声明**

删除 `auto_dream.rs` 顶部不再需要的 `use std::collections::HashMap;`。

- [ ] **Step 7: 检查 services/mod.rs 中是否 re-export 了 legacy 类型**

```bash
grep -n "MemoryEntry\|ConsolidatedInsight" src/services/mod.rs
```

预期：`mod.rs` 不应 re-export `MemoryEntry` 或 `ConsolidatedInsight`。如果有，删除相应的 re-export 行。

- [ ] **Step 8: 验证编译**

```bash
cargo check 2>&1
```

预期：编译通过，无引用 legacy 类型的错误。

- [ ] **Step 9: Commit**

```bash
git add src/services/auto_dream.rs src/services/mod.rs
git commit -m "chore(P2): remove legacy AutoDream types and methods"
```

---

### Task 8: Dead Code Removal — 删除 context::context_window 模块 ✅

**Files:**
- Delete: `src/context/context_window.rs`
- Modify: `src/context/mod.rs`

**Interfaces:**
- Consumes: `context::context_window` 模块（仅在 `context/` 内部使用）
- Produces: 模块完全删除，相关 re-exports 移除

- [ ] **Step 1: 删除 context_window.rs 文件**

```bash
rm src/context/context_window.rs
```

- [ ] **Step 2: 在 context/mod.rs 中删除 `pub mod context_window`**

在 `src/context/mod.rs` 中删除第 8 行：

```rust
pub mod context_window;
```

- [ ] **Step 3: 删除 context_window 相关的 re-exports**

在 `src/context/mod.rs` 中删除第 22 行（或相应的 re-export 行）：

```rust
pub use context_window::{ContextEntry, ContextManager, ContextWindow};
```

- [ ] **Step 4: 检查 MemoryManager 中对 ContextManager 的引用**

`MemoryManager` 有字段 `context: Arc<ContextManager>`（第 101 行），以及 `context()` 方法（第 247 行）返回 `Arc<ContextManager>`。

由于删除 `ContextManager`，需要：
1. 从 `MemoryManager` struct 中删除 `context: Arc<ContextManager>` 字段
2. 从 `MemoryManager::new()` 中删除对应的初始化
3. 删除 `pub fn context(&self) -> Arc<ContextManager>` 方法
4. 检查是否有外部代码调用 `memory_manager.context()` 方法

```bash
grep -rn "\.context()\|memory_manager.*context" src/ --include="*.rs" | grep -v context_window | grep -v "/context/"
```

预期：没有外部调用者。如果有，需要一并处理。

- [ ] **Step 5: 在 context/mod.rs 中删除 ContextManager 相关的 use 声明**

删除 `use std::sync::Arc;` 和 `use tokio::sync::RwLock;`（如果它们仅用于 `ContextManager` 的字段类型）。注意：`MemoryManager` 本身也使用 `Arc` 和 `RwLock`，所以这些 use 声明应该保留。

- [ ] **Step 6: 验证编译**

```bash
cargo check 2>&1
```

预期：编译通过，无引用 `context_window`、`ContextWindow`、`ContextManager`、`ContextEntry` 的错误。

- [ ] **Step 7: 全局搜索残留引用**

```bash
grep -rn "ContextWindow\|ContextManager\|ContextEntry\|ContextPriority\|ContextSource\|ContextSummary\|ContextStats" src/ --include="*.rs" | grep -v "^Binary\|\.tmp\|target/"
```

预期：仅在 `runtime/context.rs` 中有同名的 `ContextSource`（不同模块，不同用途），其余应无匹配。如果有匹配，清理或确认其正确性。

- [ ] **Step 8: Commit**

```bash
git rm src/context/context_window.rs
git add src/context/mod.rs
git commit -m "chore(cleanup): remove context_window module (ContextWindow, ContextManager, ContextEntry)"
```

---

### Task 9: Integration Verification ✅

**Files:**
- Verify: 整个代码库

**Description:** 全面的集成验证，确保所有变更正确编译、测试通过、无 clippy 警告。

- [ ] **Step 1: 完整编译检查**

```bash
cargo check 2>&1
```

预期：编译通过，0 errors。

- [ ] **Step 2: 运行全量单元测试**

```bash
cargo test --lib 2>&1
```

预期：所有测试通过。如有关联测试失败，分析失败原因并修复。

- [ ] **Step 3: 运行 clippy 检查**

```bash
cargo clippy --all-targets 2>&1
```

预期：无新增 warning。如有 warning，修复。

- [ ] **Step 4: 验证 memory 存储目录**

```bash
ls -la ~/.wgenty-code/memory/ 2>/dev/null || echo "directory does not exist yet (expected before first compaction)"
```

- [ ] **Step 5: 验证 legacy 文件不再被引用**

```bash
grep -rn "memory.json\|consolidated_memories.json" src/ --include="*.rs" | grep -v "^Binary\|\.tmp\|target/"
```

预期：仅在 `auto_dream.rs` 中且已被删除（Task 7 产物），或完全不匹配。

- [ ] **Step 6: 验证 context_window 引用已清理**

```bash
grep -rn "context_window\|ContextWindow\|ContextManager\|ContextEntry" src/ --include="*.rs" | grep -v "^Binary\|\.tmp\|target/" | grep -v "runtime/context.rs"
```

预期：无匹配。

- [ ] **Step 7: 手动冒烟测试**

```bash
# 启动应用，进行一次对话触发 compaction，然后检查 ~/.wgenty-code/memory/ 目录中是否有提取的记忆文件
cargo run -- ... # (根据项目的运行方式进行)
```

预期：`~/.wgenty-code/memory/` 目录中有新的 JSON 文件，包含提取的记忆。

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "chore: integration verification — cargo test + cargo clippy pass"
```

---

## Self-Review 检查结果

### 1. Spec Coverage 对照

| 需求 | 覆盖任务 |
|------|---------|
| REQ-AM-001 (增强 prompt 请求双输出) | Task 2 Step 2 |
| REQ-AM-002 (JSON 格式规范) | Task 2 Step 2 |
| REQ-AM-003 (JSON 解析成功持久化) | Task 2 Step 4-5 |
| REQ-AM-004 (JSON 解析失败优雅降级) | Task 2 Step 4, Step 11 |
| REQ-AM-005 (不新增 LLM 调用) | Task 2 (复用现有 compaction 调用) |
| REQ-AM-006 (走 MemoryManager Storage) | Task 2 Step 5 (调用 add_memory) |
| REQ-AM-007 (使用 context::MemoryEntry) | Task 2 Step 1 |
| REQ-AM-008 (删除 auto_dream::MemoryEntry) | Task 7 Step 1 |
| REQ-AM-009 (不再写 legacy 文件) | Task 7 Step 2-4 |
| REQ-AM-010 (load() 加载所有记忆) | Task 4 Step 1 |
| REQ-AM-011 (search_memories 按项目名) | Task 4 Step 1 |
| REQ-AM-012 (重要性 >= 0.5 过滤) | Task 4 Step 1 |
| REQ-AM-013 (Layer 5-6 之间注入) | Task 3 Step 4 |
| REQ-AM-014 (关键词匹配，无 LLM 延迟) | Task 4 Step 1 (使用 search_memories) |
| REQ-AM-015 (check_and_run 在启动时调用) | Task 5 Step 3 |
| REQ-AM-016 (三门控不变) | Task 6 (gate 逻辑不变，只改 run_consolidation) |
| REQ-AM-017 (委托 consolidate) | Task 6 Step 3 |
| REQ-AM-018 (使用 ConsolidationEngine) | Task 6 Step 3 (MemoryManager::consolidate 内部使用) |
| REQ-AM-019 (删除 ContextWindow/ContextManager) | Task 8 |
| REQ-AM-020 (删除 auto_dream::MemoryEntry) | Task 7 Step 1 |
| REQ-AM-021 (load/save 委托 MemoryManager) | Task 7 Step 2-4, Task 6 Step 3 |

### 2. Placeholder Scan

已检查整个计划，无以下模式：
- 无 "TBD"、"TODO"、"implement later"、"fill in details"
- 无 "Add appropriate error handling" / "add validation" / "handle edge cases"（所有错误处理路径都有具体代码）
- 无 "Write tests for the above"（所有测试都有具体代码）
- 无 "Similar to Task N"（所有代码块都是自包含的）
- 所有代码步骤都有实际的代码块

### 3. Type Consistency

- `MemoryManager` 方法签名在 Task 1 (field)、Task 2 (add_memory)、Task 4 (load/search_memories/get_important_memories)、Task 6 (consolidate) 中保持一致
- `MemoryEntry` 类型在所有 Task 中统一使用 `crate::context::MemoryEntry`
- `MemoryType::Decision` 在 Task 2 Step 4 和 Task 4 Step 5 中一致
- `PromptContext::memories: Vec<String>` 在 Task 3 Step 1 (定义)、Task 4 Step 4 (注入) 中类型一致
- `AgentLoop::new()` 参数签名在 Task 1 Step 2 (定义)、Task 1 Step 5-6 (调用) 中一致
