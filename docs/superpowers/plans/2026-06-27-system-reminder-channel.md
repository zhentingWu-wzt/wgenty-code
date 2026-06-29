---
change: system-reminder-channel
design-doc: docs/superpowers/specs/2026-06-27-system-reminder-channel-design.md
base-ref: f6bbb1e23a4a840a195820bc4e4bae896530babe
---

# System Reminder Channel 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 wgenty-code 中提供与 Claude Code 等价的 `<system-reminder>` 注入通道：把用户级/项目级文件源 + UserPromptSubmit hook 注入聚合到每轮 user message 头部，硬切移除现有 Layer 7/8。

**Architecture:** 新增 `build_user_turn_reminder(ctx, hook_injections) -> Option<ReminderOutput>`（in `src/prompts/mod.rs`），由 `AgentLoop::process_input_inner` 在 `await` hook fire 之后调用，把 `to_model` 拼到 user message content 头部、`to_transcript` 投递到 TUI；Layer 7/8 push 删除；新增 4 个文件源 reader 与 1 个 hook 收集函数。

**Tech Stack:** Rust 1.x、tokio、`dirs::home_dir()`、现有 `HookManager` / `tokio::process::Command` 子进程链路、`tracing`。

## Global Constraints

- 严格 1:1 对齐 Claude Code 的 `<system-reminder>` 文本骨架（措辞、缩进、空行、来源标注），见设计文档 D6；偏离需在 PR 中给出明确理由。
- 标题使用 `# wgentyMd`（D1 = B 已确认，非 `# claudeMd`）。
- Hook 范围仅消费 `UserPromptSubmit`（D5）；其它 hook 事件不参与 reminder。
- BREAKING：v0.1.0 阶段硬切，不保留向后兼容、不引入 feature flag、不需要数据迁移；回滚 = `git revert`。
- Subagent 路径必须**不调用** `build_user_turn_reminder`（已知限制 K2）。
- Hook fire `await` 超时 10s；超时 → warning log + 空 outcomes + turn 继续（D7 / R6）。
- Token 预算警告每 session 仅触发一次；hook 注入**不计入**预算（D9 / T4）。
- 输出语言：中文（plan、commit message 主题行允许英文动词前缀如 `feat:` / `fix:`）。

设计文档锚点：决策 D1-D9 在 §2、实现细节在 §3、测试策略在 §5、Spec Patch 在 §8。

---

## 文件结构

```
src/
├── prompts/mod.rs                 # 新增 ReminderOutput / build_user_turn_reminder / 常量 / 移除 Layer 7,8
├── runtime/hooks/mod.rs           # 新增 InjectedFragment / collect_injections / 扩展 HookOutcome 字段
├── utils/project.rs               # 新增 read_user_global_instructions / read_user_global_rules
├── tui/
│   ├── agent/mod.rs               # process_input_inner: fire hook + build reminder + 拼 user content
│   └── app/
│       ├── input.rs               # 删除 fire-and-forget UserPromptSubmit
│       └── mod.rs                 # 改造 token 预算警告 + 调用 with_project_root
└── ...
tests/
└── system_reminder.rs             # 新增 7 个集成测
```

---

## §1 数据结构与 readers

### Task 1.1: 新增 `InjectedFragment` 与 `collect_injections`

**对应 tasks.md**: 1.1
**设计依据**: D5（§2.5）+ §3.1 代码骨架
**前置依赖**: 无

**Files:**
- Modify: `src/runtime/hooks/mod.rs`（在 `HookOutcome` 附近新增结构 + 函数；在 mod tests 内增 3 个单测）

**Interfaces produced:**
- `pub struct InjectedFragment { pub content: String, pub priority: u8, pub visibility: LayerVisibility, pub source_label: String }`
- `pub fn collect_injections(outcomes: &[HookOutcome]) -> Vec<InjectedFragment>`
- 为支持 priority/visibility 从 outcome 读取，给 `HookOutcome` 新增字段 `pub injection_priority: Option<u8>`、`pub injection_visibility: Option<LayerVisibility>`（由 `run_inject_action` 在产出 outcome 时填充，见 Task 5.1）。

- [x] **Step 1: 阅读 `src/runtime/hooks/mod.rs` 中 `HookOutcome`、`HookAction::InjectContext`、`LayerVisibility` 现有定义（约 50-110 行、780-800 行）。**

- [x] **Step 2: 在 `HookOutcome` 定义处新增两个可选字段：**

```rust
pub struct HookOutcome {
    pub def: HookDefinition,
    pub continue_execution: bool,
    pub reason: Option<String>,
    pub injected_content: Option<String>,
    pub user_answer: Option<UserAnswer>,
    // 新增：当 outcome 来自 InjectContext 时填充
    pub injection_priority: Option<u8>,
    pub injection_visibility: Option<LayerVisibility>,
}
```

- [x] **Step 3: 全仓修复 `HookOutcome` 字面构造点（编译器报错驱动），把两个新字段统一加 `None` 占位。**

运行: `cargo check 2>&1 | grep -A2 "missing.*injection"` 直到无错。

- [x] **Step 4: 在 `LayerVisibility` 后新增 `InjectedFragment`：**

```rust
#[derive(Debug, Clone)]
pub struct InjectedFragment {
    pub content: String,
    pub priority: u8,
    pub visibility: LayerVisibility,
    pub source_label: String,
}
```

- [x] **Step 5: 在同文件新增 `collect_injections`：**

```rust
pub fn collect_injections(outcomes: &[HookOutcome]) -> Vec<InjectedFragment> {
    let mut out: Vec<InjectedFragment> = outcomes
        .iter()
        .enumerate()
        .filter_map(|(idx, oc)| {
            let content = oc.injected_content.as_ref()?;
            if content.is_empty() {
                return None;
            }
            Some(InjectedFragment {
                content: content.clone(),
                priority: oc.injection_priority.unwrap_or(50),
                visibility: oc
                    .injection_visibility
                    .clone()
                    .unwrap_or(LayerVisibility::Visible),
                source_label: format!("hook:UserPromptSubmit:{idx}"),
            })
        })
        .collect();
    out.sort_by_key(|f| f.priority); // stable sort: ties 保留传入顺序
    out
}
```

- [x] **Step 6: 在 `mod tests` 内追加三个单测（空、单 outcome、多 outcome 排序）：**

```rust
#[test]
fn collect_injections_empty_outcomes_returns_empty() {
    assert!(collect_injections(&[]).is_empty());
}

#[test]
fn collect_injections_single_outcome_extracts_fragment() {
    let outcome = HookOutcome {
        def: HookDefinition {
            event: HookEvent::UserPromptSubmit,
            matcher: None,
            when_state: None,
            actions: vec![],
        },
        continue_execution: true,
        reason: None,
        injected_content: Some("hello".into()),
        user_answer: None,
        injection_priority: Some(20),
        injection_visibility: Some(LayerVisibility::Internal),
    };
    let frags = collect_injections(&[outcome]);
    assert_eq!(frags.len(), 1);
    assert_eq!(frags[0].content, "hello");
    assert_eq!(frags[0].priority, 20);
    assert_eq!(frags[0].source_label, "hook:UserPromptSubmit:0");
    matches!(frags[0].visibility, LayerVisibility::Internal);
}

#[test]
fn collect_injections_sorts_by_priority_stable() {
    let mk = |content: &str, prio: u8| HookOutcome {
        def: HookDefinition {
            event: HookEvent::UserPromptSubmit,
            matcher: None,
            when_state: None,
            actions: vec![],
        },
        continue_execution: true,
        reason: None,
        injected_content: Some(content.into()),
        user_answer: None,
        injection_priority: Some(prio),
        injection_visibility: Some(LayerVisibility::Visible),
    };
    let outcomes = vec![mk("low2", 30), mk("high", 10), mk("low1", 30)];
    let frags = collect_injections(&outcomes);
    assert_eq!(
        frags.iter().map(|f| f.content.as_str()).collect::<Vec<_>>(),
        vec!["high", "low2", "low1"]
    );
}
```

- [x] **Step 7: 运行单测**

```bash
cargo test -p wgenty-code collect_injections
```

期望: 3 个测试 PASS。

- [x] **Step 8: Commit**

```bash
git add src/runtime/hooks/mod.rs
git commit -m "feat(hooks): add InjectedFragment and collect_injections"
```

---

### Task 1.2: 新增 `read_user_global_instructions`

**对应 tasks.md**: 1.2
**设计依据**: D3
**前置依赖**: 无

**Files:**
- Modify: `src/utils/project.rs`

**Interfaces produced:**
- `pub fn read_user_global_instructions() -> Option<(PathBuf, String)>`

- [x] **Step 1: 在 `src/utils/project.rs` 顶部确认 `use std::path::PathBuf;` 存在；引入 `dirs` 依赖（已存在则跳过，否则在 `Cargo.toml` 添加 `dirs = "5"` 并 `cargo check`）。**

- [x] **Step 2: 在 `read_md_sections` 之后追加：**

```rust
/// 读取用户级全局指令：`~/.wgenty-code/WGENTY.md`。
/// 文件缺失、读失败、内容为空时返回 None。
pub fn read_user_global_instructions() -> Option<(PathBuf, String)> {
    let home = dirs::home_dir()?;
    let path = home.join(".wgenty-code").join("WGENTY.md");
    let content = std::fs::read_to_string(&path).ok()?;
    if content.is_empty() {
        None
    } else {
        Some((path, content))
    }
}
```

- [x] **Step 3: 在文件底部 `#[cfg(test)] mod tests` 中追加（若无 tests 模块则新建）：**

```rust
#[cfg(test)]
mod user_instr_tests {
    use super::*;
    use tempfile::TempDir;

    fn with_fake_home<F: FnOnce()>(home: &std::path::Path, f: F) {
        let prev = std::env::var_os("HOME");
        std::env::set_var("HOME", home);
        f();
        match prev {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }

    #[test]
    fn user_instructions_present() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join(".wgenty-code");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("WGENTY.md"), "hello").unwrap();
        with_fake_home(tmp.path(), || {
            let got = read_user_global_instructions();
            assert!(got.is_some());
            assert_eq!(got.unwrap().1, "hello");
        });
    }

    #[test]
    fn user_instructions_missing_returns_none() {
        let tmp = TempDir::new().unwrap();
        with_fake_home(tmp.path(), || {
            assert!(read_user_global_instructions().is_none());
        });
    }

    #[test]
    fn user_instructions_empty_returns_none() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join(".wgenty-code");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("WGENTY.md"), "").unwrap();
        with_fake_home(tmp.path(), || {
            assert!(read_user_global_instructions().is_none());
        });
    }
}
```

> 备注: `dirs::home_dir()` 在 Unix 上读取 `HOME` 环境变量；测试通过临时改 `HOME` 实现 sandbox。`tempfile` 可能需要在 `Cargo.toml` `[dev-dependencies]` 添加。

- [x] **Step 4: 运行**

```bash
cargo test -p wgenty-code user_instr_tests
```

期望: 3 个测试 PASS。

- [x] **Step 5: Commit**

```bash
git add src/utils/project.rs Cargo.toml Cargo.lock
git commit -m "feat(project): read user global WGENTY.md from ~/.wgenty-code"
```

---

### Task 1.3: 新增 `read_user_global_rules`

**对应 tasks.md**: 1.3
**设计依据**: D3
**前置依赖**: Task 1.2（共用 `dirs` 与测试 helper）

**Files:**
- Modify: `src/utils/project.rs`

**Interfaces produced:**
- `pub fn read_user_global_rules() -> Vec<(PathBuf, String)>`（按 `file_name` 字母序）

- [x] **Step 1: 在 `read_user_global_instructions` 之后追加：**

```rust
/// 扫 `~/.wgenty-code/rules/` 顶层 `.md` 文件（忽略子目录、非 .md、空文件），
/// 按文件名字母序返回 `(绝对路径, 内容)`。
pub fn read_user_global_rules() -> Vec<(PathBuf, String)> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    let rules_dir = home.join(".wgenty-code").join("rules");
    let Ok(entries) = std::fs::read_dir(&rules_dir) else {
        return Vec::new();
    };
    let mut files: Vec<std::fs::DirEntry> = entries
        .filter_map(|e| e.ok())
        .filter(|e| {
            let is_file = e.file_type().ok().map(|t| t.is_file()).unwrap_or(false);
            let is_md = e
                .path()
                .extension()
                .map(|x| x == "md")
                .unwrap_or(false);
            is_file && is_md
        })
        .collect();
    files.sort_by_key(|e| e.file_name());
    files
        .into_iter()
        .filter_map(|e| {
            let path = e.path();
            let content = std::fs::read_to_string(&path).ok()?;
            if content.is_empty() {
                None
            } else {
                Some((path, content))
            }
        })
        .collect()
}
```

- [x] **Step 2: 追加单测：**

```rust
#[cfg(test)]
mod user_rules_tests {
    use super::*;
    use tempfile::TempDir;

    fn with_fake_home<F: FnOnce()>(home: &std::path::Path, f: F) {
        let prev = std::env::var_os("HOME");
        std::env::set_var("HOME", home);
        f();
        match prev {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }

    #[test]
    fn rules_missing_dir_returns_empty() {
        let tmp = TempDir::new().unwrap();
        with_fake_home(tmp.path(), || {
            assert!(read_user_global_rules().is_empty());
        });
    }

    #[test]
    fn rules_alphabetical_order() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join(".wgenty-code").join("rules");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("zebra.md"), "Z").unwrap();
        std::fs::write(dir.join("alpha.md"), "A").unwrap();
        std::fs::write(dir.join("middle.md"), "M").unwrap();
        with_fake_home(tmp.path(), || {
            let rules = read_user_global_rules();
            let names: Vec<_> = rules
                .iter()
                .map(|(p, _)| p.file_name().unwrap().to_string_lossy().to_string())
                .collect();
            assert_eq!(names, vec!["alpha.md", "middle.md", "zebra.md"]);
        });
    }

    #[test]
    fn rules_ignores_subdirs_and_non_md() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join(".wgenty-code").join("rules");
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub").join("nested.md"), "nope").unwrap();
        std::fs::write(dir.join("note.txt"), "nope").unwrap();
        std::fs::write(dir.join("ok.md"), "ok").unwrap();
        with_fake_home(tmp.path(), || {
            let rules = read_user_global_rules();
            assert_eq!(rules.len(), 1);
            assert_eq!(rules[0].1, "ok");
        });
    }
}
```

- [x] **Step 3: 运行**

```bash
cargo test -p wgenty-code user_rules_tests
```

期望: 3 个测试 PASS。

- [x] **Step 4: Commit**

```bash
git add src/utils/project.rs
git commit -m "feat(project): read user global rules from ~/.wgenty-code/rules"
```

---

### Task 1.4: `PromptContext` 新增 `project_root` + builder

**对应 tasks.md**: 1.4
**设计依据**: D4
**前置依赖**: 无

**Files:**
- Modify: `src/prompts/mod.rs`

**Interfaces produced:**
- `PromptContext.project_root: Option<PathBuf>`
- `pub fn with_project_root(self, path: PathBuf) -> Self`

- [x] **Step 1: 在 `PromptContext` struct 内追加字段：**

```rust
pub project_root: Option<PathBuf>,
```

并在 `use` 区补 `use std::path::PathBuf;`。

- [x] **Step 2: 更新 `Default` / `PromptContext::new()`：默认 `project_root: None`；更新 `Debug` impl 增加该字段。**

- [x] **Step 3: 新增 builder 方法：**

```rust
impl PromptContext {
    pub fn with_project_root(mut self, path: PathBuf) -> Self {
        self.project_root = Some(path);
        self
    }
}
```

- [x] **Step 4: 运行 `cargo check`，修复字面构造（如 `PromptContext { ... }`）报错为追加 `project_root: None`。**

- [x] **Step 5: 新增单测：**

```rust
#[test]
fn prompt_context_with_project_root_sets_field() {
    let ctx = PromptContext::new().with_project_root(PathBuf::from("/repo"));
    assert_eq!(ctx.project_root.as_deref(), Some(std::path::Path::new("/repo")));
}
```

- [x] **Step 6: 运行**

```bash
cargo test -p wgenty-code prompt_context_with_project_root_sets_field
```

期望: PASS。

- [x] **Step 7: Commit**

```bash
git add src/prompts/mod.rs
git commit -m "feat(prompts): add project_root field to PromptContext"
```

---

## §2 Reminder builder

### Task 2.1: 新增 reminder 文本常量

**对应 tasks.md**: 2.1
**设计依据**: D6 + §3.1
**前置依赖**: 无

**Files:**
- Modify: `src/prompts/mod.rs`（文件顶部常量段）

- [x] **Step 1: 在 `BASE_INSTRUCTIONS` 常量下追加 reminder preamble 常量（与设计文档 §3.1 完全一致；闭合 preamble 6 空格缩进精确复刻）：**

```rust
const REMINDER_PREAMBLE_OPENING: &str =
    "As you answer the user's questions, you can use the following context:\n\
     # wgentyMd\n\
     Codebase and user instructions are shown below. Be sure to adhere to\n\
     these instructions. IMPORTANT: These instructions OVERRIDE any default\n\
     behavior and you MUST follow them exactly as written.\n";

const REMINDER_PREAMBLE_CLOSING: &str =
    "      IMPORTANT: this context may or may not be relevant to your tasks.\n\
     \x20     You should not respond to this context unless it is highly relevant\n\
     \x20     to your task.";

// 来源标注 description 常量
const USER_INSTRUCTIONS_DESC: &str = "user's private global instructions for all projects";
const PROJECT_INSTRUCTIONS_DESC: &str = "project instructions, checked into the codebase";
const PROJECT_AGENTS_DESC: &str = "project agent conventions, checked into the codebase";
const HOOK_INJECTION_DESC: &str = "dynamic hook injection";
```

- [x] **Step 2: `cargo check` 确保通过（常量未使用会有 dead_code warning，下一 task 消除）。**

- [x] **Step 3: 不单独 commit，与 Task 2.2 一起提交。**

---

### Task 2.2: 实现 `build_user_turn_reminder` + `ReminderOutput`

**对应 tasks.md**: 2.2
**设计依据**: D2 + D6 + D8 + §3.3
**前置依赖**: Task 1.1（`InjectedFragment`）、1.2、1.3、1.4、2.1

**Files:**
- Modify: `src/prompts/mod.rs`

**Interfaces produced:**
- `pub struct ReminderOutput { pub to_model: String, pub to_transcript: Option<String> }`
- `pub fn build_user_turn_reminder(ctx: &PromptContext, hook_injections: &[InjectedFragment]) -> Option<ReminderOutput>`

- [x] **Step 1: 在 `src/prompts/mod.rs` 引入：**

```rust
use crate::runtime::hooks::{InjectedFragment, LayerVisibility};
use crate::utils::project::{read_user_global_instructions, read_user_global_rules};
use std::path::Path;
```

- [x] **Step 2: 新增 `ReminderOutput`：**

```rust
#[derive(Debug, Clone)]
pub struct ReminderOutput {
    pub to_model: String,
    pub to_transcript: Option<String>,
}
```

- [x] **Step 3: 实现 builder（按 §3.3 完整逻辑）：**

```rust
pub fn build_user_turn_reminder(
    ctx: &PromptContext,
    hook_injections: &[InjectedFragment],
) -> Option<ReminderOutput> {
    // ── 收集文件段 ──
    struct Segment {
        path: PathBuf,
        description: &'static str,
        content: String,
    }
    let mut segments: Vec<Segment> = Vec::new();

    if let Some((path, content)) = read_user_global_instructions() {
        segments.push(Segment {
            path,
            description: USER_INSTRUCTIONS_DESC,
            content,
        });
    }
    for (path, content) in read_user_global_rules() {
        segments.push(Segment {
            path,
            description: USER_INSTRUCTIONS_DESC,
            content,
        });
    }
    if !ctx.wgenty_md_sections.is_empty() {
        let path = ctx
            .project_root
            .as_ref()
            .map(|p| p.join("WGENTY.md"))
            .unwrap_or_else(|| PathBuf::from("WGENTY.md"));
        let content = ctx.wgenty_md_sections.join("\n\n");
        segments.push(Segment {
            path,
            description: PROJECT_INSTRUCTIONS_DESC,
            content,
        });
    }
    if !ctx.agents_md_sections.is_empty() {
        let path = ctx
            .project_root
            .as_ref()
            .map(|p| p.join("AGENTS.md"))
            .unwrap_or_else(|| PathBuf::from("AGENTS.md"));
        let content = ctx.agents_md_sections.join("\n\n");
        segments.push(Segment {
            path,
            description: PROJECT_AGENTS_DESC,
            content,
        });
    }

    if segments.is_empty() && hook_injections.is_empty() {
        return None;
    }

    // ── 渲染 ──
    let mut to_model = String::from("<system-reminder>\n");
    let mut to_transcript = String::from("<system-reminder>\n");
    to_model.push_str(REMINDER_PREAMBLE_OPENING);
    to_transcript.push_str(REMINDER_PREAMBLE_OPENING);

    for seg in &segments {
        let header = render_attribution_header(&seg.path, seg.description);
        let block = format!("\n{}\n\n{}\n", header, seg.content);
        to_model.push_str(&block);
        to_transcript.push_str(&block);
    }

    let mut transcript_has_hook = false;
    for frag in hook_injections {
        let header = format!("Contents of {} ({}):", frag.source_label, HOOK_INJECTION_DESC);
        let block = format!("\n{}\n\n{}\n", header, frag.content);
        to_model.push_str(&block);
        if matches!(frag.visibility, LayerVisibility::Visible) {
            to_transcript.push_str(&block);
            transcript_has_hook = true;
        }
    }

    to_model.push('\n');
    to_model.push_str(REMINDER_PREAMBLE_CLOSING);
    to_model.push_str("\n</system-reminder>");

    to_transcript.push('\n');
    to_transcript.push_str(REMINDER_PREAMBLE_CLOSING);
    to_transcript.push_str("\n</system-reminder>");

    let transcript_has_content = !segments.is_empty() || transcript_has_hook;
    Some(ReminderOutput {
        to_model,
        to_transcript: if transcript_has_content {
            Some(to_transcript)
        } else {
            None
        },
    })
}
```

- [x] **Step 4: 运行 `cargo check`，修复编译错。**

- [x] **Step 5: Commit（与 Task 2.1 合并）**

```bash
git add src/prompts/mod.rs
git commit -m "feat(prompts): add build_user_turn_reminder with dual-track output"
```

---

### Task 2.3: 来源标注辅助函数 `render_attribution_header`

**对应 tasks.md**: 2.3
**设计依据**: §3.1
**前置依赖**: 与 2.2 合并（同 commit）

**Files:**
- Modify: `src/prompts/mod.rs`

- [x] **Step 1: 在 `build_user_turn_reminder` 上方新增：**

```rust
fn render_attribution_header(absolute_path: &Path, description: &str) -> String {
    format!("Contents of {} ({}):", absolute_path.display(), description)
}
```

- [x] **Step 2: 已在 Task 2.2 步骤 3 内被使用。运行 `cargo check`。**

- [x] **Step 3: 不单独 commit，并入 2.2 的 commit。**

---

### Task 2.4: 单测——完整 reminder 快照

**对应 tasks.md**: 2.4
**设计依据**: 测试策略 §5.1 U1
**前置依赖**: 2.1–2.3

**Files:**
- Modify: `src/prompts/mod.rs`（`tests` 模块）

- [x] **Step 1: 在 `tests` 模块底部追加 helper（构造 fake HOME + project root + ctx）：**

```rust
#[cfg(test)]
mod reminder_tests {
    use super::*;
    use crate::runtime::hooks::{InjectedFragment, LayerVisibility};
    use tempfile::TempDir;

    fn with_fake_home<F: FnOnce() -> R, R>(home: &std::path::Path, f: F) -> R {
        let prev = std::env::var_os("HOME");
        std::env::set_var("HOME", home);
        let r = f();
        match prev {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        r
    }

    fn make_ctx_with_project_md(root: PathBuf) -> PromptContext {
        PromptContext::new()
            .with_wgenty_md(vec!["proj wgenty content".into()])
            .with_agents_md(vec!["proj agents content".into()])
            .with_project_root(root)
    }

    #[test]
    fn reminder_full_four_sources_snapshot() {
        let tmp = TempDir::new().unwrap();
        let user_dir = tmp.path().join(".wgenty-code");
        std::fs::create_dir_all(user_dir.join("rules")).unwrap();
        std::fs::write(user_dir.join("WGENTY.md"), "user-global").unwrap();
        std::fs::write(user_dir.join("rules").join("alpha.md"), "rule-alpha").unwrap();

        let project_root = tmp.path().join("proj");
        std::fs::create_dir_all(&project_root).unwrap();

        let ctx = make_ctx_with_project_md(project_root.clone());
        let out = with_fake_home(tmp.path(), || {
            build_user_turn_reminder(&ctx, &[]).expect("reminder Some")
        });

        // preamble 完整存在
        assert!(out.to_model.starts_with("<system-reminder>\n"));
        assert!(out.to_model.contains("# wgentyMd"));
        assert!(out.to_model.ends_with("</system-reminder>"));
        assert!(out.to_model.contains("IMPORTANT: this context may or may not be relevant"));

        // 顺序：user WGENTY → rules/alpha → project WGENTY → project AGENTS
        let order: Vec<_> = ["user-global", "rule-alpha", "proj wgenty content", "proj agents content"]
            .iter()
            .map(|needle| out.to_model.find(needle).expect(needle))
            .collect();
        assert!(order.windows(2).all(|w| w[0] < w[1]), "段顺序错: {:?}", order);

        // 来源标注是绝对路径
        let abs_user_wgenty = tmp.path().join(".wgenty-code").join("WGENTY.md");
        assert!(out.to_model.contains(&format!(
            "Contents of {} (user's private global instructions for all projects):",
            abs_user_wgenty.display()
        )));
    }
}
```

> 注：snapshot 测以"包含关键片段 + 顺序"代替全文 byte-for-byte 比较，避免临时目录路径变化导致脆弱。

- [x] **Step 2: 运行**

```bash
cargo test -p wgenty-code reminder_full_four_sources_snapshot
```

期望: PASS。失败时打印 `out.to_model` 排查。

- [x] **Step 3: 暂不 commit；与 2.5–2.8 合并提交。**

---

### Task 2.5: 单测——缺失源 + 全缺返回 None

**对应 tasks.md**: 2.5
**设计依据**: §5.1 U2/U3
**前置依赖**: 2.4

- [x] **Step 1: 在 `reminder_tests` 内追加：**

```rust
#[test]
fn reminder_missing_user_wgenty_no_empty_header() {
    let tmp = TempDir::new().unwrap();
    // 只有项目源；用户级缺失
    let project_root = tmp.path().join("proj");
    std::fs::create_dir_all(&project_root).unwrap();
    let ctx = make_ctx_with_project_md(project_root);

    let out = with_fake_home(tmp.path(), || {
        build_user_turn_reminder(&ctx, &[]).expect("Some")
    });

    // 无空白 "Contents of ():"
    assert!(!out.to_model.contains("Contents of  ("));
    assert!(!out.to_model.contains("user's private global instructions"));
}

#[test]
fn reminder_all_missing_returns_none() {
    let tmp = TempDir::new().unwrap();
    let ctx = PromptContext::new(); // 无任何文件段、无 project_root
    let result = with_fake_home(tmp.path(), || build_user_turn_reminder(&ctx, &[]));
    assert!(result.is_none());
}

#[test]
fn reminder_only_hooks_no_files_yields_block() {
    let tmp = TempDir::new().unwrap();
    let ctx = PromptContext::new();
    let frag = InjectedFragment {
        content: "from-hook".into(),
        priority: 10,
        visibility: LayerVisibility::Visible,
        source_label: "hook:UserPromptSubmit:0".into(),
    };
    let out = with_fake_home(tmp.path(), || {
        build_user_turn_reminder(&ctx, &[frag]).expect("Some")
    });
    assert!(out.to_model.contains("from-hook"));
    assert!(out.to_model.contains("hook:UserPromptSubmit:0"));
    assert!(out.to_transcript.is_some());
}
```

- [x] **Step 2: 运行**

```bash
cargo test -p wgenty-code reminder_missing_user_wgenty_no_empty_header reminder_all_missing_returns_none reminder_only_hooks_no_files_yields_block
```

期望: 3 PASS。

- [x] **Step 3: 暂不 commit。**

---

### Task 2.6: 单测——绝对路径

**对应 tasks.md**: 2.6
**设计依据**: §5.1 U5
**前置依赖**: 2.5

- [x] **Step 1: 追加：**

```rust
#[test]
fn reminder_absolute_paths_in_attribution() {
    let tmp = TempDir::new().unwrap();
    let project_root = tmp.path().join("proj");
    std::fs::create_dir_all(&project_root).unwrap();
    let ctx = PromptContext::new()
        .with_wgenty_md(vec!["X".into()])
        .with_project_root(project_root.clone());
    let out = with_fake_home(tmp.path(), || {
        build_user_turn_reminder(&ctx, &[]).expect("Some")
    });
    let abs = project_root.join("WGENTY.md");
    assert!(abs.is_absolute(), "project_root must be absolute for this test");
    assert!(out.to_model.contains(&format!("Contents of {}", abs.display())));
}
```

- [x] **Step 2: 运行测试，期望 PASS。**

- [x] **Step 3: 不 commit，并入 2.8。**

---

### Task 2.7: 单测——rules 字母序

**对应 tasks.md**: 2.7
**设计依据**: §5.1 U4

- [x] **Step 1: 追加：**

```rust
#[test]
fn reminder_user_rules_alphabetical_order() {
    let tmp = TempDir::new().unwrap();
    let rules = tmp.path().join(".wgenty-code").join("rules");
    std::fs::create_dir_all(&rules).unwrap();
    std::fs::write(rules.join("zeta.md"), "ZETA").unwrap();
    std::fs::write(rules.join("alpha.md"), "ALPHA").unwrap();

    let ctx = PromptContext::new();
    let out = with_fake_home(tmp.path(), || {
        build_user_turn_reminder(&ctx, &[]).expect("Some")
    });
    let a = out.to_model.find("ALPHA").unwrap();
    let z = out.to_model.find("ZETA").unwrap();
    assert!(a < z, "alpha must precede zeta in reminder");
}
```

- [x] **Step 2: 运行测试，期望 PASS。不 commit。**

---

### Task 2.8: 单测——hook 优先级排序 + visibility 分流 (U6/U7/U8)

**对应 tasks.md**: 2.8
**设计依据**: §5.1 U6/U7/U8

- [x] **Step 1: 追加：**

```rust
#[test]
fn reminder_hook_priority_sorting() {
    let tmp = TempDir::new().unwrap();
    let ctx = PromptContext::new();
    let frags = vec![
        InjectedFragment {
            content: "Z".into(),
            priority: 30,
            visibility: LayerVisibility::Visible,
            source_label: "hook:UserPromptSubmit:0".into(),
        },
        InjectedFragment {
            content: "A".into(),
            priority: 10,
            visibility: LayerVisibility::Visible,
            source_label: "hook:UserPromptSubmit:1".into(),
        },
    ];
    let out = with_fake_home(tmp.path(), || {
        build_user_turn_reminder(&ctx, &frags).expect("Some")
    });
    let a = out.to_model.find("\nA\n").unwrap();
    let z = out.to_model.find("\nZ\n").unwrap();
    assert!(a < z, "priority 10 must precede priority 30");
}

#[test]
fn reminder_internal_visibility_excludes_transcript() {
    let tmp = TempDir::new().unwrap();
    let ctx = PromptContext::new();
    let frag = InjectedFragment {
        content: "SECRET".into(),
        priority: 10,
        visibility: LayerVisibility::Internal,
        source_label: "hook:UserPromptSubmit:0".into(),
    };
    let out = with_fake_home(tmp.path(), || {
        build_user_turn_reminder(&ctx, &[frag]).expect("Some")
    });
    assert!(out.to_model.contains("SECRET"));
    // 全部段都是 Internal 且无文件源 → transcript 应为 None
    assert!(out.to_transcript.is_none());
}

#[test]
fn reminder_visible_hook_in_both_outputs() {
    let tmp = TempDir::new().unwrap();
    let project_root = tmp.path().join("proj");
    std::fs::create_dir_all(&project_root).unwrap();
    let ctx = PromptContext::new()
        .with_wgenty_md(vec!["proj".into()])
        .with_project_root(project_root);
    let frag = InjectedFragment {
        content: "PUBLIC".into(),
        priority: 10,
        visibility: LayerVisibility::Visible,
        source_label: "hook:UserPromptSubmit:0".into(),
    };
    let out = with_fake_home(tmp.path(), || {
        build_user_turn_reminder(&ctx, &[frag]).expect("Some")
    });
    assert!(out.to_model.contains("PUBLIC"));
    let transcript = out.to_transcript.expect("transcript Some");
    assert!(transcript.contains("PUBLIC"));
    assert!(transcript.contains("proj"));
}
```

- [x] **Step 2: 运行**

```bash
cargo test -p wgenty-code reminder_
```

期望: §2 全部单测 PASS（约 9 个）。

- [x] **Step 3: Commit（§2 所有单测合并）**

```bash
git add src/prompts/mod.rs
git commit -m "test(prompts): cover build_user_turn_reminder rendering and visibility"
```

---

## §3 请求构造层接入

> **风险节点**：本节修改 TUI agent loop 与并发路径。每完成一个 step 后先 `cargo check`，再 `cargo test --lib`；测试通过前不要继续推进。

### Task 3.1: 在 `process_input_inner` 内接入 reminder

**对应 tasks.md**: 3.1
**设计依据**: D1（拼到 user message content 头部）+ §3.2 调用链
**前置依赖**: Task 1.x、2.x 全部完成

**Files:**
- Modify: `src/tui/agent/mod.rs`（约 121-134 行 `process_input_inner`）

**Interfaces consumed:**
- `build_user_turn_reminder(ctx, &[InjectedFragment]) -> Option<ReminderOutput>`
- `collect_injections(&[HookOutcome]) -> Vec<InjectedFragment>`

> 注：此 task 暂以"空 hook outcomes"接入，hook fire 改造在 Task 3.2 完成；这种渐进式让本 task 独立通过 cargo check + 单测。

- [x] **Step 1: 在 `AgentLoop`（`src/tui/agent/mod.rs`）的字段中确认是否已携带 `PromptContext`。若没有，从已传入字段（如 `prompt_context: PromptContext`）读取；若 AgentLoop 不持有，先看其构造点 `src/tui/app/turn.rs` 是否传 ctx——必要时把 `prompt_context` 加进 `AgentLoop` 字段。**

阅读 `src/tui/app/turn.rs::spawn_agent_turn`（grep `AgentLoop::new` 找到构造）确认传参。最小侵入做法：把已聚合好的 `Arc<PromptContext>` 在构造时塞入。

- [x] **Step 2: 修改 `process_input_inner`：**

```rust
async fn process_input_inner(&mut self, input: String) -> Result<(), String> {
    self.inject_background_results().await;

    // 1. 收集 hook 注入（hook fire 在 Task 3.2 接通；本 task 先空 outcomes）
    let injections: Vec<crate::runtime::hooks::InjectedFragment> = Vec::new();

    // 2. 构造 reminder
    let reminder = crate::prompts::build_user_turn_reminder(
        self.prompt_context.as_ref(),
        &injections,
    );

    // 3. 拼 user content
    let user_content = match &reminder {
        Some(r) => format!("{}\n\n{}", r.to_model, input),
        None => input.clone(),
    };

    // 4. token 估算 + push history
    {
        let mut history = self.conversation_history.lock().await;
        let input_tokens = user_content.len() / 4;
        self.token_counter.add_input(input_tokens);
        history.push(ChatMessage::user(&user_content));
    }

    // 5. transcript 投递（visible reminder）
    if let Some(r) = &reminder {
        if let Some(transcript) = &r.to_transcript {
            // 沿用现有 push_system_message 渠道（如 AgentLoop 持有 ui_tx 或 ui_sender）
            self.push_system_message(transcript.clone());
        }
    }

    self.run_agent_loop().await
}
```

> 若 `AgentLoop` 没有 `push_system_message` 等价方法，复用现有把 system message 投递到 TUI transcript 的现有 API（grep `push_system_message` / `ui_tx.send` 找到）。

- [x] **Step 3: `cargo check` 修编译错（缺字段、缺 use 等）。**

- [x] **Step 4: 运行已有单测确认未破坏**

```bash
cargo test -p wgenty-code --lib
```

- [x] **Step 5: 暂不 commit；与 3.2、3.3 合并。**

---

### Task 3.2: 把 fire-and-forget 改为 `await`

**对应 tasks.md**: 3.2
**设计依据**: D7（方案 B 在 AgentLoop 内 await）+ R6 graceful degradation
**前置依赖**: 3.1

**Files:**
- Modify: `src/tui/app/input.rs`（约 161-189 行删除 `tokio::spawn(... fire ...)`）
- Modify: `src/tui/agent/mod.rs`（在 `process_input_inner` 内 fire + await）

- [x] **Step 1: 在 `src/tui/app/input.rs` 删除 161-189 行 fire-and-forget UserPromptSubmit 整块（保留前后的 `/help` 处理与 slash 路由）。其它 hook 事件（如 SlashCommand）的 fire-and-forget 不动。**

- [x] **Step 2: 在 `AgentLoop` 字段中确认/添加 `hook_manager: Arc<HookManager>` 与 `session_id: String`（如已存在，跳过）。**

- [x] **Step 3: 修改 `process_input_inner` 第 1 步为：**

```rust
// 1a. Fire UserPromptSubmit hook (await, 10s timeout)
let outcomes = {
    let hook_ctx = crate::runtime::hooks::HookContext {
        event: "UserPromptSubmit".to_string(),
        tool_name: None,
        tool_input: Some(serde_json::Value::String(input.clone())),
        tool_result: None,
        session_id: Some(self.session_id.clone()),
        working_directory: std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        comet_phase: None,
        workflow_state: None,
        variables: Default::default(),
    };
    match tokio::time::timeout(
        std::time::Duration::from_secs(10),
        self.hook_manager.fire(
            &crate::runtime::hooks::HookEvent::UserPromptSubmit,
            &hook_ctx,
            None,
            None,
        ),
    )
    .await
    {
        Ok(v) => v,
        Err(_) => {
            tracing::warn!("UserPromptSubmit hook timed out after 10s; proceeding with empty outcomes");
            Vec::new()
        }
    }
};

// 1b. 收集 hook injections
let injections = crate::runtime::hooks::collect_injections(&outcomes);
```

替换 Task 3.1 中 `let injections: Vec<_> = Vec::new();` 一行。

- [x] **Step 4: `cargo check`，修编译错。**

- [x] **Step 5: 运行**

```bash
cargo test -p wgenty-code --lib
```

期望全 PASS。

- [x] **Step 6: 不 commit，进入 3.3。**

---

### Task 3.3: `collect_injections` 串入构造路径

**对应 tasks.md**: 3.3
**设计依据**: §3.2

> Task 3.2 已完成此项接入（步骤 3 的 `collect_injections(&outcomes)`）。本 task 仅做形式收敛与 commit。

- [x] **Step 1: grep 确认调用链：**

```bash
grep -n "collect_injections" src/tui/agent/mod.rs
```

应有 1 处。

- [x] **Step 2: Commit（合并 3.1+3.2+3.3）**

```bash
git add src/tui/agent/mod.rs src/tui/app/input.rs
git commit -m "feat(tui): inject system reminder at user-turn boundary"
```

---

### Task 3.4: 集成测——首轮 user message 含 reminder

**对应 tasks.md**: 3.4
**设计依据**: 测试 §5.2 I1
**前置依赖**: 3.3

**Files:**
- Create: `tests/system_reminder.rs`

- [x] **Step 1: 阅读 `tests/` 目录现有集成测找 mocking 模式（grep `MockHttp` / `mock` 之类）；确认是否有现成的"启动一个 mini agent + 注入 fake HTTP"工具，若没有，复用最简单的"只验证 reminder 文本片段进入 history"的策略。**

- [x] **Step 2: 创建 `tests/system_reminder.rs`：**

```rust
//! Integration tests for the <system-reminder> injection channel.
//!
//! Strategy: rather than spinning up the full agent loop, we invoke
//! `build_user_turn_reminder` against a configured PromptContext with
//! the same inputs the agent would pass, then assert the structure.
//! Full-loop tests run via I4/I6 with mocked HookManager.

use std::path::PathBuf;
use tempfile::TempDir;

use wgenty_code::prompts::{build_user_turn_reminder, PromptContext};

fn with_fake_home<F: FnOnce() -> R, R>(home: &std::path::Path, f: F) -> R {
    let prev = std::env::var_os("HOME");
    std::env::set_var("HOME", home);
    let r = f();
    match prev {
        Some(v) => std::env::set_var("HOME", v),
        None => std::env::remove_var("HOME"),
    }
    r
}

#[test]
fn first_turn_user_message_contains_reminder() {
    let tmp = TempDir::new().unwrap();
    let project_root = tmp.path().join("proj");
    std::fs::create_dir_all(&project_root).unwrap();

    let ctx = PromptContext::new()
        .with_wgenty_md(vec!["PROJECT-WGENTY".into()])
        .with_project_root(project_root);

    let reminder = with_fake_home(tmp.path(), || {
        build_user_turn_reminder(&ctx, &[]).expect("reminder Some")
    });

    let user_input = "hello model";
    let user_content = format!("{}\n\n{}", reminder.to_model, user_input);
    assert!(user_content.starts_with("<system-reminder>"));
    assert!(user_content.contains("PROJECT-WGENTY"));
    assert!(user_content.ends_with(user_input));
}
```

> 如 `wgenty_code::prompts::PromptContext` 与 `build_user_turn_reminder` 不是 `pub`，先在 `lib.rs` 暴露（保证集成测可见）。

- [x] **Step 3: 运行**

```bash
cargo test -p wgenty-code --test system_reminder first_turn_user_message_contains_reminder
```

期望 PASS。

- [x] **Step 4: 暂不 commit，与 3.5 合并。**

---

### Task 3.5: 集成测——第二轮 reminder 再出现

**对应 tasks.md**: 3.5
**设计依据**: §5.2 I2

- [x] **Step 1: 在 `tests/system_reminder.rs` 追加：**

```rust
#[test]
fn second_turn_reminder_reappears() {
    let tmp = TempDir::new().unwrap();
    let project_root = tmp.path().join("proj");
    std::fs::create_dir_all(&project_root).unwrap();
    let ctx = PromptContext::new()
        .with_wgenty_md(vec!["P".into()])
        .with_project_root(project_root);

    let (a, b) = with_fake_home(tmp.path(), || {
        let r1 = build_user_turn_reminder(&ctx, &[]).expect("turn1");
        let r2 = build_user_turn_reminder(&ctx, &[]).expect("turn2");
        (r1, r2)
    });
    assert_eq!(a.to_model, b.to_model, "reminder must be rebuilt identically per turn");
    assert!(a.to_model.contains("<system-reminder>"));
}
```

- [x] **Step 2: 运行测试，期望 PASS。**

- [x] **Step 3: Commit**

```bash
git add tests/system_reminder.rs src/lib.rs
git commit -m "test: integration coverage for per-turn reminder injection"
```

---

## §4 移除旧 Layer + 适配 builder

### Task 4.1: 删除 Layer 7（AGENTS.md）与 Layer 8（WGENTY.md）

**对应 tasks.md**: 4.1
**设计依据**: §1.3 硬切 + §3.2
**前置依赖**: §3 已上线

**Files:**
- Modify: `src/prompts/mod.rs`（约 197-213 行）

- [ ] **Step 1: 删除 `assemble_instructions` 中两个块：**

```rust
// ── Layer 7: AGENTS.md Convention ───────────────────────────────────
if !context.agents_md_sections.is_empty() { ... }

// ── Layer 8: WGENTY.md 项目事实 ──────────────────────────────────
if !context.wgenty_md_sections.is_empty() { ... }
```

- [ ] **Step 2: 更新文件顶部 Layer 注释（行 3-8 的层次说明），去掉 7/8 描述，添加备注："Project & user instructions are injected via the per-turn `<system-reminder>` channel (build_user_turn_reminder), not as system messages."**

- [ ] **Step 3: `cargo check`，处理 `agents_md_sections` / `wgenty_md_sections` 字段 dead_code 警告：保留字段（reminder 仍读取），加 `#[allow(...)]` 不必要——builder 已使用。**

- [ ] **Step 4: 不 commit，合并 §4 全部。**

---

### Task 4.2: 确保 builder 字段被消费

**对应 tasks.md**: 4.2
**设计依据**: §3.2 + D4

- [ ] **Step 1: grep 验证字段读取链：**

```bash
grep -n "wgenty_md_sections\|agents_md_sections" src/prompts/mod.rs
```

应在 `build_user_turn_reminder` 内出现 2 次，`assemble_instructions` 内 0 次。

- [ ] **Step 2: 无代码改动。进入 4.3。**

---

### Task 4.3: 在 app 构造 `PromptContext` 时调用 `with_project_root`

**对应 tasks.md**: 4.3
**设计依据**: D4

**Files:**
- Modify: `src/tui/app/mod.rs`（约 277-308 行）

- [ ] **Step 1: 在原 `let project_root = std::env::current_dir().unwrap_or_else(...)` 之后，把 `project_root` 透传给 `PromptContext`：**

```rust
let mut prompt_ctx = prompt_ctx
    .with_wgenty_md(wgenty_sections)
    .with_agents_md(agents_sections)
    .with_project_root(project_root.clone());
```

- [ ] **Step 2: `cargo check`。**

- [ ] **Step 3: 不 commit，进入 4.4。**

---

### Task 4.4: 单测——硬切验证

**对应 tasks.md**: 4.4
**设计依据**: §5.1 U9

**Files:**
- Modify: `src/prompts/mod.rs`（tests）

- [ ] **Step 1: 在 `tests` 模块底部新增：**

```rust
#[test]
fn assemble_instructions_no_layer_7_8() {
    let settings = Settings::default();
    let ctx = PromptContext::new()
        .with_cwd("/tmp")
        .with_shell("zsh")
        .with_wgenty_md(vec!["should-not-appear-as-system".into()])
        .with_agents_md(vec!["should-not-appear-as-system".into()]);
    let instructions = assemble_instructions(&settings, &ctx);
    let blob: String = instructions
        .system_messages
        .iter()
        .filter_map(|m| m.content.clone())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !blob.contains("# AGENTS.md"),
        "Layer 7 must be removed"
    );
    assert!(
        !blob.contains("# WGENTY.md — 项目规则与约定"),
        "Layer 8 must be removed"
    );
    assert!(
        !blob.contains("should-not-appear-as-system"),
        "agent/wgenty content must not leak into system messages"
    );
}
```

- [ ] **Step 2: 运行**

```bash
cargo test -p wgenty-code assemble_instructions_no_layer_7_8
```

期望 PASS。

- [ ] **Step 3: Commit（§4 合并）**

```bash
git add src/prompts/mod.rs src/tui/app/mod.rs
git commit -m "feat(prompts): hard-cut Layer 7/8; project root flows into reminder"
```

---

## §5 Hook injection 接通

### Task 5.1: 验证 / 补齐 `injected_content` 填充

**对应 tasks.md**: 5.1
**设计依据**: D5
**前置依赖**: 1.1（HookOutcome 已加 priority/visibility 字段）

**Files:**
- Modify: `src/runtime/hooks/mod.rs`（`run_inject_action` / 等价路径）

- [ ] **Step 1: grep `run_inject_action\|HookAction::InjectContext.*=>` 找到当前 HookOutcome 生成点。**

```bash
grep -n "InjectContext" src/runtime/hooks/mod.rs
```

预计在第 941 行附近的 match 分支。

- [ ] **Step 2: 在该分支构造 HookOutcome 时填充新增字段：**

```rust
HookAction::InjectContext {
    source,
    priority,
    visibility,
} => {
    let content = render_inject_source(source, ctx); // 现有逻辑
    HookOutcome {
        def: def.clone(),
        continue_execution: true,
        reason: None,
        injected_content: content,
        user_answer: None,
        injection_priority: Some(*priority),
        injection_visibility: Some(visibility.clone()),
    }
}
```

（具体变量名按现有代码调整。）

- [ ] **Step 3: 在 mod tests 内新增 / 调整：**

```rust
#[test]
fn inject_context_outcome_carries_priority_and_visibility() {
    let mut hm = HookManager::default();
    let def = HookDefinition {
        event: HookEvent::UserPromptSubmit,
        matcher: None,
        when_state: None,
        actions: vec![HookAction::InjectContext {
            source: ContextSource::Inline("X".into()),
            priority: 7,
            visibility: LayerVisibility::Internal,
        }],
    };
    hm.register(def);
    let ctx = HookContext { /* ... 复用现有 test helper */ ..Default::default() };
    let outcomes = futures::executor::block_on(hm.fire(
        &HookEvent::UserPromptSubmit, &ctx, None, None,
    ));
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].injection_priority, Some(7));
    assert!(matches!(outcomes[0].injection_visibility, Some(LayerVisibility::Internal)));
    assert_eq!(outcomes[0].injected_content.as_deref(), Some("X"));
}
```

> 注：若现有 InjectContext 单测已断言 `injected_content`，仅追加 priority/visibility 断言行即可，避免重复。

- [ ] **Step 4: 运行**

```bash
cargo test -p wgenty-code inject_context_outcome_carries_priority_and_visibility
```

期望 PASS。

- [ ] **Step 5: Commit**

```bash
git add src/runtime/hooks/mod.rs
git commit -m "feat(hooks): propagate priority and visibility from InjectContext to HookOutcome"
```

---

### Task 5.2: 多 hook 优先级排序在请求构造层串通

**对应 tasks.md**: 5.2
**设计依据**: D5

- [ ] **Step 1: 已由 Task 1.1 `collect_injections` + Task 3.2 `await fire` 完成串通。新增一个**调用 `collect_injections` 但用真实 outcome 的轻量单测**（在 `src/runtime/hooks/mod.rs::tests`）**：

```rust
#[test]
fn multiple_inject_hooks_sort_by_priority_after_collect() {
    let mut hm = HookManager::default();
    hm.register(HookDefinition {
        event: HookEvent::UserPromptSubmit,
        matcher: None,
        when_state: None,
        actions: vec![HookAction::InjectContext {
            source: ContextSource::Inline("LOW".into()),
            priority: 30,
            visibility: LayerVisibility::Visible,
        }],
    });
    hm.register(HookDefinition {
        event: HookEvent::UserPromptSubmit,
        matcher: None,
        when_state: None,
        actions: vec![HookAction::InjectContext {
            source: ContextSource::Inline("HIGH".into()),
            priority: 5,
            visibility: LayerVisibility::Visible,
        }],
    });
    let ctx = HookContext { ..Default::default() };
    let outcomes = futures::executor::block_on(hm.fire(
        &HookEvent::UserPromptSubmit, &ctx, None, None,
    ));
    let frags = collect_injections(&outcomes);
    let order: Vec<_> = frags.iter().map(|f| f.content.as_str()).collect();
    assert_eq!(order, vec!["HIGH", "LOW"]);
}
```

- [ ] **Step 2: 运行测试，期望 PASS。**

- [ ] **Step 3: Commit**

```bash
git add src/runtime/hooks/mod.rs
git commit -m "test(hooks): verify priority sort end-to-end across multiple inject hooks"
```

---

### Task 5.3: 集成测——hook 注入端到端

**对应 tasks.md**: 5.3
**设计依据**: §5.2 I4

**Files:**
- Modify: `tests/system_reminder.rs`

- [ ] **Step 1: 追加：**

```rust
use wgenty_code::runtime::hooks::{
    collect_injections, ContextSource, HookAction, HookDefinition, HookEvent, HookManager,
    LayerVisibility,
};

#[test]
fn hook_inject_content_end_to_end() {
    let mut hm = HookManager::default();
    hm.register(HookDefinition {
        event: HookEvent::UserPromptSubmit,
        matcher: None,
        when_state: None,
        actions: vec![HookAction::InjectContext {
            source: ContextSource::Inline("EXTRA".into()),
            priority: 50,
            visibility: LayerVisibility::Visible,
        }],
    });

    let ctx = wgenty_code::runtime::hooks::HookContext {
        event: "UserPromptSubmit".into(),
        ..Default::default()
    };
    let outcomes = futures::executor::block_on(hm.fire(
        &HookEvent::UserPromptSubmit, &ctx, None, None,
    ));
    let frags = collect_injections(&outcomes);

    let prompt_ctx = PromptContext::new();
    let reminder =
        build_user_turn_reminder(&prompt_ctx, &frags).expect("reminder must be Some with hook");
    let user_content = format!("{}\n\n{}", reminder.to_model, "user-message");
    assert!(user_content.contains("EXTRA"));
}
```

> 如 `HookContext::default()` 不存在，构造完整字面量（参考 Task 3.2 步骤 3）。

- [ ] **Step 2: 运行**

```bash
cargo test -p wgenty-code --test system_reminder hook_inject_content_end_to_end
```

期望 PASS。

- [ ] **Step 3: 不 commit，与 5.4 合并。**

---

### Task 5.4: 集成测——优先级排序端到端

**对应 tasks.md**: 5.4
**设计依据**: §5.2

- [ ] **Step 1: 追加：**

```rust
#[test]
fn two_hooks_render_in_priority_order_in_reminder() {
    let mut hm = HookManager::default();
    hm.register(HookDefinition {
        event: HookEvent::UserPromptSubmit,
        matcher: None,
        when_state: None,
        actions: vec![HookAction::InjectContext {
            source: ContextSource::Inline("FROM-LOW-PRIO".into()),
            priority: 90,
            visibility: LayerVisibility::Visible,
        }],
    });
    hm.register(HookDefinition {
        event: HookEvent::UserPromptSubmit,
        matcher: None,
        when_state: None,
        actions: vec![HookAction::InjectContext {
            source: ContextSource::Inline("FROM-HIGH-PRIO".into()),
            priority: 5,
            visibility: LayerVisibility::Visible,
        }],
    });

    let ctx = wgenty_code::runtime::hooks::HookContext {
        event: "UserPromptSubmit".into(),
        ..Default::default()
    };
    let outcomes = futures::executor::block_on(hm.fire(
        &HookEvent::UserPromptSubmit, &ctx, None, None,
    ));
    let frags = collect_injections(&outcomes);
    let reminder = build_user_turn_reminder(&PromptContext::new(), &frags).unwrap();
    let high = reminder.to_model.find("FROM-HIGH-PRIO").unwrap();
    let low = reminder.to_model.find("FROM-LOW-PRIO").unwrap();
    assert!(high < low, "priority 5 must render before priority 90");
}
```

- [ ] **Step 2: 运行测试，期望 PASS。**

- [ ] **Step 3: Commit**

```bash
git add tests/system_reminder.rs
git commit -m "test: integration coverage for hook injection end-to-end"
```

---

## §6 Token 预算警告

### Task 6.1: 改造警告为"完整 reminder 块"

**对应 tasks.md**: 6.1
**设计依据**: D9
**前置依赖**: §2 + §4

**Files:**
- Modify: `src/tui/app/mod.rs`（约 283-303 行 warn 逻辑）
- Possibly modify: `src/tui/agent/mod.rs`（如果决定首次构造时计算）

> 选址说明：D9 要求"首次构造 reminder 时一次"。最简实现：在 app 启动构造 PromptContext 完毕后，一次性调用 `build_user_turn_reminder(&ctx, &[])` 估算 token，不需要等到第一次 turn。这保留了与现有"启动期一次性警告"等价的 UX，同时把估算输入从"两文件 sections"换成"完整 reminder 文本含 preamble + 用户级源"。

- [ ] **Step 1: 替换 283-303 行的 wgenty/agents 估算块为：**

```rust
// 估算完整 reminder（含 preamble + 4 个文件源；不含 hook 注入）
let reminder_token_estimate = {
    // 临时构造 ctx 用于估算（与下文真正用于 agent 的 ctx 同字段）
    let preview_ctx = crate::prompts::PromptContext::new()
        .with_wgenty_md(wgenty_sections.clone())
        .with_agents_md(agents_sections.clone())
        .with_project_root(project_root.clone());
    match crate::prompts::build_user_turn_reminder(&preview_ctx, &[]) {
        Some(out) => crate::utils::estimate_tokens(&out.to_model),
        None => 0,
    }
};
if reminder_token_estimate > 2000 {
    tracing::warn!(
        reminder_tokens = reminder_token_estimate,
        "<system-reminder> block estimate ~{} tokens. \
         Consider trimming WGENTY.md / AGENTS.md / ~/.wgenty-code/ files to keep per-turn input lean.",
        reminder_token_estimate,
    );
}
```

> Note: 阈值常量 `2000` 直接复用，按 D9 不引入新常量。

- [ ] **Step 2: `cargo check`。**

- [ ] **Step 3: 不 commit，合并 6.2-6.5。**

---

### Task 6.2: 警告触发位置 / once 语义

**对应 tasks.md**: 6.2
**设计依据**: D9

- [ ] **Step 1: 当前实现已"启动期一次"。无须 `Once` 守卫——app 构造仅一次。在改造代码上方注释明确："Fires once per session at app startup (before first turn)."**

- [ ] **Step 2: 不 commit。**

---

### Task 6.3: hook 注入不计入预算

**对应 tasks.md**: 6.3
**设计依据**: D9 + T4

- [ ] **Step 1: 已由 Task 6.1 `build_user_turn_reminder(&preview_ctx, &[])`（第二参数显式空 slice）满足。注释中加一行说明："Hook injections are dynamic per turn; not included in this estimate."**

- [ ] **Step 2: 不 commit。**

---

### Task 6.4: 单测——超阈值触发警告

**对应 tasks.md**: 6.4
**设计依据**: §5.3

> 触发 `tracing::warn!` 在单测中验证较复杂；改为验证"预算估算函数"返回值。提取一个纯函数 `estimate_reminder_tokens(ctx) -> usize` 便于单测。

- [ ] **Step 1: 在 `src/prompts/mod.rs` 公开一个辅助：**

```rust
pub fn estimate_reminder_tokens(ctx: &PromptContext) -> usize {
    match build_user_turn_reminder(ctx, &[]) {
        Some(out) => out.to_model.len() / 4,
        None => 0,
    }
}
```

> 复用 `len() / 4` 与 `utils::estimate_tokens` 等价语义（grep 验证 `estimate_tokens` 实现一致；若不同则改为 `crate::utils::estimate_tokens(&out.to_model)`）。

- [ ] **Step 2: 把 Task 6.1 步骤 1 的内联估算改为 `crate::prompts::estimate_reminder_tokens(&preview_ctx)`。**

- [ ] **Step 3: 在 `src/prompts/mod.rs::tests` 添加：**

```rust
#[test]
fn estimate_reminder_tokens_threshold() {
    let tmp = tempfile::TempDir::new().unwrap();
    let project_root = tmp.path().join("proj");
    std::fs::create_dir_all(&project_root).unwrap();
    let huge = "X".repeat(20_000);
    let ctx = PromptContext::new()
        .with_wgenty_md(vec![huge])
        .with_project_root(project_root);

    let tokens = with_fake_home(tmp.path(), || estimate_reminder_tokens(&ctx));
    assert!(tokens > 2000, "huge section should exceed 2000 tokens, got {tokens}");
}
```

> `with_fake_home` 已在 reminder_tests 内定义；提到 tests 模块顶层使用，或复制本测试到 `reminder_tests` 模块。

- [ ] **Step 4: 运行测试，期望 PASS。**

- [ ] **Step 5: 不 commit，合并 6.5。**

---

### Task 6.5: 单测——未超阈值不告警

**对应 tasks.md**: 6.5
**设计依据**: §5.3

- [ ] **Step 1: 追加：**

```rust
#[test]
fn estimate_reminder_tokens_under_threshold() {
    let tmp = tempfile::TempDir::new().unwrap();
    let project_root = tmp.path().join("proj");
    std::fs::create_dir_all(&project_root).unwrap();
    let ctx = PromptContext::new()
        .with_wgenty_md(vec!["tiny content".into()])
        .with_project_root(project_root);
    let tokens = with_fake_home(tmp.path(), || estimate_reminder_tokens(&ctx));
    assert!(tokens < 2000, "tiny section under threshold, got {tokens}");
}
```

- [ ] **Step 2: 运行测试，期望 PASS。**

- [ ] **Step 3: Commit（§6 合并）**

```bash
git add src/prompts/mod.rs src/tui/app/mod.rs
git commit -m "feat(prompts): token budget warning covers full reminder block"
```

---

## §7 Documentation & polish

### Task 7.1: 在项目根 `WGENTY.md` 增加 "Context injection channels" 段

**对应 tasks.md**: 7.1
**前置依赖**: 无

**Files:**
- Modify: `WGENTY.md`（项目根）

- [ ] **Step 1: 在文件末尾追加段落（示例文本，按需润色，但保持中文）：**

```markdown
## Context injection channels

wgenty-code 提供两层用户级上下文通道，自动随每轮 user message 注入：

- `~/.wgenty-code/WGENTY.md` — 用户级全局指令（对所有项目生效）。
- `~/.wgenty-code/rules/*.md` — 用户级规则文件（顶层 `.md`，按文件名字母序拼入）。

加上项目根的 `WGENTY.md` / `AGENTS.md`，共 4 个静态源；UserPromptSubmit hook 的 `InjectContext` 动态注入也走同一通道。每轮内容会以 `<system-reminder>` 块拼到 user message 头部。
```

- [ ] **Step 2: 不 commit，与 §7 其它一起。**

---

### Task 7.2: CHANGELOG BREAKING 说明

**对应 tasks.md**: 7.2

**Files:**
- Modify: `CHANGELOG.md`（若不存在则在最简 stub 中创建 `## Unreleased` 节）

- [ ] **Step 1: 在 `## Unreleased` 下追加：**

```markdown
### BREAKING

- 项目说明（`AGENTS.md` / `WGENTY.md`）不再以 system message 形式注入 prompt 链。
  新增 `<system-reminder>` 通道，每轮拼到 user message 头部；同时聚合
  `~/.wgenty-code/WGENTY.md` 与 `~/.wgenty-code/rules/*.md`，以及
  `UserPromptSubmit` hook 的 `InjectContext` 动态注入。

  影响范围: 依赖旧 system message 文本（如 `# AGENTS.md`、
  `# WGENTY.md — 项目规则与约定`）的下游工具需要更新。
```

- [ ] **Step 2: 不 commit，进入 7.3。**

---

### Task 7.3: 示例 rule 文件

**对应 tasks.md**: 7.3

> 该文件位于用户 home，不在仓库；本 task 是 dogfood 步骤。

- [ ] **Step 1: 运行**

```bash
mkdir -p "$HOME/.wgenty-code/rules"
cp "$HOME/.claude/rules/comet-phase-guard.md" "$HOME/.wgenty-code/rules/comet-phase-guard.md"
ls -la "$HOME/.wgenty-code/rules/"
```

期望: 看到 `comet-phase-guard.md`。

- [ ] **Step 2: 在 plan 之外记录此动作（无 commit）；如 source 文件不存在，跳过并在 §8 验证阶段提示用户手动放一个 stub。**

---

### Task 7.4: cargo test + clippy

**对应 tasks.md**: 7.4

- [ ] **Step 1: 运行**

```bash
cargo test --workspace
```

期望: 所有测试 PASS。

- [ ] **Step 2: 运行**

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

期望: 0 warning。若 reminder 字段被 dead_code 警告，按需消除（通常加 `#[allow(dead_code)]` 或确实用上）。

- [ ] **Step 3: 不 commit，进入 7.5。**

---

### Task 7.5: cargo fmt 检查

**对应 tasks.md**: 7.5

- [ ] **Step 1: 运行**

```bash
cargo fmt -- --check
```

如失败：

```bash
cargo fmt
```

- [ ] **Step 2: Commit（§7 合并）**

```bash
git add WGENTY.md CHANGELOG.md src/ tests/ Cargo.toml Cargo.lock
git commit -m "docs(reminder): document user-level injection channels and BREAKING migration"
```

---

## §8 验证

### Task 8.1: 12 验收场景全覆盖

**对应 tasks.md**: 8.1
**设计依据**: §5.3 表

- [ ] **Step 1: 对照设计文档 §5.3 表逐项确认：场景 1 → I1、场景 2 → U1、...、场景 12 → 总数 19 测试。**

- [ ] **Step 2: 若发现缺口，回填测试。**

- [ ] **Step 3: 无代码改动，无 commit。**

---

### Task 8.2: repl 手工验证

**对应 tasks.md**: 8.2

- [ ] **Step 1: 启动**

```bash
cargo run --release -- repl
```

- [ ] **Step 2: 输入任意 prompt，启用 debug toggle（grep `debug` 找命令）或查看 logs 中 history 内容；确认 user message content 头部含 `<system-reminder>`。**

- [ ] **Step 3: 记录截图 / log 片段（非必须）。无 commit。**

---

### Task 8.3: 缺失文件优雅降级

**对应 tasks.md**: 8.3

- [ ] **Step 1: 在另一 shell**

```bash
mv "$HOME/.wgenty-code/WGENTY.md" "$HOME/.wgenty-code/WGENTY.md.bak" 2>/dev/null || true
```

- [ ] **Step 2: 在 repl 中再次提交 prompt；确认无报错、reminder 中无空 `Contents of` 标题。**

- [ ] **Step 3: 恢复：**

```bash
mv "$HOME/.wgenty-code/WGENTY.md.bak" "$HOME/.wgenty-code/WGENTY.md" 2>/dev/null || true
```

---

### Task 8.4: hook 端到端验证

**对应 tasks.md**: 8.4

- [ ] **Step 1: 在 `settings.json` 临时加入：**

```json
{
  "hooks": {
    "UserPromptSubmit": [
      {
        "actions": [
          {
            "InjectContext": {
              "source": { "Inline": "VERIFY-HOOK-INJECTED" },
              "priority": 10,
              "visibility": "Visible"
            }
          }
        ]
      }
    ]
  }
}
```

- [ ] **Step 2: 重启 repl，提交 prompt，确认 history 中 user message 含 `VERIFY-HOOK-INJECTED`。**

- [ ] **Step 3: 移除临时配置。**

---

### Task 8.5: 单次查询模式验证

**对应 tasks.md**: 8.5

- [ ] **Step 1: 运行**

```bash
cargo run --release -- repl --prompt "X" 2>&1 | head -50
```

- [ ] **Step 2: 启用 trace 或临时加 `dbg!` 确认 reminder 注入。验证后清理。**

---

## §9 Open Questions 收尾

> O1/O2/O3 已在设计阶段闭合。本节为形式化标记。

### Task 9.1: O1 标题决策

- [ ] **Step 1: 在 `openspec/changes/system-reminder-channel/proposal.md`（或 design doc）末尾确认已记录 D1=B（`# wgentyMd`）。无代码改动。**

### Task 9.2: O2 死锁验证

- [ ] **Step 1: 设计文档 §2.7 D7 已给出死锁分析。无代码改动。**

### Task 9.3: O3 LayerVisibility 实现

- [ ] **Step 1: 设计文档 §2.8 D8 + builder 双轨输出已实现。无代码改动。**

- [ ] **Step 2: §9 全部 task 勾选完成；如 OpenSpec 需要状态同步，运行 comet guard：**

```bash
"$COMET_BASH" "$COMET_STATE" set system-reminder-channel verify --field current_phase
```

（按 comet 流程；不修改源代码。）

---

## 自查（Self-Review）

- **Spec 覆盖**: tasks.md 共 9 节 37 个子任务全部映射；每节末尾 commit；测试覆盖 19 个（U1-U12 + I1-I7）。
- **类型一致性**: `InjectedFragment` / `ReminderOutput` / `PromptContext::project_root` 在所有任务中签名一致；`HookOutcome` 新字段 `injection_priority` / `injection_visibility` 在 Task 1.1 引入、Task 5.1 填充、Task 1.1 `collect_injections` 消费。
- **关键风险节点回顾**: §3 修改 tui/agent 后必须先 `cargo check` 再写测试（已在 Task 3.1 步骤 3-4 强调）；§5 hook outcome 字段补齐若遗漏全文检索 `HookOutcome { ` 字面构造点（已在 Task 1.1 步骤 3 提示）。
- **占位符扫描**: 全文 0 个 TODO/TBD/"实现 later"；所有代码块给出完整 Rust 片段。

## 执行 Handoff

Plan 完成并保存到 `docs/superpowers/plans/2026-06-27-system-reminder-channel.md`。两种执行选项：

1. **Subagent-Driven（推荐）** — 每个 task 派 fresh subagent、双审查、快速迭代。
2. **Inline Execution** — 在当前会话内按 batch + checkpoint 执行。

请选择执行方式。
