# Brainstorm Summary: esc-interrupt-turn

> 恢复检查点 - brainstorming 已定稿（用户已确认方案 A + 硬编码 i18n）。

## 探索结论（已验证代码路径）

- `cancel_current_turn()` (`turn.rs:328`) 已存在：abort JoinHandle + phase=Idle + suppress_phase_updates + `TurnAborted::Interrupted`。仅 `/clear` 调用。
- 空闲态 ESC 退出程序 (`event_key.rs:451-453`)。
- 上下文面板优先消费 ESC：focus view / completion / permission(=Deny) / question / session popup / status bar，均 early-return。
- `streaming_content` 仅在 `streaming_active` 时渲染 (`chat.rs:80`)；`StreamDone` 将其提交为 Assistant 消息 (`event.rs:153`)。
- `suppress_phase_updates` 仅抑制 phase 更新，不抑制 content 事件 (`event.rs:21`)。
- 测试可用 `App::new(DaemonClient::new("http://localhost:0"), session, settings)` 构造 (`mod.rs:754`)。

## 候选方案

### 方案 A（推荐）：interrupt 包装器 + generation reset + 提交 partial + 移除 ESC-quit
- 新增 `interrupt_running_turn()` 包装 `cancel_current_turn()`：提交 partial streaming 为 Assistant 消息、清 streaming_active、finalize tool placeholder、reset_agent_generation 取消子代理、推送 `⏹ Interrupted by user`。
- ESC 分支置于所有面板之后、scroll 之前，gate `current_turn_handle.is_some()`。
- 移除 ESC-quit。复用现有 abort 机制（cancel_current_turn 已用）。
- **优点**：UX 完整（保留 partial、反馈、取消子代理），复用 proven 机制，改动聚焦 `src/tui/`。
- **缺点**：重复 /clear 的 generation-reset 逻辑片段。

### 方案 B：最小化 - ESC 仅调 cancel_current_turn()
- 不提交 partial、不 reset 子代理、不推送消息。
- **优点**：代码最少。
- **缺点**：partial 内容消失（streaming_active=false 后不渲染）、子代理继续空跑耗 token、无反馈。UX 明显更差。否决。

### 方案 C：协作式取消（CancellationToken）
- 向 AgentLoop 注入 CancellationToken，ESC 触发，循环在 await 点检查退出。
- **优点**：更优雅，无 mid-operation abort。
- **缺点**：需贯穿 AgentLoop + 所有工具执行路径，触及 `src/tui/agent/`、`src/agent/`，范围远超功能本身。与现有 abort 模式不一致。否决（ disproportionate scope）。

## 选定方案：A

## 技术风险

1. **stale 事件重新激活 streaming** [已分析-低危]：abort 丢弃 in-flight future（reqwest stream / tool await），abort 后无新 content 事件产生；已入队事件先于 KeyEvent 处理（FIFO），仅丰富随后提交的 partial。suppress_phase_updates 抑制 phase 翻转。与 /clear 同机制。
   - [候选缓解] 可在 ContentDelta/StreamDone/ToolResult 处理器加 `!suppress_phase_updates` gate，但改变现有行为、范围扩大。当前不采用，保留为后续观察项。
2. **子进程未被硬终止** [已知限制]：exec_command 的子进程独立 tokio::spawn，abort 不 kill。best-effort，记入非目标。
3. **i18n** [已确认-硬编码]：`⏹ Interrupted by user` 字符串。现有同类 system 消息（"Plan mode enabled" 等）为硬编码英文。为与周围代码一致，采用硬编码；不新增 i18n key（用户已确认）。

## 测试策略

- 单元测试 `interrupt_running_turn()`（input.rs 或 turn.rs test mod）：构造 App，spawn dummy 长任务作 `current_turn_handle`，填 `streaming_content`，调用后断言：partial 提交为 Assistant 消息、streaming_active=false、has_running_tool=false、系统消息存在、current_turn_handle=None、phase=Idle、suppress=true。
- 单元测试 ESC 分支：构造 App + dummy handle，发送 `KeyCode::Esc` KeyEvent，断言走中断路径（current_turn_handle 清空、消息出现）。
- 单元测试 idle ESC 不退出：无 handle 时发 Esc，`should_quit` 保持 false。
- 手动验证：streaming/tool/compact 中断、permission ESC 仍 Deny、Ctrl+C 双击退出。

## Spec Patch 评估

delta spec (`specs/tui-turn-interruption/spec.md`) 已含 6 个 Requirement + 验收场景，覆盖中断/保留 partial/反馈/取消子代理/不退出/面板优先级。无需 Spec Patch。

## 用户确认结果

- 方案 A 已确认（vs B/C）。
- i18n：硬编码（用户已确认）。
