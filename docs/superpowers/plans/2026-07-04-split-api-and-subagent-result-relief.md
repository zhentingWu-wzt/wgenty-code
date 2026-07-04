---
change: split-api-and-subagent-result-relief
design-doc: docs/superpowers/specs/2026-07-04-split-api-and-subagent-result-relief-design.md
base-ref: 94fe946430da339e979f2d1fe111032cb6ac2159
---

# API 模块拆分 & Subagent 大结果交付缓解 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将 `src/api/mod.rs` 拆分为 `error.rs` + `types.rs`(纯重构,工作区已落地),并把 `src/teams/subagent_mailbox.rs` 从"全量返回"两档改造为 B3 三档(Inline / Offloaded / Summarized),在不丢细节的前提下控制父 agent 上下文 token 成本。

**Architecture:** Change A 是验证型任务 —— 拆分代码已在工作区(mod.rs 381 行 + re-export、error.rs 115 行、types.rs 269 行均已就位),只需跑 `cargo build/test/clippy` 确认零行为变化、旧 `crate::api::*` 路径仍编译。Change B 是行为变更 —— 新增 `MAX_FULL_INLINE_LEN=8000` / `SUMMARY_HEAD_LEN=1500` 两个常量与 `SubagentResponse::Summarized` 变体,`offload_if_large()` 三档分档(≤4000 Inline / 4000–8000 Offloaded / >8000 Summarized;磁盘失败降级 Inline 全量),`to_content()` 三档输出(含 footer 通信磁盘路径),删除 dead code `to_compact()` 及其两个测试。调用方 `src/tools/meta/task.rs` 零改动(`to_content()` 签名不变)。

**Tech Stack:** Rust,serde,reqwest,tracing,chrono,uuid。验证用 `cargo build && cargo test --lib && cargo clippy --lib`。

## Global Constraints

- **不改 api 公共接口**:`crate::api::ChatMessage`、`crate::api::format_api_error` 等旧路径必须继续编译(design doc "Non-Goals")。
- **不改 mailbox JSONL 文件格式**:`StoredResult` 结构不变(design doc "Non-Goals")。
- **不改 subagent 调度逻辑、不改 `do_auto_compact` / `micro_compact`**(design doc "Non-Goals")。
- **常量值必须逐字匹配**:`MAX_INLINE_RESULT_LEN = 4000`(已存在,语义不变)、`MAX_FULL_INLINE_LEN = 8000`(新增)、`SUMMARY_HEAD_LEN = 1500`(新增)(design doc "双阈值与常量"表)。
- **footer 文本必须逐字匹配**(design doc "`to_content()` 三档输出"):
  - Offloaded:`{content}\n\n---\n[Full result ({content_len} chars) persisted at: `{path}` for recovery]`
  - Summarized:`{summary}\n\n---\n[Summary only. Full result ({content_len} chars) at `{path}` — file_read for details]`(注意是 em-dash `—`,backtick 包裹 path)
- **磁盘失败降级**:`store()` 失败时即使 `>8000` 也返回 `Inline { content }` 全量(design doc "磁盘持久化失败降级":完整性优先于 token 控制)。
- **summary 生成方式**:`content.chars().take(SUMMARY_HEAD_LEN).collect::<String>()`(按字符数取前缀,非字节)(design doc `offload_if_large` 代码块)。
- **`content_len` 语义**:存 `content.len()`(字节长度),与现有 `Offloaded` 变体一致。

---

## File Structure

| 文件 | 责任 | Change | 本计划是否改动 |
|------|------|--------|----------------|
| `src/api/mod.rs` | `ApiClient` 实现 + `pub mod error/types` 声明 + glob re-export | A | 已落地,Task 1 仅验证 |
| `src/api/error.rs` | `wrap_network_error` / `format_api_error` 及其单测 | A | 已落地,Task 1 仅验证 |
| `src/api/types.rs` | wire-format 类型(`ToolDefinition`/`ChatMessage`/`ChatRequest`/`ChatResponse`/`StreamChunk` 等)及其单测 | A | 已落地,Task 1 仅验证 |
| `src/teams/subagent_mailbox.rs` | `SubagentResponse` 变体、`offload_if_large()` 三档分档、`to_content()` 三档输出、`SubagentResultMailbox::store/read`、测试 | B | Task 2–5 改动 |
| `src/tools/meta/task.rs` | subagent 调用方(`:558` 后台、`:704` 同步调 `offload_if_large(...).to_content()`) | B | 零改动,Task 6 仅确认 |
| `src/tui/agent/compaction.rs` | `do_auto_compact` / `micro_compact` | — | 零改动(B3 不依赖 compaction) |

---

## Task 1: 验证 Change A — src/api 模块拆分(纯重构)

**Files:**
- Verify (already in workspace): `src/api/mod.rs`(381 行,re-export 已就位 on lines 6-17)
- Verify (already in workspace): `src/api/error.rs`(115 行,untracked)
- Verify (already in workspace): `src/api/types.rs`(269 行,untracked)

**Interfaces:**
- Consumes: 无(工作区代码已就位)
- Produces: 确认 `crate::api::*` 旧路径继续编译,`crate::api::format_api_error`(`daemon/handlers.rs:98` 调用)、`crate::api::ChatMessage`(20+ 处调用)、`crate::api::ApiClient`、`crate::api::ToolDefinition` 等均可用

**背景**:tasks.md 1.1–1.3(创建 error.rs/types.rs、移动代码、更新 mod.rs re-export)已在工作区完成。git status 显示 `src/api/mod.rs` 为 modified、`error.rs`/`types.rs` 为 untracked。本任务是 Change A 的实际执行步骤 —— task 1.4 验证。

- [x] **Step 1: 确认工作区拆分文件存在且 re-export 就位**

Run:
```bash
ls -la src/api/error.rs src/api/types.rs src/api/mod.rs
wc -l src/api/error.rs src/api/types.rs src/api/mod.rs
```
Expected: 三个文件均存在;`error.rs` 与 `types.rs` 为 untracked(`git status` 显示 `??`),`mod.rs` 为 modified(`M`)。

确认 `src/api/mod.rs` 头部包含:
```rust
pub mod error;
pub mod types;
// ...
pub use types::*;
pub(crate) use error::*;
```
(re-export 在 lines 6、9、16、17;lines 11-17 注释说明 glob 选择理由)

- [x] **Step 2: cargo build 验证编译通过**

Run:
```bash
cargo build 2>&1 | tail -20
```
Expected: `Finished` 无错误。旧 `crate::api::*` 路径(`crate::api::ChatMessage`、`crate::api::ApiClient`、`crate::api::ToolDefinition`、`crate::api::format_api_error` 等)全部继续编译 —— glob re-export 生效。

- [x] **Step 3: cargo test --lib 验证迁移的单测全绿**

Run:
```bash
cargo test --lib 2>&1 | tail -30
```
Expected: 所有测试 PASS,包括从 `mod.rs` 迁移到 `error.rs` / `types.rs` 的单元测试。无 `FAILED`、无 panic。

- [x] **Step 4: cargo clippy --lib 验证无新警告**

Run:
```bash
cargo clippy --lib 2>&1 | tail -20
```
Expected: 无 warning、无 error(或仅有与本次拆分无关的既有警告)。特别确认无 `pub(crate) use error::*;` glob 相关警告。

- [x] **Step 5: 确认 critical 调用点仍编译**

Run:
```bash
grep -rn "crate::api::format_api_error" src/
```
Expected: `src/daemon/handlers.rs:98` 仍命中 —— 证明 `pub(crate) use error::*;` glob 成功再导出 `format_api_error` 为 `crate::api::format_api_error`。

Run:
```bash
grep -rn "crate::api::ChatMessage\|crate::api::ApiClient\|crate::api::ToolDefinition" src/ | wc -l
```
Expected: 命中数 ≥ 20(证明 `pub use types::*;` glob 成功再导出 wire-format 类型)。

- [x] **Step 6: Commit Change A**

```bash
git add src/api/mod.rs src/api/error.rs src/api/types.rs
git commit -m "refactor(api): split src/api/mod.rs into error.rs + types.rs

Pure refactor — error helpers (wrap_network_error/format_api_error) move to
error.rs, wire-format types (ChatMessage/ToolDefinition/ChatResponse/StreamChunk
etc.) move to types.rs. mod.rs keeps ApiClient impl + glob re-export
(pub use types::*; pub(crate) use error::*;) so all crate::api::* paths
continue to compile. Zero behavior change."
```

---

## Task 2: 新增 Summarized 变体 + 常量 + 更新 len()/to_content()

**Files:**
- Modify: `src/teams/subagent_mailbox.rs`
  - 常量区(line 33 附近):新增两个常量
  - `SubagentResponse` enum(lines 57-67):新增 `Summarized` 变体
  - `to_content()`(lines 74-92):新增 `Summarized` 分支
  - `len()`(lines 103-108):新增 `Summarized` 分支

**Interfaces:**
- Consumes: 无
- Produces:
  - `pub const MAX_FULL_INLINE_LEN: usize = 8000;`
  - `pub const SUMMARY_HEAD_LEN: usize = 1500;`
  - `SubagentResponse::Summarized { summary: String, mailbox_path: PathBuf, content_len: usize }`
  - `to_content()` 对 `Summarized` 返回 `{summary}\n\n---\n[Summary only. Full result ({content_len} chars) at `{path}` — file_read for details]`
  - `len()` 对 `Summarized` 返回 `*content_len`

**说明**:本任务只做结构层改动(新变体 + `to_content`/`len` 处理新变体以通过编译)。`offload_if_large()` 三档分档在 Task 3 实现,所以本任务后 `Summarized` 变体仍不会被 `offload_if_large` 产生 —— 用直接构造的方式测试 `to_content()`。

- [ ] **Step 1: 写失败测试 — Summarized 的 to_content 输出含 summary + footer + path**

在 `src/teams/subagent_mailbox.rs` 的 `#[cfg(test)] mod tests` 内(现有测试之后、`to_compact` 相关测试之前,即 line 311 之前)新增:

```rust
    #[test]
    fn test_summarized_to_content_has_summary_and_path() {
        let summary = "A".repeat(1500);
        let resp = SubagentResponse::Summarized {
            summary: summary.clone(),
            mailbox_path: PathBuf::from("/tmp/fake_mailbox/result.jsonl"),
            content_len: 9000,
        };
        let text = resp.to_content();
        // Summary (head 1500 chars) is present
        assert!(text.contains(&summary));
        // Footer communicates disk path + content_len + file_read hint
        assert!(text.contains("Summary only."));
        assert!(text.contains("Full result (9000 chars)"));
        assert!(text.contains("/tmp/fake_mailbox/result.jsonl"));
        assert!(text.contains("file_read for details"));
    }

    #[test]
    fn test_summarized_summary_is_head_prefix() {
        // Verify the summary field semantics: it should be the head prefix
        // of the full content. This test documents the contract that
        // offload_if_large (Task 3) will produce summary via
        // content.chars().take(SUMMARY_HEAD_LEN).collect().
        let full = "B".repeat(9000);
        let expected_summary: String = full.chars().take(SUMMARY_HEAD_LEN).collect();
        assert_eq!(expected_summary.len(), 1500);
        assert_eq!(expected_summary, "B".repeat(1500));
    }
```

- [ ] **Step 2: 运行测试确认失败(变体不存在,编译错误)**

Run:
```bash
cargo test --lib subagent_mailbox::tests::test_summarized_to_content_has_summary_and_path 2>&1 | tail -20
```
Expected: 编译失败 —— `no variant named Summarized found for enum SubagentResponse`(变体尚未定义)。

- [ ] **Step 3: 新增两个常量**

在 `src/teams/subagent_mailbox.rs` line 33(`MAX_INLINE_RESULT_LEN` 定义)之后新增:

```rust
/// Threshold (chars) above which the full content is no longer inlined into
/// the parent agent's context — instead a head-prefix summary is delivered
/// with a disk-recovery hint. Results between `MAX_INLINE_RESULT_LEN` and
/// this value are still inlined in full (with a disk copy).
pub const MAX_FULL_INLINE_LEN: usize = 8000;

/// Length (chars) of the head-prefix summary delivered for `Summarized`
/// results. `content.chars().take(SUMMARY_HEAD_LEN)`.
pub const SUMMARY_HEAD_LEN: usize = 1500;
```

- [ ] **Step 4: 新增 Summarized 变体**

修改 `SubagentResponse` enum(替换 lines 57-67 整个 enum 定义):

```rust
/// Wrapper returned by `offload_if_large`. Three tiers balance "no detail
/// loss" against parent-agent context token cost:
/// - `Inline`: small results (≤4000 chars) returned as-is, no disk copy.
/// - `Offloaded`: medium results (4000–8000 chars) returned in full AND
///   persisted to disk for recovery.
/// - `Summarized`: large results (>8000 chars) deliver a head-prefix summary
///   plus a disk path; the full content is only on disk (recoverable via
///   `file_read`).
///
/// On disk-persistence failure, results degrade to `Inline` (full content,
/// no copy) — content integrity takes priority over token control.
#[derive(Debug, Clone)]
pub enum SubagentResponse {
    /// ≤4000 chars: full content inline, no disk copy.
    Inline { content: String },
    /// 4000–8000 chars: full content inline + disk copy for recovery.
    Offloaded {
        content: String,
        mailbox_path: PathBuf,
        content_len: usize,
    },
    /// >8000 chars: head-prefix summary inline + disk copy; full content
    /// recoverable via `file_read` on `mailbox_path`.
    Summarized {
        summary: String,
        mailbox_path: PathBuf,
        content_len: usize,
    },
}
```

- [ ] **Step 5: 更新 to_content() 新增 Summarized 分支**

修改 `to_content()`(替换 lines 74-92 整个方法):

```rust
    /// Turn this response into text suitable for tool output.
    ///
    /// - `Inline` → content as-is.
    /// - `Offloaded` → full content + footer noting the on-disk recovery path.
    /// - `Summarized` → head-prefix summary + footer pointing to the disk
    ///   copy for `file_read` recovery.
    pub fn to_content(&self) -> String {
        match self {
            Self::Inline { content } => content.clone(),
            Self::Offloaded {
                content,
                mailbox_path,
                content_len,
            } => format!(
                "{content}\n\n\
                 ---\n\
                 [Full result ({content_len} chars) persisted at: `{path}` for recovery]",
                content = content,
                content_len = content_len,
                path = mailbox_path.display(),
            ),
            Self::Summarized {
                summary,
                mailbox_path,
                content_len,
            } => format!(
                "{summary}\n\n\
                 ---\n\
                 [Summary only. Full result ({content_len} chars) at `{path}` — file_read for details]",
                summary = summary,
                content_len = content_len,
                path = mailbox_path.display(),
            ),
        }
    }
```

注意:Summarized footer 中的 `—` 是 em-dash(U+2014),不是 ASCII `-`。path 用 backtick 包裹。

- [ ] **Step 6: 更新 len() 新增 Summarized 分支**

修改 `len()`(替换 lines 103-108 整个方法):

```rust
    pub fn len(&self) -> usize {
        match self {
            Self::Inline { content } => content.len(),
            Self::Offloaded { content_len, .. } => *content_len,
            Self::Summarized { content_len, .. } => *content_len,
        }
    }
```

- [ ] **Step 7: 运行测试确认新增测试通过**

Run:
```bash
cargo test --lib subagent_mailbox::tests::test_summarized_to_content_has_summary_and_path subagent_mailbox::tests::test_summarized_summary_is_head_prefix 2>&1 | tail -20
```
Expected: 两个测试 PASS。

- [ ] **Step 8: 运行全量 lib 测试确认无回归**

Run:
```bash
cargo test --lib 2>&1 | tail -20
```
Expected: 所有测试 PASS(包括既有 4 个 mailbox 测试 + 新增 2 个)。`offload_if_large` 尚未产生 `Summarized`,但既有 Inline/Offloaded 行为不变。

- [ ] **Step 9: Commit**

```bash
git add src/teams/subagent_mailbox.rs
git commit -m "feat(subagent_mailbox): add Summarized variant + MAX_FULL_INLINE_LEN/SUMMARY_HEAD_LEN constants

Add third tier to SubagentResponse for >8000-char results: head-prefix
summary (1500 chars) + disk path footer. Update to_content() and len() to
handle the new variant. offload_if_large() 3-tier dispatch follows in next
commit."
```

---

## Task 3: offload_if_large() 三档分档 + 边界测试

**Files:**
- Modify: `src/teams/subagent_mailbox.rs`
  - `offload_if_large()`(lines 207-238):改造为三档分档
  - `#[cfg(test)] mod tests`:新增 3 个测试

**Interfaces:**
- Consumes: Task 2 的 `Summarized` 变体、`MAX_FULL_INLINE_LEN`、`SUMMARY_HEAD_LEN`
- Produces: `offload_if_large()` 三档行为
  - `len <= 4000` → `Inline { content }`
  - `4000 < len <= 8000` + store 成功 → `Offloaded { content, mailbox_path, content_len }`
  - `len > 8000` + store 成功 → `Summarized { summary: content.chars().take(1500).collect(), mailbox_path, content_len }`
  - 任何 `len > 4000` + store 失败 → `Inline { content }`(全量,降级,logged)

- [ ] **Step 1: 写失败测试 — very_large 产生 Summarized**

在 `src/teams/subagent_mailbox.rs` 的 `#[cfg(test)] mod tests` 内新增:

```rust
    #[test]
    fn test_very_large_result_summarized() {
        let mailbox = SubagentResultMailbox::new(
            std::env::temp_dir().join("wgenty_test_mailbox_summarized"),
        );
        let very_large = "A".repeat(9000);
        let response = mailbox.offload_if_large(
            "general-purpose",
            "very large analysis",
            "session_sum",
            &very_large,
        );
        match response {
            SubagentResponse::Summarized {
                ref summary,
                ref mailbox_path,
                ref content_len,
            } => {
                // content_len is the full byte length, not summary length
                assert_eq!(*content_len, 9000);
                // summary is the head 1500 chars (by chars(), not bytes)
                assert_eq!(summary.chars().count(), 1500);
                assert_eq!(summary.as_str(), &very_large[..1500]); // ASCII so byte==char
                // disk copy exists for recovery
                assert!(mailbox_path.exists());
            }
            _ => panic!("Expected Summarized for 9000-char result"),
        }
    }
```

- [ ] **Step 2: 写失败测试 — 边界 4000 vs 4001**

```rust
    #[test]
    fn test_boundary_4000_inline_vs_offloaded() {
        let mailbox = SubagentResultMailbox::new(
            std::env::temp_dir().join("wgenty_test_mailbox_boundary_4k"),
        );
        // Exactly 4000 → Inline (<= threshold)
        let exactly_4000 = "A".repeat(4000);
        let resp = mailbox.offload_if_large("explore", "b4k", "s1", &exactly_4000);
        match resp {
            SubagentResponse::Inline { content } => assert_eq!(content.len(), 4000),
            _ => panic!("Expected Inline for 4000-char result (<= MAX_INLINE_RESULT_LEN)"),
        }

        // 4001 → Offloaded (> threshold, <= MAX_FULL_INLINE_LEN)
        let just_over_4001 = "A".repeat(4001);
        let resp = mailbox.offload_if_large("explore", "b4k", "s2", &just_over_4001);
        match resp {
            SubagentResponse::Offloaded { content_len, .. } => assert_eq!(content_len, 4001),
            _ => panic!("Expected Offloaded for 4001-char result"),
        }
    }
```

- [ ] **Step 3: 写失败测试 — 边界 8000 vs 8001**

```rust
    #[test]
    fn test_boundary_8000_offloaded_vs_summarized() {
        let mailbox = SubagentResultMailbox::new(
            std::env::temp_dir().join("wgenty_test_mailbox_boundary_8k"),
        );
        // Exactly 8000 → Offloaded (<= MAX_FULL_INLINE_LEN)
        let exactly_8000 = "A".repeat(8000);
        let resp = mailbox.offload_if_large("explore", "b8k", "s1", &exactly_8000);
        match resp {
            SubagentResponse::Offloaded { content_len, .. } => assert_eq!(content_len, 8000),
            _ => panic!("Expected Offloaded for 8000-char result (<= MAX_FULL_INLINE_LEN)"),
        }

        // 8001 → Summarized (> MAX_FULL_INLINE_LEN)
        let just_over_8001 = "A".repeat(8001);
        let resp = mailbox.offload_if_large("explore", "b8k", "s2", &just_over_8001);
        match resp {
            SubagentResponse::Summarized { content_len, .. } => assert_eq!(content_len, 8001),
            _ => panic!("Expected Summarized for 8001-char result (> MAX_FULL_INLINE_LEN)"),
        }
    }
```

- [ ] **Step 4: 运行测试确认失败**

Run:
```bash
cargo test --lib subagent_mailbox::tests::test_very_large_result_summarized subagent_mailbox::tests::test_boundary_4000_inline_vs_offloaded subagent_mailbox::tests::test_boundary_8000_offloaded_vs_summarized 2>&1 | tail -30
```
Expected: 3 个测试 FAIL —— `test_very_large_result_summarized` 与 `test_boundary_8000_offloaded_vs_summarized` 会 panic("Expected Summarized ..."),因为当前 `offload_if_large` 对 >4000 一律返回 `Offloaded`(无第三档)。`test_boundary_4000_inline_vs_offloaded` 应已 PASS(4000 → Inline、4001 → Offloaded 是当前行为)。

- [ ] **Step 5: 改造 offload_if_large() 为三档分档**

修改 `offload_if_large()`(替换 lines 199-238 整个方法,含 doc 注释):

```rust
    /// Persist a subagent result and return a [`SubagentResponse`] tiered by
    /// content size:
    ///
    /// - `len <= MAX_INLINE_RESULT_LEN` (4000): `Inline` — full content, no
    ///   disk copy.
    /// - `MAX_INLINE_RESULT_LEN < len <= MAX_FULL_INLINE_LEN` (4000–8000):
    ///   `Offloaded` — full content inline + disk copy for recovery.
    /// - `len > MAX_FULL_INLINE_LEN` (>8000): `Summarized` — head-prefix
    ///   summary (`SUMMARY_HEAD_LEN` chars) inline + disk copy; full content
    ///   recoverable via `file_read`.
    ///
    /// On disk-persistence failure for any result >4000, degrades to
    /// `Inline` with the **full** content (no truncation, no copy) — content
    /// integrity takes priority over token control. The failure is logged.
    pub fn offload_if_large(
        &self,
        subagent_type: &str,
        description: &str,
        session_id: &str,
        content: &str,
    ) -> SubagentResponse {
        let len = content.len();
        if len <= MAX_INLINE_RESULT_LEN {
            return SubagentResponse::Inline {
                content: content.to_string(),
            };
        }
        // >4000: attempt disk persistence first.
        match self.store(subagent_type, description, session_id, content) {
            Ok(path) => {
                if len <= MAX_FULL_INLINE_LEN {
                    // 4000–8000: full content inline + disk copy.
                    SubagentResponse::Offloaded {
                        content: content.to_string(),
                        mailbox_path: path,
                        content_len: len,
                    }
                } else {
                    // >8000: head-prefix summary inline + disk copy.
                    let summary: String = content.chars().take(SUMMARY_HEAD_LEN).collect();
                    SubagentResponse::Summarized {
                        summary,
                        mailbox_path: path,
                        content_len: len,
                    }
                }
            }
            Err(e) => {
                // Disk failure: degrade to Inline (full content, no copy, logged).
                // Integrity > token control — even >8000 returns full content.
                tracing::warn!(
                    error = %e,
                    "Failed to persist subagent result; returning full inline (no recovery copy)"
                );
                SubagentResponse::Inline {
                    content: content.to_string(),
                }
            }
        }
    }
```

- [ ] **Step 6: 运行 3 个新测试确认通过**

Run:
```bash
cargo test --lib subagent_mailbox::tests::test_very_large_result_summarized subagent_mailbox::tests::test_boundary_4000_inline_vs_offloaded subagent_mailbox::tests::test_boundary_8000_offloaded_vs_summarized 2>&1 | tail -20
```
Expected: 3 个测试 PASS。

- [ ] **Step 7: 运行全量 mailbox 测试确认无回归**

Run:
```bash
cargo test --lib subagent_mailbox 2>&1 | tail -20
```
Expected: 所有 mailbox 测试 PASS,包括:
- `test_small_result_stays_inline`(≤4000 → Inline)
- `test_large_result_offloaded_with_full_content`(5000 → Offloaded,仍在 4000–8000 区间,行为不变)
- `test_offloaded_to_content_returns_full_result`(5000,footer 含 "persisted")
- `test_inline_to_content_unchanged`
- `test_summarized_to_content_has_summary_and_path`(Task 2)
- `test_summarized_summary_is_head_prefix`(Task 2)
- `test_very_large_result_summarized`(本 Task)
- `test_boundary_4000_inline_vs_offloaded`(本 Task)
- `test_boundary_8000_offloaded_vs_summarized`(本 Task)
- `test_to_compact_returns_full_content`、`test_offloaded_to_compact_returns_full_content`(仍存在,Task 5 删除)

- [ ] **Step 8: Commit**

```bash
git add src/teams/subagent_mailbox.rs
git commit -m "feat(subagent_mailbox): offload_if_large 3-tier dispatch (Inline/Offloaded/Summarized)

>4000 chars now split: 4000-8000 returns Offloaded (full inline + disk),
>8000 returns Summarized (1500-char head prefix + disk path footer).
Disk failure still degrades to Inline full content (integrity > token control).
Adds boundary tests at 4000/4001 and 8000/8001."
```

---

## Task 4: 磁盘持久化失败降级测试(R4)

**Files:**
- Modify: `src/teams/subagent_mailbox.rs` `#[cfg(test)] mod tests`:新增 1 个测试

**Interfaces:**
- Consumes: Task 3 的 `offload_if_large()` 三档逻辑(Err 分支已返回 `Inline`)
- Produces: 验证 R4(Persistence failure does not lose content)对 `>8000` 场景的覆盖

**背景**:Task 3 的 `offload_if_large()` Err 分支已实现降级(`Inline` 全量)。本任务新增测试覆盖 `>8000` + store 失败的 spec scenario(design doc Test Strategy "新增" 第 4 项)。用不可写目录模拟 `store()` 失败。

- [ ] **Step 1: 写测试 — >8000 + store 失败 → Inline 全量**

在 `src/teams/subagent_mailbox.rs` 的 `#[cfg(test)] mod tests` 内新增:

```rust
    #[test]
    fn test_disk_persistence_failure_degrades_to_inline() {
        // Use a path that cannot be created/written to simulate store() failure.
        // On most Unix systems, writing under a non-existent root path fails.
        let bad_mailbox = SubagentResultMailbox::new(
            PathBuf::from("/this/path/does/not/exist/wgenty_test_bad_mailbox"),
        );
        let very_large = "C".repeat(9000);
        let response = bad_mailbox.offload_if_large(
            "general-purpose",
            "disk fail test",
            "session_fail",
            &very_large,
        );
        match response {
            SubagentResponse::Inline { content } => {
                // Full content returned despite >8000 + disk failure —
                // integrity > token control (spec R4).
                assert_eq!(content.len(), 9000);
                assert_eq!(content, very_large);
            }
            _ => panic!(
                "Expected Inline (full content) on disk persistence failure, even for >8000"
            ),
        }
    }
```

- [ ] **Step 2: 运行测试确认通过**

Run:
```bash
cargo test --lib subagent_mailbox::tests::test_disk_persistence_failure_degrades_to_inline 2>&1 | tail -20
```
Expected: PASS。`store()` 在 `/this/path/does/not/exist/...` 下创建文件会返回 `Err`,`offload_if_large` 的 Err 分支返回 `Inline { content }` 全量。

**如果测试 FAIL**(例如某些环境下 `/this/path/...` 可被 root 创建,或 `SubagentResultMailbox::new` 的 `create_dir_all` 静默失败但 `store` 仍成功):改用更可靠的失败注入 —— 构造一个 `base_dir` 指向一个**已存在的文件**(非目录),使 `std::fs::write` 在其下创建子文件失败:
```rust
let existing_file = std::env::temp_dir().join("wgenty_test_bad_mailbox_file");
std::fs::write(&existing_file, "blocker").unwrap();
let bad_mailbox = SubagentResultMailbox::new(existing_file);
```
重新运行直到 PASS。

- [ ] **Step 3: 运行全量 mailbox 测试确认无回归**

Run:
```bash
cargo test --lib subagent_mailbox 2>&1 | tail -20
```
Expected: 所有 mailbox 测试 PASS。

- [ ] **Step 4: Commit**

```bash
git add src/teams/subagent_mailbox.rs
git commit -m "test(subagent_mailbox): disk persistence failure degrades to Inline (>8000)

Covers spec R4: >8000-char result + store() failure returns Inline full
content (no truncation), integrity prioritized over token control."
```

---

## Task 5: 删除 dead code to_compact() + 其两个测试

**Files:**
- Modify: `src/teams/subagent_mailbox.rs`
  - 删除 `to_compact()` 方法(lines 94-101)
  - 删除 `test_to_compact_returns_full_content`(lines 312-321)
  - 删除 `test_offloaded_to_compact_returns_full_content`(lines 323-332)

**Interfaces:**
- Consumes: 无
- Produces: 移除 dead code `to_compact()`

**背景**:design doc "现状分析" 第 2 点已确认 `to_compact()` 是 dead code —— grep 全仓无外部调用方(仅 `subagent_mailbox.rs` 内部定义 + 2 测试)。compaction 走 `do_auto_compact`(全局 LLM summary,`src/tui/agent/compaction.rs:107`),不调 `to_compact`。删除无功能影响。

- [ ] **Step 1: 确认 to_compact 无外部调用方(删除前最后核验)**

Run:
```bash
grep -rn "to_compact" src/ --include="*.rs"
```
Expected: 仅命中 `src/teams/subagent_mailbox.rs` 内部(方法定义 + 2 测试调用)。无 `src/tools/`、`src/tui/`、`src/daemon/` 等外部调用。若出现外部调用,**停止本任务**并上报(design doc dead code 结论有误)。

- [ ] **Step 2: 删除 to_compact() 方法**

删除 `src/teams/subagent_mailbox.rs` 中的整个 `to_compact()` 方法(含 doc 注释,原 lines 94-101):

```rust
    /// Return the full content without truncation.
    ///
    /// Previously this truncated large inline results for compaction safety.
    /// Subagent results are considered important and must never be truncated,
    /// so this now delegates to [`to_content`](Self::to_content).
    pub fn to_compact(&self) -> String {
        self.to_content()
    }
```

删除后,`to_content()` 与 `len()` 之间应直接相邻(无空段多余空行)。

- [ ] **Step 3: 删除 test_to_compact_returns_full_content**

删除整个测试函数:

```rust
    #[test]
    fn test_to_compact_returns_full_content() {
        let large = "X".repeat(5000);
        let resp = SubagentResponse::Inline {
            content: large.clone(),
        };
        let compact = resp.to_compact();
        // No truncation — full content preserved.
        assert_eq!(compact, large);
    }
```

- [ ] **Step 4: 删除 test_offloaded_to_compact_returns_full_content**

删除整个测试函数:

```rust
    #[test]
    fn test_offloaded_to_compact_returns_full_content() {
        let mailbox =
            SubagentResultMailbox::new(std::env::temp_dir().join("wgenty_test_mailbox_compact"));
        let large_content = "Y".repeat(5000);
        let response = mailbox.offload_if_large("plan", "compact test", "session4", &large_content);
        let compact = response.to_compact();
        // Full content preserved, no truncation.
        assert!(compact.contains(&large_content));
    }
```

- [ ] **Step 5: 确认无残留 to_compact 引用**

Run:
```bash
grep -n "to_compact" src/teams/subagent_mailbox.rs
```
Expected: 无输出(方法与测试均已删除)。

Run:
```bash
grep -rn "to_compact" src/ --include="*.rs"
```
Expected: 无输出(全仓无 `to_compact` 引用)。

- [ ] **Step 6: cargo build 确认编译通过**

Run:
```bash
cargo build 2>&1 | tail -10
```
Expected: `Finished` 无错误。删除 dead code 不影响编译。

- [ ] **Step 7: cargo test --lib 确认无回归**

Run:
```bash
cargo test --lib 2>&1 | tail -20
```
Expected: 所有测试 PASS。`to_compact` 的 2 个测试已删除,其余测试不受影响。

- [ ] **Step 8: cargo clippy --lib 确认无新警告**

Run:
```bash
cargo clippy --lib 2>&1 | tail -10
```
Expected: 无 warning、无 error。

- [ ] **Step 9: Commit**

```bash
git add src/teams/subagent_mailbox.rs
git commit -m "refactor(subagent_mailbox): remove dead code to_compact() + its 2 tests

to_compact() was never called externally (grep confirms zero callers
outside subagent_mailbox.rs). Compaction goes through do_auto_compact
(global LLM summary), not to_compact. Removes the method and its two
tests (test_to_compact_returns_full_content,
test_offloaded_to_compact_returns_full_content). No functional change."
```

---

## Task 6: 调用方无改动确认 + 最终全量验证

**Files:**
- Verify: `src/tools/meta/task.rs`(零改动)
- Verify: `src/tui/agent/compaction.rs`(零改动)
- Verify: `src/teams/subagent_mailbox.rs`(Task 2–5 改动已完成)

**Interfaces:**
- Consumes: Task 2–5 的全部改动
- Produces: 确认 spec 全部 4 个 Requirements 通过测试覆盖;`to_content()` 签名不变,调用方零改动

- [ ] **Step 1: 确认调用方 to_content() 签名未变,零改动**

Run:
```bash
grep -n "offload_if_large\|to_content\|to_compact" src/tools/meta/task.rs
```
Expected: 命中 `src/tools/meta/task.rs:558`(`offload_if_large` 后台调用)、`:559`(`.to_content()`)、`:704`(`offload_if_large` 同步调用)、`:749`(`response.to_content()`)。**无 `to_compact` 引用**。`to_content()` 签名仍是 `pub fn to_content(&self) -> String`(未变),调用方零改动。

- [ ] **Step 2: 确认 compaction 未被本 change 改动**

Run:
```bash
git diff --stat 94fe946430da339e979f2d1fe111032cb6ac2159 -- src/tui/agent/compaction.rs
```
Expected: 无输出(B3 不依赖 compaction,compaction.rs 零改动)。

- [ ] **Step 3: cargo build 全量编译**

Run:
```bash
cargo build 2>&1 | tail -10
```
Expected: `Finished` 无错误。

- [ ] **Step 4: cargo test --lib 全量测试**

Run:
```bash
cargo test --lib 2>&1 | tail -30
```
Expected: 所有测试 PASS。subagent_mailbox 模块测试清单(共 9 个,2 个旧 to_compact 测试已删 + 6 个新增 + 1 个降级):
- `test_small_result_stays_inline`(R1 ≤4000 → Inline)
- `test_large_result_offloaded_with_full_content`(R1 4000–8000 → Offloaded 全量)
- `test_offloaded_to_content_returns_full_result`(R3 footer 含 persisted + path)
- `test_inline_to_content_unchanged`
- `test_summarized_to_content_has_summary_and_path`(R3 Summarized footer 含 path + file_read)
- `test_summarized_summary_is_head_prefix`(R2 summary 是头 1500 字前缀)
- `test_very_large_result_summarized`(R1/R2 >8000 → Summarized)
- `test_boundary_4000_inline_vs_offloaded`(R2 边界 4000/4001)
- `test_boundary_8000_offloaded_vs_summarized`(R2 边界 8000/8001)
- `test_disk_persistence_failure_degrades_to_inline`(R4 磁盘失败 → Inline 全量)

- [ ] **Step 5: cargo clippy --lib 无新警告**

Run:
```bash
cargo clippy --lib 2>&1 | tail -10
```
Expected: 无 warning、无 error。

- [ ] **Step 6: Spec scenario 覆盖核验**

对照 `openspec/changes/split-api-and-subagent-result-relief/specs/subagent-result-delivery/spec.md` 的 4 个 Requirements:

| Requirement | Scenario | 覆盖测试 |
|---|---|---|
| R1: Large results accessible without loss | Parent agent can recover full content | `test_large_result_offloaded_with_full_content` + `test_very_large_result_summarized`(磁盘可恢复) |
| R1: Large results accessible without loss | Large result not replaced by short prefix-only summary | `test_very_large_result_summarized`(summary 是 1500 字,非 200 字;且有磁盘全文) |
| R2: Delivery controls token cost | Full content not unconditionally inlined | `test_very_large_result_summarized` + `test_boundary_8000_offloaded_vs_summarized`(>8000 不全量 inline) |
| R3: Disk persistence for recovery | Large result persisted to disk | `test_offloaded_to_content_returns_full_result` + `test_summarized_to_content_has_summary_and_path`(footer 含 path) |
| R3: Disk persistence for recovery | Recovery path communicated to parent | 同上(footer 通信 path) |
| R4: Persistence failure does not lose content | Persistence failure returns full content inline + logged | `test_disk_persistence_failure_degrades_to_inline` |

确认所有 scenario 均有测试覆盖。无遗漏。

- [ ] **Step 7: 确认 git 工作区干净(所有改动已 commit)**

Run:
```bash
git status
```
Expected: `nothing to commit, working tree clean`(Change A 在 Task 1 commit,Change B 在 Task 2–5 commit;本任务无新改动)。

Run:
```bash
git log --oneline 94fe946430da339e979f2d1fe111032cb6ac2159..HEAD
```
Expected: 5 个 commit(refactor(api) split + feat Summarized variant + feat offload_if_large 3-tier + test disk failure + refactor remove to_compact)。

---

## Self-Review 核验

**1. Spec coverage**(对照 design doc "与 spec requirements 的映射"):
- R1(Large results accessible without loss):Task 2(Summarized 变体)+ Task 3(`offload_if_large` 三档)+ `test_large_result_offloaded_with_full_content` / `test_very_large_result_summarized`。✅
- R2(Delivery controls token cost):Task 3(`MAX_FULL_INLINE_LEN=8000` 上限)+ `test_very_large_result_summarized` / `test_boundary_8000_offloaded_vs_summarized`。✅
- R3(Disk persistence for recovery):Task 3(`store()` 写 JSONL)+ Task 2(to_content footer 含 path)+ `test_offloaded_to_content_returns_full_result` / `test_summarized_to_content_has_summary_and_path`。✅
- R4(Persistence failure does not lose content):Task 3(Err 分支降级 Inline)+ Task 4(`test_disk_persistence_failure_degrades_to_inline`)。✅

**2. Placeholder scan**:无 TBD/TODO/"implement later"/"add appropriate error handling" 等。所有代码步骤均含完整代码。✅

**3. Type consistency**:
- `SubagentResponse::Summarized { summary: String, mailbox_path: PathBuf, content_len: usize }` —— Task 2 定义、Task 3 构造、Task 4 match 解构,字段名一致。✅
- `MAX_FULL_INLINE_LEN`/`SUMMARY_HEAD_LEN` —— Task 2 定义、Task 3 使用,名称一致。✅
- `to_content()` 签名 `&self -> String` —— Task 2 更新、Task 6 确认调用方未变。✅
- `offload_if_large()` 签名 `&self, &str, &str, &str, &str -> SubagentResponse` —— Task 3 实现、Task 6 确认 `task.rs` 调用方未变。✅
- footer 文本逐字匹配 design doc(含 em-dash `—`、backtick 包裹 path)。✅
