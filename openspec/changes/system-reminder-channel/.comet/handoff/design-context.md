# Comet Design Handoff

- Change: system-reminder-channel
- Phase: design
- Mode: compact
- Context hash: fb73328403d45730d7603de7b269c75d0de81a8867a9678ab682699df2f40b64

Generated-by: comet-handoff.sh

OpenSpec remains the canonical capability spec. This handoff is a deterministic, source-traceable context pack, not an agent-authored summary.

## openspec/changes/system-reminder-channel/proposal.md

- Source: openspec/changes/system-reminder-channel/proposal.md
- Lines: 1-51
- SHA256: 90d2cd784d93f9ec10d61fc0aadd04718e4d95d708029311c91e7f197d0ee406

```md
## Why

wgenty-code 目前把项目根 `WGENTY.md` 与 `AGENTS.md` 作为静态 system message 拼进系统提示链（`prompts/mod.rs` 的 Layer 7、Layer 8），缺少类似 Claude Code 的 `<system-reminder>` 注入通道。这导致：

1. **失忆**：长对话中规则随上下文窗口位置漂移，模型逐渐忽略；
2. **污染**：用户级规则（流程守卫、跨项目偏好）无处安放，要么塞进项目文件污染团队配置，要么塞进 `developer_instructions` 但语义混淆；
3. **缓存语义错位**：会变的项目说明拼在 system prompt，让原本应该稳定可缓存的系统提示头部失稳；
4. **半成品 Hook 通道**：`HookAction::InjectContext` 数据结构完整且有测试，但 `outcomes[].injected_content` 在生产代码中无人消费，动态内容无法注入上下文。

完全对齐 Claude Code 的 `<system-reminder>` 模式：把项目说明 + 全局规则放进**每轮 user message 头部的 reminder 块**，双层 preamble 包裹，逐文件来源标注。

## What Changes

- **新增**：用户级全局指令文件 `~/.wgenty-code/WGENTY.md`（reader 与 Q5 设定）。
- **新增**：用户级规则目录 `~/.wgenty-code/rules/*.md`（字母序，无脑全文注入；不加 frontmatter 过滤）。
- **新增**：`<system-reminder>` 注入通道——每轮把 4 段内容（用户 WGENTY.md / 用户 rules / 项目 WGENTY.md / 项目 AGENTS.md）拼进 user message 头部，含双层 preamble 与 `Contents of <绝对路径> (<描述>):` 来源标注。
- **新增**：动态注入源 —— hook `UserPromptSubmit` 等事件输出的 `injected_content` 接入同一 reminder 通道（接通 `HookAction::InjectContext` 半成品）。
- **BREAKING**：移除 `prompts/mod.rs` Layer 7、Layer 8 的 system message push（`# AGENTS.md` 与 `# WGENTY.md — 项目规则与约定`）。项目说明唯一来源切换到 reminder 通道。
- **修改**：token 预算警告（`tui/app/mod.rs` 现仅算 WGENTY+AGENTS）扩展为计算整个 reminder 块。
- **保留**：现有 `PromptContextBuilder::with_wgenty_md` / `with_agents_md` 公开 API 签名不变，仅实现重写，调用方无感。

## Capabilities

### New Capabilities

- `system-reminder-injection`: `<system-reminder>` 块的构造、来源标注、双层 preamble、每轮注入 user message 头部的契约；多源聚合（全局指令、全局规则、项目说明）的顺序与优雅降级（缺失文件跳过）；token 预算计算与一次性警告；与 Claude Code 注入结构 1:1 对齐。

### Modified Capabilities

- `hook-lifecycle-complete`: `HookAction::InjectContext` 从「数据结构 + 单测」状态转为「生产代码消费 `outcomes[].injected_content`」状态；动态注入内容在下一轮 user message 中可被模型看到，并按 `priority` 与 reminder 块协调位置。

## Impact

**代码**
- `src/utils/project.rs`：保留 `read_wgenty_md_sections` / `read_agents_md_sections`，新增 `read_user_global_instructions`、`read_user_global_rules`。
- `src/prompts/mod.rs`：删除 Layer 7、Layer 8；新增 reminder builder（独立函数 + 公开 API）；调整 `AssembledInstructions` 输出形态以承载 reminder。
- `src/tui/agent/`（请求构造层）：在每轮发送给模型前，把 reminder 拼进 user message 头部。
- `src/tui/app/mod.rs`、`src/tui/app/event.rs`：token 预算警告逻辑扩展；调用点适配新 builder。
- `src/runtime/hooks/mod.rs`：把 `outcomes[].injected_content` 聚合输出给请求构造层，与静态 reminder 协调。

**外部约束**
- 新增两条用户级文件源（`~/.wgenty-code/WGENTY.md`、`~/.wgenty-code/rules/*.md`）—— 不存在时优雅降级。
- 与 `~/.wgenty-code/settings.json` 的 hooks 配置（已包含 PreToolUse 阶段守卫）协作，新增 `UserPromptSubmit` hook 注入路径示例。

**测试**
- 至少 6 个新增单测/集成测覆盖 12 条验收场景（reminder 结构、per-turn 重发、缺失降级、硬切验证、hook inject 接通、token 预算计算等）。

**非影响**
- subagent prompt 构造 —— 明确排除（Q5）。
- skills / permissions / MCP / collaboration —— 不动。
- 现有 `~/.wgenty-code/skills/` 同步机制 —— 不动。
```

## openspec/changes/system-reminder-channel/design.md

- Source: openspec/changes/system-reminder-channel/design.md
- Lines: 1-200
- SHA256: 2945175eb1f39c018a487438e57be83e3da3c64ceced242ca178f7022a2abca2

[TRUNCATED]

```md
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
```

Full source: openspec/changes/system-reminder-channel/design.md

## openspec/changes/system-reminder-channel/tasks.md

- Source: openspec/changes/system-reminder-channel/tasks.md
- Lines: 1-69
- SHA256: 9dbb1789576ccc4a0f50dafdc72c373d62ad83b3f1509e762ef1b29cbc04d1e1

```md
## 1. 数据结构与 readers

- [ ] 1.1 在 `src/runtime/hooks/mod.rs` 新增 `InjectedFragment` 公共结构（`content`/`priority`/`visibility`/`source_label`），并新增 `collect_injections(&[HookOutcome]) -> Vec<InjectedFragment>` 辅助函数；增加单测覆盖空 outcomes、单 outcome、多 outcome 排序场景
- [ ] 1.2 在 `src/utils/project.rs` 新增 `read_user_global_instructions() -> Option<(PathBuf, String)>` 读取 `~/.wgenty-code/WGENTY.md`，使用 `dirs::home_dir()`，无 home / 文件不存在均返回 `None`；增加单测覆盖存在、缺失、空文件三种情况
- [ ] 1.3 在 `src/utils/project.rs` 新增 `read_user_global_rules() -> Vec<(PathBuf, String)>` 扫 `~/.wgenty-code/rules/*.md` 顶层非空 .md 文件，按字母序返回；忽略子目录与非 .md；增加单测覆盖空目录、多文件排序、子目录忽略
- [ ] 1.4 扩展 `PromptContext` 增加 `project_root: Option<PathBuf>` 字段及 `with_project_root(path)` builder 方法；保持向后兼容（默认 `None`）

## 2. Reminder builder

- [ ] 2.1 在 `src/prompts/mod.rs` 新增私有常量 `REMINDER_PREAMBLE_OPENING` 和 `REMINDER_PREAMBLE_CLOSING`，精确复刻 Claude Code 措辞（含 `# claudeMd` 与闭合 preamble 6 空格缩进）
- [ ] 2.2 实现 `build_user_turn_reminder(ctx: &PromptContext, hook_injections: &[InjectedFragment]) -> Option<String>`：聚合 4 个文件源 + hook 注入按 priority 排序，按 D6 文本骨架渲染；4 源全缺且无 hook 注入返回 `None`
- [ ] 2.3 实现来源标注辅助函数 `render_attribution_header(absolute_path: &Path, description: &str) -> String`，统一输出 `Contents of <absolute-path> (<description>):` 格式
- [ ] 2.4 单测：全 4 源齐全的完整 reminder 文本快照（含具体顺序、缩进、preamble）
- [ ] 2.5 单测：缺失各文件源时不出现空标题、不报错；4 源全缺且无 hook 返回 `None`
- [ ] 2.6 单测：来源标注路径是绝对路径
- [ ] 2.7 单测：rules/*.md 字母序
- [ ] 2.8 单测：hook 注入按 priority asc 排序，ties 保持调用方传入顺序

## 3. 请求构造层接入

- [ ] 3.1 查找 `src/tui/agent/` 下构造发送给模型的 user message 的位置（预计在 stream.rs 或 mod.rs 的请求装配处），把 reminder 注入路径插入：构造 user content 字符串时先拼 reminder，再拼原始 prompt
- [ ] 3.2 把 `tui/app/input.rs:181` 的 `tokio::spawn(async move { hm.fire(...) })` 改为 `await` 同步执行，并把 outcomes 通过 PendingInput 或等价通道传给请求构造层
- [ ] 3.3 在请求构造路径调用 `collect_injections(&outcomes)` 提取 `InjectedFragment`，传给 `build_user_turn_reminder`
- [ ] 3.4 集成测：模拟 user 输入，断言第一轮 user message content 头部包含 `<system-reminder>` 块
- [ ] 3.5 集成测：连续两轮 user 输入，第二轮 user message 再次包含 reminder（per-turn 验证）

## 4. 移除旧 Layer + 适配 builder

- [ ] 4.1 删除 `src/prompts/mod.rs` Layer 7（AGENTS.md）和 Layer 8（WGENTY.md）的 system message push 代码块
- [ ] 4.2 `PromptContextBuilder::with_wgenty_md` / `with_agents_md` 保持签名不变，仅在 `assemble_instructions` 内部确保数据被 reminder builder 而不是 system message push 使用
- [ ] 4.3 `src/tui/app/mod.rs` 在构造 PromptContext 时同时调用 `with_project_root(std::env::current_dir())`，让 reminder builder 能渲染绝对路径
- [ ] 4.4 单测：assembled system_messages 中**不再**出现 `# AGENTS.md` 或 `# WGENTY.md — 项目规则与约定` 文本（硬切验证）

## 5. Hook injection 接通

- [ ] 5.1 验证 `UserPromptSubmit` hook 的 `HookOutcome` 中 `injected_content` 已经被正确填充（如未填充则在 `run_inject_action` 路径补齐）；增加单测保证 `HookAction::InjectContext` 的 outcomes 包含 `injected_content`
- [ ] 5.2 在请求构造层把 hook 收集到的 `InjectedFragment` 与文件源一起传给 reminder builder，验证多个 hook 时优先级和顺序正确
- [ ] 5.3 集成测：在 `settings.json` 配置 `UserPromptSubmit` hook 返回 `"injected_content": "EXTRA"`，断言下一轮 user message 中可见 `EXTRA` 字符串
- [ ] 5.4 集成测：配置两个 hook（priority 不同），断言注入内容按 priority 排序

## 6. Token 预算警告

- [ ] 6.1 把 `src/tui/app/mod.rs` 现有"WGENTY+AGENTS 超阈值警告"改造为"完整 reminder 块超阈值警告"
- [ ] 6.2 警告触发位置：首次构造 reminder 时计算（不是 session 启动时），保留"每 session 仅一次"语义
- [ ] 6.3 hook 注入内容**不计入**预算（动态、每轮变）；只计入 4 个文件源 + preamble overhead
- [ ] 6.4 单测：超阈值触发警告，二次构造不重复触发
- [ ] 6.5 单测：未超阈值不发警告

## 7. Documentation & polish

- [ ] 7.1 在 `WGENTY.md`（项目根）新增一段 "Context injection channels" 说明 `~/.wgenty-code/WGENTY.md` + `~/.wgenty-code/rules/` 用法（注意：是文档说明，不是实际放规则）
- [ ] 7.2 CHANGELOG 标记 BREAKING："项目说明改走 system reminder 通道，不再出现在 system prompt 链路"
- [ ] 7.3 在 `~/.wgenty-code/rules/` 新建示例文件 `comet-phase-guard.md`（从 `~/.claude/rules/comet-phase-guard.md` 拷贝），用于 dogfood 本次实现
- [ ] 7.4 运行完整 `cargo test` 与 `cargo clippy -- -D warnings`，零 warning 通过
- [ ] 7.5 运行 `cargo fmt -- --check`，格式合规

## 8. 验证

- [ ] 8.1 验证 12 条验收场景全部覆盖至少 1 个测试用例
- [ ] 8.2 启动 `wgenty-code repl`，输入任意 prompt，用 logs / debug toggle 确认 user message 内容含 reminder 块
- [ ] 8.3 删除 `~/.wgenty-code/WGENTY.md`，再次输入 prompt，确认无报错、无空标题
- [ ] 8.4 配置 `UserPromptSubmit` hook 返回 inject content，重启 repl 验证 hook 注入端到端工作
- [ ] 8.5 用 `cargo run -- repl --prompt "X"` 单次查询模式同样验证 reminder 注入

## 9. 解决 design doc 的 Open Questions

- [ ] 9.1 O1: 决定 `# claudeMd` 标题保留 vs 改名（design 阶段加载 brainstorming 时定）
- [ ] 9.2 O2: 验证 `tui/app/input.rs` UserPromptSubmit fire 改 await 不引入死锁（design 阶段读 start_next_turn 并发模型）
- [ ] 9.3 O3: 定 `LayerVisibility::Internal` 在 TUI transcript 层的具体过滤实现路径
```

## openspec/changes/system-reminder-channel/specs/hook-lifecycle-complete/spec.md

- Source: openspec/changes/system-reminder-channel/specs/hook-lifecycle-complete/spec.md
- Lines: 1-100
- SHA256: ec18c823e594d3fcbddab6845de8f9149e6512dcc9359d0894005374010043e2

[TRUNCATED]

```md
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

---

## ADDED Requirements

### Requirement: HookAction::InjectContext content reaches the next user turn

The system SHALL consume `outcomes[].injected_content` produced by hook actions (especially from `UserPromptSubmit` hooks) and surface the content to the next outgoing user message in the model request.

#### Scenario: UserPromptSubmit hook returns injected content
- **WHEN** a `UserPromptSubmit` hook is configured with a `HookAction::InjectContext` action that produces `injected_content = "<extra context>"`
- **AND** the hook fires after the user submits a prompt
- **THEN** the next outgoing user message to the model SHALL contain the string `<extra context>` accessible to the model
- **AND** the injection SHALL persist independently from static reminder file sources

#### Scenario: Multiple injecting hooks are concatenated
- **WHEN** two hooks both produce `injected_content` for the same `UserPromptSubmit` event
- **THEN** both contents SHALL be included in the next user message
- **AND** the concatenation order SHALL follow the order in which the hooks are declared in `settings.json`

#### Scenario: Hook returns no injected content
- **WHEN** a hook fires but its outcome's `injected_content` is `None` or empty
- **THEN** no extra content SHALL be added to the next user message from that hook

#### Scenario: Hook with continue_execution=false still injects
- **WHEN** a hook returns `{ continue_execution: false, injected_content: "blocked context" }`
- **THEN** the turn SHALL be blocked (per existing semantics)
- **AND** the injected content SHALL still be appended to the next user message that does eventually proceed

---

### Requirement: Injected hook content coordinates with reminder block

The system SHALL place hook-injected content in a deterministic position relative to the static `<system-reminder>` block when both are present.

#### Scenario: Reminder present, hook injects content
- **WHEN** both a non-empty `<system-reminder>` block (from file sources) and a non-empty hook `injected_content` exist for the same user turn
- **THEN** the user message content SHALL contain the `<system-reminder>` block first, followed by the hook-injected content, followed by the user's original prompt text

#### Scenario: Only hook content (no reminder block)
- **WHEN** all four reminder file sources are missing but a hook produces `injected_content`
- **THEN** the user message SHALL contain only the hook-injected content followed by the user's prompt
- **AND** no empty `<system-reminder>` tags SHALL appear

#### Scenario: Only reminder block (no hook content)
- **WHEN** the reminder block is non-empty but no hook produces `injected_content`
- **THEN** the user message SHALL contain only the `<system-reminder>` block followed by the user's prompt

---

### Requirement: Inject visibility honored

The system SHALL respect the `LayerVisibility` field on `HookAction::InjectContext` when wiring content into the next user message.

#### Scenario: Visible layer reaches the model
- **WHEN** a hook injects with `visibility: Visible`
```

Full source: openspec/changes/system-reminder-channel/specs/hook-lifecycle-complete/spec.md

## openspec/changes/system-reminder-channel/specs/system-reminder-injection/spec.md

- Source: openspec/changes/system-reminder-channel/specs/system-reminder-injection/spec.md
- Lines: 1-165
- SHA256: 07ecf41b8055df4b8a3519a1e702f776a6d16467e17dde4566f4ecfddda3621e

[TRUNCATED]

```md
## ADDED Requirements

### Requirement: System reminder block injection per user turn

The system SHALL inject a `<system-reminder>` block at the head of every user message sent to the model. The block SHALL appear before the user's actual prompt text in the same message payload.

#### Scenario: First turn injection
- **WHEN** the user submits any prompt for the first time in a session
- **THEN** the outgoing request to the model SHALL include a user message whose content begins with a `<system-reminder>` block followed by the user's prompt text

#### Scenario: Subsequent turn injection
- **WHEN** the user submits a second or later prompt in the same session
- **THEN** the outgoing request SHALL again contain the `<system-reminder>` block at the head of the new user message
- **AND** the reminder content SHALL be re-evaluated from current file sources (not cached from the first turn)

#### Scenario: System prompt remains clean
- **WHEN** the reminder block is constructed
- **THEN** none of the reminder content SHALL appear in any `ChatMessage::system` of the system prompt chain
- **AND** the `system_messages` Vec returned by the prompt assembler SHALL NOT contain `# AGENTS.md` or `# WGENTY.md — 项目规则与约定` layers

---

### Requirement: Four content source layers in deterministic order

The reminder block SHALL aggregate content from up to four file sources, in this exact order: user-global instructions, user-global rules, project instructions, project agent conventions.

#### Scenario: All four sources present
- **WHEN** `~/.wgenty-code/WGENTY.md` exists, `~/.wgenty-code/rules/*.md` contains at least one file, project root `WGENTY.md` exists, and project root `AGENTS.md` exists
- **THEN** the reminder block SHALL include, in this order: user-global WGENTY content, then each `rules/*.md` file in alphabetical order by filename, then project WGENTY content, then project AGENTS content

#### Scenario: User global WGENTY.md missing
- **WHEN** `~/.wgenty-code/WGENTY.md` does not exist
- **THEN** the user-global instructions section SHALL be omitted from the reminder block
- **AND** no empty heading, placeholder, or error indicator SHALL appear in its place
- **AND** the remaining sections SHALL render in their normal order without gaps

#### Scenario: User rules directory missing or empty
- **WHEN** `~/.wgenty-code/rules/` does not exist, or exists but contains no `*.md` files
- **THEN** the user-global rules section SHALL be omitted from the reminder block

#### Scenario: Project WGENTY.md missing
- **WHEN** the project root `WGENTY.md` does not exist
- **THEN** the project instructions section SHALL be omitted from the reminder block

#### Scenario: Project AGENTS.md missing
- **WHEN** the project root `AGENTS.md` does not exist
- **THEN** the project agent conventions section SHALL be omitted from the reminder block

#### Scenario: All sources missing
- **WHEN** none of the four file sources exist
- **THEN** the reminder block SHALL be omitted entirely (no preamble, no closing, no empty `<system-reminder>` tags)
- **AND** the user message SHALL be sent as if no reminder mechanism existed

---

### Requirement: Rules directory alphabetical ordering

When multiple files exist under `~/.wgenty-code/rules/`, the system SHALL include them in case-sensitive byte-wise ascending filename order.

#### Scenario: Multiple rule files
- **WHEN** `~/.wgenty-code/rules/` contains `comet-phase-guard.md`, `apple.md`, and `zebra.md`
- **THEN** the reminder block SHALL include them in this order: `apple.md`, `comet-phase-guard.md`, `zebra.md`

#### Scenario: Non-markdown files in rules directory
- **WHEN** `~/.wgenty-code/rules/` contains a file `notes.txt` alongside `foo.md`
- **THEN** the reminder block SHALL include only `foo.md`
- **AND** `notes.txt` SHALL be ignored without error

#### Scenario: Subdirectories in rules directory
- **WHEN** `~/.wgenty-code/rules/` contains a subdirectory `archive/old.md`
- **THEN** the reminder block SHALL ignore the subdirectory and its contents
- **AND** only top-level `*.md` files SHALL be considered

---

### Requirement: Source attribution header per section

Each content section in the reminder block SHALL be preceded by a `Contents of <absolute-path> (<description>):` header line that identifies its source.

#### Scenario: User-global WGENTY.md attribution
```

Full source: openspec/changes/system-reminder-channel/specs/system-reminder-injection/spec.md

