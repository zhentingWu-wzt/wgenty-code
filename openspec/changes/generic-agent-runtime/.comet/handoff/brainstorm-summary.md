# Brainstorm Summary

- Change: generic-agent-runtime
- Date: 2026-06-23

## 确认的技术方案

**核心原则**：不新增"工作流"抽象（StateMachine, TransitionGuard, WorkflowEngine, GuardPipeline），只扩展已有的 hooks 系统。Comet = YAML hook 配置 + scripts + SKILL.md。

**Rust Runtime 只扩展 hooks 系统**（从 `src/hooks/` 移入 `src/runtime/hooks/` 并扩展）：
- 新增事件：`SlashCommand`、`StateChanged`
- 新增 action：`inject_context`（带 visibility: internal/visible）、`ask_user`（暂停 agent loop 等待用户响应）
- 新增条件：`when_state`（字符串匹配当前状态）

**数据注入模式（纯数据，无 Engine 对象）**：
- Session 启动时一次性解析 YAML → 产出 Guards(Vec<ToolGuard>)、Layers(Vec<ContextLayer>)、Routes(Vec<CommandRoute>)、StateHandle(Arc<RwLock<str>>) → 注入各组件
- 注入完成后各组件不依赖任何 "workflow" 或 "engine" 对象
- Agent Loop 代码搜不到 workspace、engine、yaml

**Comet workflow.yaml**：声明式 hook 配置，Rust 只是解释它。

## 关键取舍与风险

1. **取舍：放弃泛型 StateMachine**。不建 StateMachine<S>，用 `Arc<RwLock<str>>` + `StateChanged` hook 事件替代。优点是 Rust 零领域知识，缺点是失去编译时状态验证（YAML 中的 state 字符串拼写错误要到运行时才检测到）。缓解：workflow.yaml 加载时校验 state 引用一致性。

2. **取舍：Comet 脚本保持独立**。不做 ScriptRunner（删除 `src/runtime/script.rs`），现有 hook 系统的 `run script` action 已经能执行脚本。Comet 脚本（comet-guard.sh 等）继续作为 hook action 的 command。

3. **取舍：InteractionService 只是 hook action 的底层实现**。`ask_user` 是 hook action，InteractionService trait 是实现细节。TUI、CLI、headless 分别实现，hook 系统不感知平台差异。

4. **风险：SlashCommand hook 事件是新增的**。现有 hooks 只有 tool lifecycle + session lifecycle。SlashCommand 是 user input lifecycle 的扩展。需要确保与现有 UserPromptSubmit hook 不冲突。

## 测试策略

- **单元测试**：hook 扩展（新增事件/action/条件各自的单元测试）、ContextAssembler（priority 排序、visibility 分离）
- **集成测试**：Comet workflow.yaml 加载 → Guard/Layer/Route 注入 → ToolExecutor 正确拦截/放行、ContextAssembler 正确分层、CommandRouter 正确路由
- **回归测试**：现有 hooks 全部通过、现有 slash command（/clear, /help, /plan, /continue, /undo, /init）行为不变
- **手动验证**：`grep -r "comet\|openspec\|phase" src/runtime/` 零结果、内部 prompt 对用户不可见

## Spec Patch

无。设计方向与 open 阶段创建的 delta spec 一致，不需要回写。
