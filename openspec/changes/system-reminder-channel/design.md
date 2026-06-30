## Context

wgenty-code 当前在 `src/prompts/mod.rs` 的 `assemble_instructions` 中以 8 层 `ChatMessage::system` 拼装系统提示。Layer 7（AGENTS.md）和 Layer 8（WGENTY.md）以静态 system message 形式承载项目级说明，长对话中权重衰减明显；`~/.wgenty-code/` 没有全局指令或规则文件源；`HookAction::InjectContext` 有完整数据结构和单测，但 `HookOutcome.injected_content` 在 `src/tui/agent/` 与 `src/tools/executor.rs` 调用 `fire()` 后的返回值上从未被消费。

参照对象：Claude Code 在每轮 user message 之前注入一段 `<system-reminder>` 块，块内包含双层 preamble + 多源内容 + 逐文件 `Contents of <path> (...)` 来源标注。本对话从注入到当前会话上下文反推得到结构，可作为 1:1 参照。

外部约束：项目仍为 `0.1.0`，可接受 BREAKING 切换；hooks 配置已通过本次会话前置工作落地（`~/.wgenty-code/settings.json` 注册了 PreToolUse `comet-hook-guard.sh`）。

## Goals / Non-Goals

**Goals:**
- 在 wgenty-code 中提供与 Claude Code 等价的 `<system-reminder>` 注入通道（结构、措辞、来源标注 1:1 对齐）。
- 聚合 4 个静态文件源 + 任意数量动态 hook 源，统一进入 reminder 通道；缺失文件优雅降级。
- 把 reminder 拼到每轮 user message 头部，而非 system prompt；硬切移除现有 Layer 7、Layer 8。
- 接通 `HookAction::InjectContext` 半成品：`UserPromptSubmit` hook 输出的 `injected_content` 在下一轮 user message 中可被模型看到。
- 提供 token 预算计算和一次性警告，避免 reminder 膨胀失控。

**Non-Goals:**
- 不改 subagent prompt 构造路径（明确排除）。
- 不为 rules/*.md 引入 frontmatter 过滤、条件加载、按 phase 切换（无脑全文注入）。
- 不为现有 system layer 行为做向后兼容（硬切）。
- 不改 skills/permissions/MCP/collaboration 链路。
- 不改 `~/.wgenty-code/skills/` 同步机制（另案）。
- 不引入新的可观测性子系统（沿用现有 tracing/log）。

## Decisions

### D1 — Reminder 文本的承载位置：拼进 user message 内容头部，不作为独立 system message

候选：
- (A) 作为新的 `ChatMessage::system`，紧贴当前 `Vec<ChatMessage::system>` 之后、user 之前。
- (B) 拼进 user 消息 content 字符串头部，与用户原文同一条 message。← **选定**
- (C) Anthropic API `system` 字段后追加。

**选 B**。理由：
1. 探查 Claude Code 实际行为后用户已明确 Q1=B；
2. 语义上 reminder 是"附属于本轮"的，留在 user 消息内更贴近"上下文"而非"规则"；
3. 系统提示保持稳定，命中 prompt caching 概率更高；
4. 不必新增 message 类型，复用现有 `ChatMessage::User { content }` 直接前置拼接。

代价：`src/tui/agent/` 的发送路径要在拼 user message 时调用 reminder builder；不能在 `assemble_instructions` 内完成全部工作。

### D2 — Reminder 构造的入口：新增公开函数 `build_user_turn_reminder()`

放在 `src/prompts/mod.rs`，签名草案：

```rust
pub fn build_user_turn_reminder(
    ctx: &PromptContext,
    hook_injections: &[InjectedFragment],
) -> Option<String>
```

返回 `Option<String>`：四个文件源全部缺失且无 hook 注入时返回 `None`，调用方据此决定是否前置拼接。`InjectedFragment` 是新增数据结构（见 D5），封装 hook `injected_content` + priority + visibility。

避免把 reminder 拼进 `AssembledInstructions`：那是 system prompt 的产物；reminder 走 user 消息路径，**两者应解耦**。

### D3 — 文件源 reader：两个新增、两个保留

`src/utils/project.rs`：
- 新增 `read_user_global_instructions() -> Option<String>` 读 `~/.wgenty-code/WGENTY.md`（单文件，按 `---` 拆 section 不再需要——reminder 直接整文件注入，保留原始格式）。
- 新增 `read_user_global_rules() -> Vec<(PathBuf, String)>` 读 `~/.wgenty-code/rules/*.md`，按字母序，返回 `(绝对路径, 内容)` 对（路径用于来源标注）。
- 保留 `read_wgenty_md_sections` / `read_agents_md_sections`：项目文件继续按 `---` 拆 section（保持现有行为），但在拼 reminder 时重新合并为单字符串。

`~/.wgenty-code/` 路径解析：从 `dirs::home_dir()` 派生，避免硬编码 `$HOME`。对 `dirs::home_dir() = None`（极少见的 CI 环境）优雅降级为"用户全局源全部缺失"。

### D4 — 项目 WGENTY.md / AGENTS.md 来源路径解析

`PromptContext` 当前不携带项目根绝对路径——`with_wgenty_md` / `with_agents_md` 只接收已经拆好的 `Vec<String>` sections。新增方案：

- 扩展 `PromptContext` 增加 `project_root: Option<PathBuf>` 字段，由 `src/tui/app/mod.rs` 在构造时填入 `std::env::current_dir()`。
- reminder builder 用 `project_root.join("WGENTY.md").display()` 渲染来源标注；若 sections 非空但 `project_root` 缺失，使用相对路径 `WGENTY.md` 作为兜底。

公开 API `with_wgenty_md` / `with_agents_md` 签名不变，新增 `with_project_root(path)` 配对方法，保持原有调用方编译通过。

### D5 — Hook injection 桥接：新增 `InjectedFragment` + 让 `fire()` 调用方收集

`src/runtime/hooks/mod.rs`：
- 新增 pub struct：
  ```rust
  pub struct InjectedFragment {
      pub content: String,
      pub priority: u8,
      pub visibility: LayerVisibility,
      pub source_label: Option<String>, // e.g. "from UserPromptSubmit hook"
  }
  ```
- `HookManager::fire(...)` 已返回 `Vec<HookOutcome>`；新增辅助函数 `collect_injections(outcomes: &[HookOutcome]) -> Vec<InjectedFragment>`，从 outcomes 中筛出非空 `injected_content`，配合 hook 定义的 priority/visibility。

`src/tui/agent/` 或对应请求构造层：在调用 `fire(&HookEvent::UserPromptSubmit, ...)` 后保留 outcomes，传给 `build_user_turn_reminder` 用作 `hook_injections` 参数。

`PreToolUse` hook 是否参与 reminder？**不参与**——`PreToolUse` 触发时机在主轮之中，不是"下一轮 user message 前"，注入会迟到。reminder 通道仅消费与"下一轮 user 消息"在时间上紧邻的事件：主要是 `UserPromptSubmit`，可能扩展到 `Stop`/`SessionStart`（保留扩展位但首版仅 `UserPromptSubmit`）。

### D6 — Reminder 文本骨架（精确格式）

```
<system-reminder>
As you answer the user's questions, you can use the following context:
# wgentyMd
Codebase and user instructions are shown below. Be sure to adhere to
these instructions. IMPORTANT: These instructions OVERRIDE any default
behavior and you MUST follow them exactly as written.

Contents of /Users/X/.wgenty-code/WGENTY.md (user's private global instructions for all projects):

<内容>

Contents of /Users/X/.wgenty-code/rules/<a>.md (user's private global instructions for all projects):

<内容>

Contents of /Users/X/.wgenty-code/rules/<b>.md (user's private global instructions for all projects):

<内容>

Contents of <project>/WGENTY.md (project instructions, checked into the codebase):

<内容>

Contents of <project>/AGENTS.md (project agent conventions, checked into the codebase):

<内容>

<hook-injected fragments, sorted by priority asc, ties by hook order>

      IMPORTANT: this context may or may not be relevant to your tasks.
      You should not respond to this context unless it is highly relevant
      to your task.
</system-reminder>
```

注意：
- 缩进精确复刻 Claude Code 的"前面留 6 空格"特征（用于视觉与语义弱化）。
- 段间空行：每段 `Contents of ...` 与内容之间 1 空行，段与段之间 1 空行。
- `# wgentyMd` 标题（O1 最终决策：方案 B 改名）。brainstorming 阶段曾考虑保留 `# claudeMd` 以复用 Claude 模型对该 token 的先验，但最终选择本地化为 `# wgentyMd` 以贴合 wgenty-code 项目身份；反悔成本极低（改一行常量）。详见 Open Question O1 的最终决议。

骨架中的 `# claudeMd` 字样是 Claude Code 参照对象的原文；wgenty-code 实现用 `# wgentyMd` 替换。除标题 token 外，preamble 措辞与来源标注格式与 Claude Code 1:1 对齐。

### D7 — Token 预算：复用 `tui/app/mod.rs` 现有警告位

现有逻辑（`tui/app/mod.rs:~136`）在 session 启动时一次性估算 WGENTY+AGENTS sections 字节数，超阈值发出 system message。

改造：
- 估算输入由"WGENTY+AGENTS sections"扩展为"完整 reminder 块文本（含 preamble + 来源标注 + 所有源）"。
- 估算时机从"session 启动一次"改为"首次构造 reminder 时一次"（更准确，因为用户全局源也算进去）。
- 沿用现有 `fires_once_per_session` 模式（一个 bool 标志）。

阈值默认值不变（沿用现有数）。reminder 不算 hook 注入内容（动态、每轮变）—— hook 注入若超大体积超额，由 hook 设计者负责。

### D8 — Subagent 排除策略：限定调用点

reminder builder 仅在 main session 的 user message 发送路径调用。`src/agent/`（subagent 运行时）有独立的 message 构造路径，**不调用** `build_user_turn_reminder()`。无需在 builder 内部加运行时判断——通过"哪些调用点连进来"实现物理隔离。

代价：未来若决定 subagent 也注入 reminder（Q5 反悔），需手动在 subagent 路径加调用，编译期检查不会提醒——可接受。

### D9 — 测试策略：单测 + 集成测

单测（`src/prompts/mod.rs::tests`）：
- 全 4 源齐全 → reminder 文本顺序/格式/来源标注正确
- 任一源缺失 → 优雅降级（无空标题）
- 全部缺失 → 返回 `None`
- rules/*.md 字母序
- 来源标注路径绝对化

集成测（`tests/system_reminder.rs` 新增）：
- 启动 TUI 模拟 user 输入，断言发出去的 user message 含 reminder 块
- 第二轮再次包含（per-turn 验证）
- 配置 UserPromptSubmit hook 返回 injected_content → 下一轮 user message 含该内容

至少 6 个新增测试，覆盖 12 条验收场景。

## Risks / Trade-offs

| Risk | Mitigation |
|---|---|
| `# claudeMd` 标题对 wgenty-code 项目身份不一致，未来用户困惑 | 设计 doc 阶段确定（O1）；如选择换名，提前在 spec 文本中固化措辞，避免实现期反复 |
| reminder 拼进 user message 头部可能干扰用户在 transcript 中阅读原始 prompt | TUI 展示层只渲染用户输入部分，reminder 不进 transcript 视觉层；保留 `LayerVisibility::Internal` 语义 |
| hook injected content 与 reminder 同源出现"内容混淆"（哪段是文件哪段是 hook） | hook 注入段附 `source_label`（如 `<!-- hook: comet-state-recover -->`），渲染时作为注释行可读但不污染 |
| 每轮多消耗 ~1.5k+ input tokens | 用户级 + 项目级文件大小通常稳定，prompt caching 命中后实际 cost 低；超阈值有警告兜底 |
| `dirs::home_dir() = None` 的 CI / headless 场景 | reader 静默降级为空，集成测覆盖这一路径 |
| hook 异步 fire 与 user message 构造的时序：hook 还没返回，user message 已经在发 | 主流程已是 `hm.fire(...).await` 后续推进，需保证 `UserPromptSubmit` hook 在 enqueue agent input 之前 await；现有 `tui/app/input.rs:181` 使用 `tokio::spawn` fire-and-forget 必须改为 `await` |
| BREAKING 切换无迁移路径 | 0.1.0 阶段可接受；CHANGELOG 明确写"项目说明改走 reminder 通道，不再出现在系统提示" |

## Migration Plan

1. 实现 reader、reminder builder、`InjectedFragment` 通道。
2. 在 `tui/agent/` 请求构造路径接入 reminder。
3. 改造 `tui/app/input.rs` 的 `UserPromptSubmit` fire 为 await 而非 spawn（同步要求）。
4. 移除 `prompts/mod.rs` 的 Layer 7、Layer 8；保留 `with_wgenty_md`/`with_agents_md` API 但语义改变。
5. 更新 `tui/app/mod.rs` 的 token 警告逻辑。
6. 新增测试覆盖 12 场景。
7. 文档：CHANGELOG 标记 BREAKING；README 一段说明 `~/.wgenty-code/WGENTY.md` 与 `~/.wgenty-code/rules/` 使用方法。

**回滚策略**：单 commit 落地结构调整后通过 revert 回退；不引入 feature flag。0.1.0 阶段可接受。

## Open Questions（全部已在 brainstorming 阶段闭合）

- **O1 — `# claudeMd` 标题保留与否** → **已决：方案 B，改名为 `# wgentyMd`**。项目身份一致性优先于复用 Claude 模型先验；反悔代价极低（改一行常量）。实现见 `src/prompts/mod.rs::REMINDER_PREAMBLE_OPENING`。
- **O2 — `tui/app/input.rs` UserPromptSubmit fire 从 spawn 改 await 是否影响其他时序** → **已决：方案 B，fire 移到 `AgentLoop::process_input_inner` 内 `await`**。死锁分析见 §2 D7：AgentLoop 在独立 task 内运行，与 UI render loop 解耦；10s timeout + `tracing::warn!` 降级兜底，无死锁路径。
- **O3 — `LayerVisibility::Internal` 对 hook 注入内容的精确行为** → **已决：方案 A，reminder builder 输出端分流**。`ReminderOutput { to_model, to_transcript }` 双轨输出；Internal 仅进 `to_model`，Visible 进两者。`to_transcript` 投递到 TUI 的链路（`AppEvent::SystemNotice`）作为已知限制 K1 延后。

> 三项 Open Questions 的实施 trace 见 `.comet/open-questions-resolution.md`。
