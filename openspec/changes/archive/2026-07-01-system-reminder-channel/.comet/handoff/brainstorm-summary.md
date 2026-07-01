# Brainstorm Summary — system-reminder-channel

- Change: system-reminder-channel
- Date: 2026-06-27
- Status: ✅ brainstorming finalized, awaiting Design Doc creation

## 确认的技术方案

完全照 Claude Code 实现 `<system-reminder>` 注入通道，把项目说明 + 用户级规则从 system prompt 迁出到 per-turn user message 头部的 reminder 块；同时接通半成品的 hook `HookAction::InjectContext` 通道。

### 核心架构
- **注入位置**：`ChatMessage::user(reminder_text + &input)`，在 `AgentLoop::process_input_inner` 的 `history.push` 处装配
- **装配位置**：内部装配（AgentLoop 内），UI 主线程零阻塞
- **双轨输出**：`build_user_turn_reminder` 返回 `ReminderOutput { to_model, to_transcript }`
  - 4 个静态文件源永远进 `to_model`，不进 transcript
  - Hook injection 按 `LayerVisibility` 分流：Visible → 两边都进；Internal → 仅 to_model
- **Hook fire 时机**：从 `tui/app/input.rs` 的 `tokio::spawn` fire-and-forget 改为 AgentLoop 内 `await`；时序从「submit 瞬间」漂移到「turn 开始」

### 4 个静态文件源（按 reminder 块内顺序）
1. `~/.wgenty-code/WGENTY.md`（用户级，整文件）
2. `~/.wgenty-code/rules/*.md`（用户级规则，字母序，整文件）
3. 项目根 `WGENTY.md`（sections join("\n\n") 后整文件等价注入）
4. 项目根 `AGENTS.md`（同上）

### Reminder 块精确骨架
```
<system-reminder>
As you answer the user's questions, you can use the following context:
# wgentyMd
Codebase and user instructions are shown below. Be sure to adhere to
these instructions. IMPORTANT: These instructions OVERRIDE any default
behavior and you MUST follow them exactly as written.

Contents of <绝对路径> (描述):

<内容>

[... 重复 4-N 段 ...]

Contents of hook:<HookEvent>:<idx> (dynamic hook injection):

<动态内容>

      IMPORTANT: this context may or may not be relevant to your tasks.
      You should not respond to this context unless it is highly relevant
      to your task.
</system-reminder>
```

### 数据结构新增
- `prompts::ReminderOutput { to_model: String, to_transcript: Option<String> }`
- `prompts::build_user_turn_reminder(ctx, &injections) -> Option<ReminderOutput>`
- `hooks::InjectedFragment { content, priority, visibility, source_label }`
- `hooks::collect_injections(&[HookOutcome]) -> Vec<InjectedFragment>`
- `PromptContext::project_root: Option<PathBuf>` + `with_project_root(path)`
- `utils::project::read_user_global_instructions() -> Option<(PathBuf, String)>`
- `utils::project::read_user_global_rules() -> Vec<(PathBuf, String)>`

### 硬切（BREAKING）
- 移除 `prompts/mod.rs` Layer 7（AGENTS.md）和 Layer 8（WGENTY.md）的 system message push
- v0.1.0 阶段可接受；无 feature flag；回滚 = git revert

## 关键取舍与风险

### 已锁定 Open Questions
- **O1**: `# claudeMd` → `# wgentyMd`（项目身份 vs Claude 模型先验，反悔成本极低）
- **O2**: UserPromptSubmit fire 移到 AgentLoop 内 await（方案 B），换取 UI 不阻塞
- **O3**: LayerVisibility::Internal 在 reminder builder 输出端分流（方案 A）

### Q4-Q7 推荐默认
- **Q4**: hook 超时（10s）→ warning 日志 + 空 outcomes 继续；不阻塞 turn
- **Q5**: hook source_label 用 `hook:<HookEvent>:<index>` 格式，与文件源对齐
- **Q6**: 用户级 WGENTY.md 整文件注入；项目级 sections join 后等价整文件
- **Q7**: token 超阈值仅警告，不阻塞当前 turn

### 主要 Trade-offs
- T1 — UI 阻塞 vs Hook 时序漂移：选了"时序漂移毫秒级"，通过 spec patch 明确
- T2 — 单一构造器 vs 多调用点：选单一，失去"部分注入"灵活性
- T3 — 整文件 vs 分节注入：选整文件（reminder 定位是"参考手册"）
- T4 — hook 内容不计入 token 预算：避免 dynamic 触发警告，conversation_history 整体上限兜底

### 主要 Risks
- R1 — Thinking 状态延后：通过 timeout=10s + 警告日志 + push_system_message 缓解
- R9 — BREAKING 无迁移路径：CHANGELOG 明确，v0.1.0 阶段可接受
- R10 — transcript 投递晚于 Thinking：记入 KNOWN，不修复

## 测试策略

19 个新增测试（≥6 要求达标）：
- 12 个单测（reminder builder 各分支、collect_injections、Layer 7/8 硬切）
- 7 个集成测（首轮注入、per-turn、文件运行时修改、hook 端到端、Visibility 分流、timeout 降级、subagent 不注入）

12 验收场景全部覆盖（详见 Design Doc §6.3）。

## Spec Patch

**回写目标**：`openspec/changes/system-reminder-channel/specs/hook-lifecycle-complete/spec.md`

**新增段**：在现有 ADDED Requirements 之前追加 MODIFIED Requirements 节，修订原 spec 中 `UserPromptSubmit hook fires on every input submission` 的措辞：

- 改名为 `UserPromptSubmit hook fires before agent turn starts`
- Description 改为：fire 时机从「submit 瞬间 fire-and-forget」改为「turn task 内 await，outcomes 被消费」
- 新增 3 个 Scenarios：
  1. Hook fires inside agent turn task（默认场景）
  2. Hook timeout degrades gracefully（10s 超时 → warning + 空 outcomes）
  3. Hook does not fire on built-in commands（unchanged，保留兼容）

完整 patch 文本见 Design Doc §5.2。

## 下一步

1. ✅ Step 1d 已完成（本文件定稿）
2. ⏭ Step 1e — 主动上下文压缩门（需暂停提示用户）
3. ⏭ Step 2 — 写 Design Doc 到 `docs/superpowers/specs/2026-06-27-system-reminder-channel-design.md` + 回写 hook-lifecycle-complete Spec Patch
4. ⏭ Step 3 — 重新生成 handoff（spec 有变更）+ 跑 `comet-guard design --apply`
