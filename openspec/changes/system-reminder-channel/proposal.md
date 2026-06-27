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
