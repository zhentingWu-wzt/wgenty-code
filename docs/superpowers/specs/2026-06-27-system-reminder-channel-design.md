---
comet_change: system-reminder-channel
role: technical-design
canonical_spec: openspec
---

# System Reminder Injection Channel — Technical Design

## 1. 背景与目标

### 1.1 背景

wgenty-code 当前用 `src/prompts/mod.rs::assemble_instructions` 以 8 层 `ChatMessage::system` 拼装系统提示。其中：

- **Layer 7** 把项目 `AGENTS.md` 以独立 system message 形式直接推入提示链；
- **Layer 8** 把项目 `WGENTY.md` 以同样的方式推入；
- 没有用户级全局指令（`~/.wgenty-code/WGENTY.md`）或用户级规则（`~/.wgenty-code/rules/*.md`）的入口；
- `HookAction::InjectContext` 在 `src/runtime/hooks/mod.rs` 中有完整的结构体与单测，但 `HookOutcome::injected_content` 在调用方从未被消费。

观察到的症状：长对话中，模型对 `AGENTS.md` / `WGENTY.md` 的规则记忆力衰减明显——它们作为前置 system message 在每轮被"权重摊薄"，模型实际响应越来越偏向最近的 user/assistant 内容。Claude Code 用 `<system-reminder>` 注入通道解决相同问题：把附属于"本轮上下文"的内容拼到每轮 user message 头部，使其在每一轮都"新鲜"。

### 1.2 参照对象

Claude Code 在每轮 user message 之前注入一段 `<system-reminder>` 块，块内包含：

1. 双层 preamble（开头强调 OVERRIDE，结尾说明 may-or-may-not-be-relevant）；
2. 多个内容源（用户全局指令、用户全局规则、项目说明）；
3. 每段内容前的 `Contents of <绝对路径> (描述):` 来源标注。

本设计 1:1 对齐 Claude Code 该机制（含措辞、缩进、来源标注），并接通半成品 hook 注入通道。

### 1.3 Goals

- 在 wgenty-code 中提供与 Claude Code 等价的 `<system-reminder>` 注入通道（结构、措辞、来源标注 1:1）。
- 聚合 4 个静态文件源（用户级 WGENTY.md / 用户级 rules / 项目 WGENTY.md / 项目 AGENTS.md）+ 任意数量动态 hook 源，统一进入 reminder 通道；缺失文件优雅降级。
- 把 reminder 拼到每轮 user message 头部，而非 system prompt；**硬切**移除 Layer 7 / Layer 8。
- 接通 `HookAction::InjectContext`：`UserPromptSubmit` hook 的 `injected_content` 在下一轮 user message 中可被模型看到，并按 `LayerVisibility` 分流到 TUI transcript。
- 提供 token 预算计算和一次性警告，避免 reminder 膨胀失控。

### 1.4 Non-Goals

- 不改 subagent prompt 构造路径（明确排除）。
- 不为 `rules/*.md` 引入 frontmatter 过滤、条件加载、按 phase 切换（无脑全文注入）。
- 不为现有 system layer 行为做向后兼容（硬切，v0.1.0 阶段可接受）。
- 不改 skills / permissions / MCP / collaboration 链路。
- 不改 `~/.wgenty-code/skills/` 同步机制（另案）。
- 不引入新的可观测性子系统（沿用现有 `tracing` / `log`）。

---

## 2. 关键设计决策

> 本节决策已在 brainstorming 阶段逐项确认并固化在 `brainstorm-summary.md`，此处给出最终版及理由。

### D1 — Reminder 文本拼进 user message 内容头部

**决策**：作为 `ChatMessage::user(reminder + &input)` 拼接，与用户原文同一条 message。

**替代方案**：
- (A) 作为新的 `ChatMessage::system`，紧贴现有 system 链之后；
- (C) Anthropic API `system` 字段后追加。

**选 B 的理由**：
1. 与 Claude Code 实际行为一致（用户已确认 Q1=B）。
2. 语义上 reminder 附属于"本轮"，留在 user 消息内更贴"上下文"而非"规则"。
3. 系统提示保持稳定，命中 prompt caching 概率更高。
4. 不必新增 message 类型。

**代价**：注入点位于 `src/tui/agent/mod.rs::process_input_inner` 的 `history.push` 处（非 `assemble_instructions` 内）。

### D2 — Reminder 构造的入口：双轨输出

**决策**：在 `src/prompts/mod.rs` 新增

```rust
pub struct ReminderOutput {
    pub to_model: String,             // 发给模型的完整 reminder（含 Internal 内容）
    pub to_transcript: Option<String>, // 在 TUI transcript 展示的副本（剔除 Internal，None 表示无可见内容）
}

pub fn build_user_turn_reminder(
    ctx: &PromptContext,
    hook_injections: &[InjectedFragment],
) -> Option<ReminderOutput>
```

**双轨设计动机**（解决 O3）：`LayerVisibility::Internal` 的 hook 注入必须"模型能看、TUI 不显示"。把分流交给 builder 输出端，让调用方拿到两份字符串，调用方按用途选择——避免 TUI 层临场过滤标签字符串。

**返回 `Option<...>`**：四源全缺且无 hook 注入时返回 `None`，调用方据此跳过前置拼接。

### D3 — 文件源 reader：两新两保留

`src/utils/project.rs` 新增：

| 函数 | 用途 | 返回 |
|------|------|------|
| `read_user_global_instructions()` | 读 `~/.wgenty-code/WGENTY.md` | `Option<(PathBuf, String)>` |
| `read_user_global_rules()` | 扫 `~/.wgenty-code/rules/*.md` 顶层非空 .md | `Vec<(PathBuf, String)>`（字母序） |

`PathBuf` 返回绝对路径，供来源标注使用。`dirs::home_dir() = None`（CI / headless）静默降级为空。

保留：`read_wgenty_md_sections` / `read_agents_md_sections` 继续按 `---` 拆 section（兼容现有调用方），但在拼 reminder 时通过 `sections.join("\n\n")` 重新合并为整文件等价字符串。

### D4 — 项目根路径传递

**决策**：`PromptContext` 新增 `project_root: Option<PathBuf>` 字段及 `with_project_root(path)` builder 方法。`src/tui/app/mod.rs` 构造 `PromptContext` 时填入 `std::env::current_dir().ok()`。

reminder builder 用 `project_root.join("WGENTY.md").display()` 渲染来源标注；若 sections 非空但 `project_root` 缺失，使用相对路径 `WGENTY.md` 兜底（不报错）。

公开 API `with_wgenty_md` / `with_agents_md` 签名不变，新增 `with_project_root` 配对方法，保持原有调用方编译通过。

### D5 — Hook injection 桥接：新增 `InjectedFragment` + 单一收集函数

`src/runtime/hooks/mod.rs` 新增：

```rust
pub struct InjectedFragment {
    pub content: String,
    pub priority: u8,
    pub visibility: LayerVisibility,
    pub source_label: String, // 格式: "hook:<HookEvent>:<index>"
}

pub fn collect_injections(outcomes: &[HookOutcome]) -> Vec<InjectedFragment>;
```

`collect_injections` 从 outcomes 中筛出非空 `injected_content`，配合 hook 定义的 priority/visibility，按 `(priority asc, declaration order asc)` 排序输出。

**Hook 范围限制**：reminder 通道仅消费 `UserPromptSubmit`（其它 hook 事件时机不匹配"下一轮 user 消息前")。`PreToolUse` 不参与 reminder。保留扩展位但首版不实现 `Stop` / `SessionStart`。

### D6 — Reminder 文本骨架

```text
<system-reminder>
As you answer the user's questions, you can use the following context:
# wgentyMd
Codebase and user instructions are shown below. Be sure to adhere to
these instructions. IMPORTANT: These instructions OVERRIDE any default
behavior and you MUST follow them exactly as written.

Contents of <abs>/~/.wgenty-code/WGENTY.md (user's private global instructions for all projects):

<内容>

Contents of <abs>/~/.wgenty-code/rules/<a>.md (user's private global instructions for all projects):

<内容>

Contents of <abs>/~/.wgenty-code/rules/<b>.md (user's private global instructions for all projects):

<内容>

Contents of <project>/WGENTY.md (project instructions, checked into the codebase):

<内容>

Contents of <project>/AGENTS.md (project agent conventions, checked into the codebase):

<内容>

Contents of hook:UserPromptSubmit:0 (dynamic hook injection):

<动态内容>

      IMPORTANT: this context may or may not be relevant to your tasks.
      You should not respond to this context unless it is highly relevant
      to your task.
</system-reminder>
```

**格式规则**：
- 标题 `# wgentyMd`（解决 O1，方案 B：本地化标题，未来变更代价极低，复用 Claude 模型先验的边际收益不抵项目身份歧义）。
- 段间空行：每段 `Contents of ...` 与内容之间 1 空行；段之间 1 空行。
- 闭合 preamble 行首 6 空格缩进（精确复刻 Claude Code）。
- 来源标注绝对路径，描述固定如上对照表（在代码中以常量定义）。
- 全 4 源缺 + 无 hook 注入 → `build_user_turn_reminder()` 返回 `None`。
- 4 源齐全 + 0 hook 注入 → 输出 reminder 块，无 hook 段。
- 0 源 + 1+ hook 注入 → 输出 reminder 块，仅含 hook 段 + 双 preamble。

### D7 — Hook fire 时机迁移（解决 O2）

**问题**：`src/tui/app/input.rs:181` 当前用 `tokio::spawn` fire-and-forget 触发 `UserPromptSubmit` hook，outcomes 永远不被消费。改造方案：

**方案 B（选定）**：Hook fire 从 `tui/app/input.rs` 移到 `AgentLoop::process_input_inner` 内 `await`，outcomes 通过 `PendingInput` 携带或直接在 AgentLoop 内消费。

**为何不是方案 A（在 input.rs 内 await 后传 outcomes）**：用户提交输入到 UI 主线程任何一处 `await` 都会阻塞渲染。AgentLoop 已在独立 task 内，可以 `await` 而不影响 UI。

**死锁分析**（O2 收尾验证）：
- 阅读 `src/tui/app/turn.rs::start_next_turn` 和 `spawn_agent_turn`：AgentLoop 通过 `tokio::spawn` 在独立任务中运行，与 UI render loop 解耦。
- `HookManager::fire(&HookEvent::UserPromptSubmit, ...)` 内部用 `tokio::time::timeout(Duration::from_secs(10), ...)`（沿用现有实现），不会无限阻塞。
- Hook 命令通过 `tokio::process::Command::spawn` 启动子进程，与主 runtime 同步等待但不持锁，无锁竞争路径。
- **结论**：方案 B 时序漂移最大 = hook 执行时间（实测毫秒级，超时 10s 兜底），UI 完全不阻塞，无死锁路径。

**用户感知**：Hook 触发从"submit 瞬间"漂移到"turn 开始时"，毫秒级差异在 TUI 不可见。

### D8 — LayerVisibility 在 reminder builder 输出端分流（解决 O3）

**问题**：`Internal` 可见性的 hook 注入必须"模型读得到、TUI transcript 看不到"。

**方案 A（选定）**：在 `build_user_turn_reminder` 内部分流到 `ReminderOutput.to_model` 和 `to_transcript`。

```rust
// 伪代码
let mut to_model = String::new();
let mut to_transcript = String::new();

// 拼 4 个文件源（两者都拼）
push_file_sources(&mut to_model);
push_file_sources(&mut to_transcript);

// 拼 hook 注入（按 visibility 分流）
for frag in hook_injections.sorted_by_priority() {
    push_fragment(&mut to_model, frag);
    if frag.visibility == LayerVisibility::Visible {
        push_fragment(&mut to_transcript, frag);
    }
}

// 完成 preamble 闭合
finalize(&mut to_model);
finalize(&mut to_transcript);

ReminderOutput {
    to_model,
    to_transcript: if to_transcript_has_visible_content { Some(to_transcript) } else { None },
}
```

**为何不是方案 B（让 TUI 层过滤标签）**：要求 TUI 渲染层识别 `<!-- internal -->` 之类标签字符串，引入跨层耦合；且 hook 注入字符串可能包含合法 `<` 字符，正则过滤脆弱。

**调用方契约**：
- 发给模型的请求 → 使用 `to_model`；
- TUI transcript 展示 → 使用 `to_transcript`（`None` 时不展示）；
- TUI 不需要做任何额外过滤。

### D9 — Token 预算警告改造

`src/tui/app/mod.rs` 现有"WGENTY+AGENTS 超阈值警告"改造为"完整 reminder 块超阈值警告"：

- 估算输入：完整 reminder 文本（含 preamble + 来源标注 + 全部文件源）。
- 估算时机：首次构造 reminder 时一次（不是 session 启动时）。
- 触发：每 session 仅一次（沿用现有 `fires_once_per_session` 模式）。
- **不计入预算**：hook 注入内容（动态、每轮变；conversation_history 整体上限兜底）。
- 阈值默认值沿用现有数（无须新引入常量）。

---

## 3. 实现拆解

### 3.1 数据结构

#### `src/runtime/hooks/mod.rs`（新增）

```rust
#[derive(Debug, Clone)]
pub struct InjectedFragment {
    pub content: String,
    pub priority: u8,
    pub visibility: LayerVisibility,
    pub source_label: String,
}

pub fn collect_injections(outcomes: &[HookOutcome]) -> Vec<InjectedFragment> {
    let mut out: Vec<_> = outcomes.iter().enumerate()
        .filter_map(|(idx, oc)| {
            oc.injected_content.as_ref().and_then(|c| {
                if c.is_empty() { None } else {
                    Some(InjectedFragment {
                        content: c.clone(),
                        priority: oc.injection_priority.unwrap_or(50),
                        visibility: oc.injection_visibility.unwrap_or(LayerVisibility::Visible),
                        source_label: format!("hook:UserPromptSubmit:{}", idx),
                    })
                }
            })
        })
        .collect();
    out.sort_by_key(|f| f.priority); // stable: ties preserve order
    out
}
```

#### `src/prompts/mod.rs`（新增）

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

pub struct ReminderOutput {
    pub to_model: String,
    pub to_transcript: Option<String>,
}

pub fn build_user_turn_reminder(
    ctx: &PromptContext,
    hook_injections: &[InjectedFragment],
) -> Option<ReminderOutput> { /* 见 §3.3 */ }

fn render_attribution_header(absolute_path: &Path, description: &str) -> String {
    format!("Contents of {} ({}):", absolute_path.display(), description)
}
```

#### `PromptContext` 字段新增

```rust
pub struct PromptContext {
    // ... 现有字段
    pub project_root: Option<PathBuf>,
}

impl PromptContextBuilder {
    pub fn with_project_root(mut self, path: PathBuf) -> Self {
        self.ctx.project_root = Some(path);
        self
    }
}
```

#### `src/utils/project.rs`（新增）

```rust
pub fn read_user_global_instructions() -> Option<(PathBuf, String)> {
    let home = dirs::home_dir()?;
    let path = home.join(".wgenty-code").join("WGENTY.md");
    let content = std::fs::read_to_string(&path).ok()?;
    if content.is_empty() { None } else { Some((path, content)) }
}

pub fn read_user_global_rules() -> Vec<(PathBuf, String)> {
    let Some(home) = dirs::home_dir() else { return vec![]; };
    let rules_dir = home.join(".wgenty-code").join("rules");
    let Ok(entries) = std::fs::read_dir(&rules_dir) else { return vec![]; };
    let mut files: Vec<_> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|x| x == "md").unwrap_or(false))
        .filter(|e| e.file_type().ok().map(|t| t.is_file()).unwrap_or(false))
        .collect();
    files.sort_by_key(|e| e.file_name());
    files.into_iter()
        .filter_map(|e| {
            let path = e.path();
            let content = std::fs::read_to_string(&path).ok()?;
            if content.is_empty() { None } else { Some((path, content)) }
        })
        .collect()
}
```

### 3.2 调用链改造

#### `src/tui/app/input.rs`（删除 fire-and-forget）

```rust
// 删除 162-188 行附近的 tokio::spawn(async move { hm.fire(...) })
// 改为：只构造 PendingInput，不 fire hook
```

#### `src/tui/agent/mod.rs::process_input_inner`（新增 fire + reminder 注入）

```rust
async fn process_input_inner(&mut self, input: String) -> Result<...> {
    // 1. Fire UserPromptSubmit hook（同步 await，10s timeout）
    let outcomes = self.hook_manager
        .fire(&HookEvent::UserPromptSubmit, &input)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!("UserPromptSubmit hook failed: {}, continuing", e);
            vec![]
        });

    // 2. 收集 hook 注入
    let injections = collect_injections(&outcomes);

    // 3. 构造 reminder
    let reminder = build_user_turn_reminder(&self.prompt_context, &injections);

    // 4. 拼 user message: to_model 给请求；to_transcript 给 TUI
    let user_content = match &reminder {
        Some(r) => format!("{}\n\n{}", r.to_model, input),
        None => input.clone(),
    };

    // 5. 推入 history
    self.history.push(ChatMessage::user(&user_content));

    // 6. TUI transcript 投递（如有 visible reminder）
    if let Some(r) = &reminder {
        if let Some(transcript) = &r.to_transcript {
            self.ui_tx.send(UIMessage::system(transcript.clone()))?;
        }
    }

    // ... 现有发送逻辑
}
```

#### `src/prompts/mod.rs`（移除 Layer 7/8）

```rust
// 删除现有 197-213 行的 Layer 7 (AGENTS.md) 和 Layer 8 (WGENTY.md) 的 system_messages.push(...)。
// 保留 PromptContextBuilder::with_wgenty_md / with_agents_md 签名不变——
// 数据仍存进 ctx，但 assemble_instructions 不再把它们推入 system 链。
// reminder builder 从 ctx 读取相同字段。
```

### 3.3 `build_user_turn_reminder` 完整逻辑

```rust
pub fn build_user_turn_reminder(
    ctx: &PromptContext,
    hook_injections: &[InjectedFragment],
) -> Option<ReminderOutput> {
    // ── 收集所有 segments ──────────────────────────────
    let mut segments: Vec<Segment> = vec![];

    // 1. 用户级 WGENTY.md
    if let Some((path, content)) = read_user_global_instructions() {
        segments.push(Segment::file(path, USER_INSTRUCTIONS_DESC, content));
    }
    // 2. 用户级 rules/*.md（字母序）
    for (path, content) in read_user_global_rules() {
        segments.push(Segment::file(path, USER_INSTRUCTIONS_DESC, content));
    }
    // 3. 项目 WGENTY.md（sections join）
    if !ctx.wgenty_md_sections.is_empty() {
        let path = ctx.project_root.as_ref()
            .map(|p| p.join("WGENTY.md"))
            .unwrap_or_else(|| PathBuf::from("WGENTY.md"));
        let content = ctx.wgenty_md_sections.join("\n\n");
        segments.push(Segment::file(path, PROJECT_INSTRUCTIONS_DESC, content));
    }
    // 4. 项目 AGENTS.md（sections join）
    if !ctx.agents_md_sections.is_empty() {
        let path = ctx.project_root.as_ref()
            .map(|p| p.join("AGENTS.md"))
            .unwrap_or_else(|| PathBuf::from("AGENTS.md"));
        let content = ctx.agents_md_sections.join("\n\n");
        segments.push(Segment::file(path, PROJECT_AGENTS_DESC, content));
    }

    // 早退：四源全缺且无 hook
    if segments.is_empty() && hook_injections.is_empty() {
        return None;
    }

    // ── 渲染 to_model 与 to_transcript ────────────────
    let mut to_model = String::from("<system-reminder>\n");
    let mut to_transcript = String::from("<system-reminder>\n");

    to_model.push_str(REMINDER_PREAMBLE_OPENING);
    to_transcript.push_str(REMINDER_PREAMBLE_OPENING);

    for seg in &segments {
        let block = format!("\n{}\n\n{}\n", seg.header(), seg.content);
        to_model.push_str(&block);
        to_transcript.push_str(&block);
    }

    // hook 注入：to_model 全部，to_transcript 仅 Visible
    let mut transcript_has_hook = false;
    for frag in hook_injections {
        let block = format!(
            "\nContents of {} (dynamic hook injection):\n\n{}\n",
            frag.source_label, frag.content
        );
        to_model.push_str(&block);
        if frag.visibility == LayerVisibility::Visible {
            to_transcript.push_str(&block);
            transcript_has_hook = true;
        }
    }

    to_model.push_str("\n");
    to_model.push_str(REMINDER_PREAMBLE_CLOSING);
    to_model.push_str("\n</system-reminder>");

    to_transcript.push_str("\n");
    to_transcript.push_str(REMINDER_PREAMBLE_CLOSING);
    to_transcript.push_str("\n</system-reminder>");

    // to_transcript 是否非空：取决于是否有 segments 或 visible hook
    let transcript_has_content = !segments.is_empty() || transcript_has_hook;

    Some(ReminderOutput {
        to_model,
        to_transcript: if transcript_has_content { Some(to_transcript) } else { None },
    })
}
```

---

## 4. 风险与取舍

| ID | 风险 | 严重度 | 缓解 |
|----|------|--------|------|
| R1 | Hook fire 改 await 导致 Thinking 状态延后展现 | Low | 10s timeout + warning log + push_system_message 兜底 |
| R2 | reminder 内容污染用户原文阅读 | Low | `to_transcript` 仅在有可见内容时投递，TUI 显示为独立 system 块 |
| R3 | hook 内容与文件内容视觉混淆 | Low | `Contents of hook:<Event>:<idx> (dynamic hook injection):` 标头与文件段视觉对齐但描述不同 |
| R4 | 每轮多消耗 ~1.5k+ input tokens | Med | prompt caching 命中后实际成本低；超阈值警告兜底 |
| R5 | `dirs::home_dir() = None`（CI / headless） | Low | reader 静默降级为空，集成测覆盖 |
| R6 | Hook 命令崩溃或超时 | Low | `unwrap_or_else` 捕获 + warning + 空 outcomes 继续；turn 不阻塞 |
| R7 | `# wgentyMd` 标题失去 Claude 模型先验 | Low | 接受；项目身份一致性优先；反悔代价极低（改一行常量） |
| R8 | reminder 拼接顺序错误导致来源错位 | Low | 段顺序在代码中固化（用户级 → 项目级 → hook），单测覆盖 |
| R9 | BREAKING 无迁移路径 | Med | v0.1.0 阶段可接受；CHANGELOG 明确标注；回滚 = git revert |
| R10 | transcript 投递晚于 Thinking 状态 | Low | 已知限制，记入 §7 KNOWN，本期不修 |

### 主要 Trade-offs

- **T1 — UI 阻塞 vs Hook 时序漂移**：选时序漂移（毫秒级，TUI 不可见），换 UI 完全不阻塞。
- **T2 — 单一构造器 vs 多调用点**：选单一 `build_user_turn_reminder` 入口，失去"部分注入"灵活性，换简化测试矩阵。
- **T3 — 整文件 vs 分节注入**：选整文件（项目 sections 重新 join），符合"参考手册"语义。
- **T4 — hook 内容不计入 token 预算**：动态、每轮变；用整体 conversation_history 上限兜底。

---

## 5. 测试策略

### 5.1 单元测试（12 个，`src/prompts/mod.rs::tests` + `src/runtime/hooks/mod.rs::tests` + `src/utils/project.rs::tests`）

| # | 名称 | 验证点 |
|---|------|--------|
| U1 | `reminder_full_four_sources_snapshot` | 4 源齐全 → 完整 reminder 文本（顺序、缩进、preamble、来源标注） |
| U2 | `reminder_missing_user_wgenty_no_empty_header` | 用户级 WGENTY.md 缺失 → 跳过该段、无空标题 |
| U3 | `reminder_all_missing_returns_none` | 4 源全缺 + 无 hook → 返回 `None` |
| U4 | `reminder_user_rules_alphabetical_order` | rules/*.md 按字母序 |
| U5 | `reminder_absolute_paths_in_attribution` | 来源标注路径是绝对路径 |
| U6 | `reminder_hook_priority_sorting` | hook 注入按 priority asc，ties 保留传入顺序 |
| U7 | `reminder_internal_visibility_excludes_transcript` | `Internal` 仅进 `to_model`，不进 `to_transcript` |
| U8 | `reminder_visible_hook_in_both_outputs` | `Visible` 同时进两个输出 |
| U9 | `assemble_instructions_no_layer_7_8` | system_messages 不再包含 `# AGENTS.md` / `# WGENTY.md — 项目规则与约定` |
| U10 | `collect_injections_empty_outcomes` | 空 outcomes → 空 Vec |
| U11 | `collect_injections_multiple_with_priority` | 多 outcome + 优先级排序 |
| U12 | `read_user_global_rules_ignores_subdirs` | 忽略子目录 + 非 .md |

### 5.2 集成测试（7 个，`tests/system_reminder.rs` 新建）

| # | 名称 | 验证点 |
|---|------|--------|
| I1 | `first_turn_user_message_contains_reminder` | 首轮 user message 头部含 `<system-reminder>` |
| I2 | `second_turn_reminder_reappears` | 第二轮 user message 再次包含 reminder（per-turn 验证） |
| I3 | `runtime_file_modification_reflected_next_turn` | 中途修改 `WGENTY.md` → 下一轮 reminder 内容更新 |
| I4 | `hook_inject_content_end_to_end` | 配置 `UserPromptSubmit` hook 返回 `EXTRA` → 下一轮 user message 含 `EXTRA` |
| I5 | `internal_visibility_not_in_transcript` | `Internal` 注入：模型可见，TUI transcript 不显示 |
| I6 | `hook_timeout_graceful_degradation` | Hook 10s 超时 → warning log + 空 outcomes + turn 继续 |
| I7 | `subagent_does_not_receive_reminder` | Subagent 路径的 user message 不含 reminder |

### 5.3 验收场景覆盖

12 验收场景全部覆盖至少 1 个测试用例：

| 场景 | 覆盖测试 |
|------|---------|
| 1. user message 含 `<system-reminder>` | I1 |
| 2. 4 段内容 | U1 |
| 3. 每段以 `Contents of ...` 开头 | U5 |
| 4. 块首 OVERRIDE preamble | U1 |
| 5. 块尾 may-or-may-not-be-relevant preamble | U1 |
| 6. 第二轮再次出现 | I2 |
| 7. 用户级 WGENTY.md 缺失优雅降级 | U2 |
| 8. 用户级 rules/ 缺失或空 | U2 (扩展 case) |
| 9. token 超阈值一次性提示 | 单测覆盖（§3.2 Token 预算） |
| 10. 硬切 Layer 7/8 | U9 |
| 11. Hook inject 端到端 | I4 |
| 12. ≥6 个新增测试 | U1-U12 + I1-I7 = 19 |

---

## 6. 实施顺序

按 `tasks.md` 9 节执行：

1. **§1 数据结构与 readers**（`InjectedFragment` + `collect_injections` + `read_user_global_*` + `PromptContext::project_root`）→ 12 个单测
2. **§2 Reminder builder**（`build_user_turn_reminder` + 常量 + 辅助函数）→ 8 个单测
3. **§3 请求构造层接入**（`tui/agent/mod.rs` 注入点 + `tui/app/input.rs` 删除 fire-and-forget）→ 2 个集成测
4. **§4 移除旧 Layer + 适配 builder**（Layer 7/8 硬切 + `with_project_root` 调用）→ 1 个单测
5. **§5 Hook injection 接通**（验证 `HookOutcome.injected_content` 链路 + 集成）→ 2 个集成测
6. **§6 Token 预算警告**（改造现有警告逻辑）→ 2 个单测
7. **§7 Documentation & polish**（CHANGELOG + 项目根 WGENTY.md 说明段 + 示例 rule 文件 + clippy/fmt）
8. **§8 验证**（手工运行 repl 验证 4 个真实场景）
9. **§9 Open Questions 收尾**（本设计已闭合 O1/O2/O3）

### 回滚策略

单 commit 落地，问题严重时 `git revert`。无 feature flag。v0.1.0 阶段可接受。

---

## 7. 已知限制（KNOWN）

- **K1**: `transcript` 投递晚于 Thinking 状态展现（毫秒级）。本期不修。
- **K2**: Subagent 路径手动通过"不调用 builder"实现隔离，编译期不检查。未来如要 subagent 也注入需手动加调用。
- **K3**: 用户级 WGENTY.md 大文件无切片机制（全文注入）。token 预算警告兜底。
- **K4**: `# wgentyMd` 标题失去 Claude 模型对 `# claudeMd` 的内置先验。可接受。

---

## 8. Spec Patch

回写目标：`openspec/changes/system-reminder-channel/specs/hook-lifecycle-complete/spec.md`

在现有 ADDED Requirements 之前追加 MODIFIED Requirements 节，修订原 `UserPromptSubmit hook fires on every input submission`：

```markdown
## MODIFIED Requirements

### Requirement: UserPromptSubmit hook fires before agent turn starts

The system SHALL fire `UserPromptSubmit` hooks inside the agent turn task and `await` their outcomes, so that injected content can be consumed by the next outgoing user message in the model request.

**Previous behavior**: hooks were fired via `tokio::spawn` from the TUI input handler the instant the user submitted, with outcomes discarded.

**New behavior**: hooks fire inside `AgentLoop::process_input_inner` at the start of each turn task. The fire is `await`-ed (not spawn-and-forget) and outcomes are passed to the reminder builder. Hook execution is bounded by a 10-second timeout; on timeout the turn proceeds with empty outcomes.

#### Scenario: Hook fires inside agent turn task
- **WHEN** the user submits a prompt and a `UserPromptSubmit` hook is configured
- **THEN** the hook SHALL fire inside the agent turn task before the user message is sent to the model
- **AND** the hook outcomes SHALL be consumed by the reminder builder for `injected_content` extraction

#### Scenario: Hook timeout degrades gracefully
- **WHEN** a `UserPromptSubmit` hook does not complete within 10 seconds
- **THEN** the system SHALL log a warning
- **AND** proceed with empty outcomes
- **AND** the user turn SHALL continue without blocking

#### Scenario: Hook does not fire on built-in commands
- **WHEN** the user input is a built-in slash command (e.g. `/help`)
- **THEN** the `UserPromptSubmit` hook SHALL NOT fire (unchanged behavior)
```
