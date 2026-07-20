# 长程自主任务能力改进计划

**日期**: 2026-07-20
**状态**: 改进计划草案
**关联**:
- `docs/superpowers/specs/2026-07-19-exec-session-inner-layer-design.md`（内层 L1 设计）
- `openspec/changes/archive/2026-07-20-exec-session-node-state-machine/design.md`（外层 node 状态机）

## 1. 背景

基于对项目长程自主任务能力的系统性分析（涵盖分层架构、node 状态机、verify-gate 与 comet verify 协同、subagent 化等主题），梳理出以下改进点。改进点按优先级分为四档（P0-P3），覆盖五个主题：

- 长程任务能力补全
- 验证体系优化
- 上下文管理优化
- 分层架构优化
- 记忆系统修复

本文档是改进计划的索引，每个改进点可独立成 change 推进。

## 2. 分层定位回顾

```
comet (流程编排, WHAT, 可选插件)
  │
  ▼
ExecutionSession 外层 (HOW, node 级)
  node 契约 + 状态机 + AutoRetry + 回退
  │ 复用
  ▼
ExecutionSession 内层 L1 (turn 级)
  SessionCoordinator + verify-gate
  │ 复用
  ▼
CheckpointStore / sandbox / guardian (机制层)
```

三层职责分离：验收（内层）-> 聚合（外层）-> 方向（comet）。改进点遵循同一分层原则，不破坏现有不变式（`exec_session/` 代码无 "comet" 字符串）。

## 3. P0：必须修（影响可用性）

### P0-1 记忆去重失效修复

**现状**
- global-memory 里 "test memory" 重复上千条，`filter_extracted_memories` 的 importance 阈值未能去重
- compact 时 LLM 产出 memories，只做 importance 过滤，不做语义去重
- project 记忆有 TF-IDF 索引，global 记忆没有任何去重机制，每轮全量注入（软上限 50）

**问题**
- global-memory 已膨胀到影响每轮注入可用性
- 相同内容反复写入，`storage.memory.max_memories` 上限被无效占用

**改进方案**
- compact 写入前做语义去重：对同 scope 同 tags 的记忆做相似度比对（cosine 或编辑距离），高于阈值（建议 0.85）合并
- global 记忆建轻量索引（不需 TF-IDF，前缀/标签分组 + 相似度即可）
- 新增 `memory dedup` 命令支持手动触发去重
- `prune` 命令增强：识别语义重复并合并（当前只按 age/importance 清理）

**收益**
- 解决当前最严重的可用性问题
- global-memory 注入回到有效信息密度

**关联模块**: `src/context/memory/`、`src/agent/runtime/compactor.rs`、`src/cli/memory.rs`

### P0-2 RLM replan 能力

**现状**
- `run_rlm_pipeline` 的 Planner 一次性产出子任务 JSON（最多 8 个），子任务带 `depends_on` 依赖
- Executor 按依赖 level 分层并行执行，子任务失败只在 Aggregator 标 `[ERROR]`，无重试或重新分解
- node 状态机补了 node 级 AutoRetry，但 RLM 任务级 replan 仍是空白

**问题**
- 复杂任务一次分解失败就废，无法动态调整
- 子任务失败后没有"重新分解失败子任务 + 其依赖"的机制
- 长程任务"试错修正"能力在任务编排层缺失

**改进方案**
- Executor 阶段子任务失败时，触发局部 replan（只重新分解失败子任务 + 其下游依赖）
- 加 `max_replan` 限制（默认 1-2 次），防止无限 replan
- 失败子任务的上下文（错误原因 + 已尝试方案）传给 Planner 做增量 replan，不是从头重新分解
- Planner 增量模式输入：原 plan + 失败子任务 id + 失败原因，输出：替换的子任务集合

**收益**
- 长程任务"试错修正"能力补全
- 与 node 级 AutoRetry 形成完整重试体系（node 级 + 任务级）

**关联模块**: `src/tools/meta/rlm/pipeline.rs`、`src/tools/meta/rlm/planner.rs`、`src/tools/meta/rlm/executor.rs`

## 4. P1：应该做（显著提升能力）

### P1-1 comet verify subagent 化

**现状**
- comet verify 在主会话读 proposal.md + design.md + delta spec + Design Doc + 改动 diff + 做代码审查
- 完整验证（7 项）上下文占用 30-50k tokens
- 代码审查（requesting-code-review）在主会话做，有确认偏差（agent 刚写完代码自己审查）

**问题**
- 主会话上下文爆炸，影响后续流程
- 代码审查独立性不足，主会话 agent 带"作者视角"偏差

**改进方案**
- 命令式检查（编译/测试）保留主会话（sandbox 执行不占上下文）
- 语义判断检查开 verification subagent：
  - subagent-A: 代码审查（requesting-code-review），输入 diff + tasks.md，输出审查报告（CRITICAL/IMPORTANT/WARNING 清单）
  - subagent-B: design 一致性 + spec 覆盖率（openspec-verify-change），输入 design.md + delta spec + 改动文件，输出一致性报告
  - subagent-C: tasks.md 完成度 + 改动范围一致性，输入 tasks.md + git diff --stat，输出完成度报告
- 主会话只保留：规模评估、命令式检查、结果汇总、用户决策阻塞点、分支处理、状态更新
- subagent 输出固定 schema（严重程度 + 位置 + 描述 + 建议），主会话汇总

**收益**
- 上下文节省 80%+（主会话只收 3 份摘要报告）
- 代码审查独立性提升（隔离确认偏差）
- 与 build 阶段 subagent-driven-development 模式对称

**关联模块**: `.wgenty-code/skills/comet-verify/SKILL.md`、`src/teams/`（Verification subagent 类型已存在）

### P1-2 Compactor micro-compaction

**现状**
- `compact()` 全量替换 history 为 summary，近期上下文可能被过度压缩
- 没有按 turn 边界的混合压缩（保留近期 + 摘要远期）
- 长会话中近期上下文丢失，agent 可能丢失"刚才做了什么"的信息

**问题**
- 全量替换式压缩在长会话中信息损失大
- 没有利用 node 状态机的结构信息做智能压缩

**改进方案**
- 按 turn 边界做混合压缩：保留最近 N turn 原文 + 远期 turn 摘要
- 结合 node 状态机：verified node 的 turn 可以更激进摘要（已有验证证据），Running/Failed node 的 turn 保留更多细节
- node 边界作为压缩锚点："node-1 完成"作为摘要分界，保留 node 元信息 + 摘要 turn 细节
- 配置项：`storage.compaction.recent_turns_kept`（默认 5）

**收益**
- 近期上下文保留，agent 不丢失"刚才做了什么"
- node 状态机与 compactor 协同：验证边界也是压缩边界

**关联模块**: `src/agent/runtime/compactor.rs`、`src/exec_session/node_runtime.rs`、`src/config/agent.rs`

### P1-3 跨会话 resume（#2 change 落地）

**现状**
- 内层 `current_turn` 游标"内层写外层读"但内层不读（L1 重启游标丢）
- 两层（内层 + 外层 node 状态机）都不支持跨进程 resume
- 外层 design.md 明确"跨会话 resume（#2 change，不在本次）"

**问题**
- 长程任务中断后无法恢复，必须从头开始
- node 状态机的 verified node 没有被用作 resume 安全锚点

**改进方案**
- 启动时读 session.json 的 `current_turn` + `node_states`，重建 turn 链和 node 链
- 恢复到最近 verified node（安全点），丢弃 Running/Failed 的未验证工作
- verified node 是 resume 的安全锚点：回退到最近 verified node 的 start_turn，清除之后的 turn + node
- 恢复后告知 agent："session resumed at node-N (verified), 丢弃了 node-N+1 (failed) 的工作"
- 崩溃一致性：session.json 原子写已保证，resume 读到的一定是完整状态

**收益**
- 长程任务"可中断可恢复"闭环
- node 状态机的 verified node 成为 resume 锚点，设计协同

**关联模块**: `src/exec_session/coordinator.rs`、`src/exec_session/node_runtime.rs`、`src/exec_session/session.rs`

## 5. P2：可以做（架构增强）

### P2-1 node 支持层次化（有条件嵌套）

**现状**
- node 是线性链，一次只有一个 current node
- 不能表达"node 内套 node"的层次化任务结构
- RLM 并行子任务 + node 串行验证的协同关系未定义

**改进方案**
- 不做完全嵌套（YAGNI），但支持"node 挂子 node"两层
- RLM 的子任务可以各自跑一个 sub-node，父 node 聚合子 node 结果
- 严格限制深度（max 2 层），防止状态机复杂度爆炸
- 父 node 的 verify_commands 聚合子 node 的验证结果

**收益**
- 解决 RLM 并行子任务 + node 串行验证的协同空白
- 支持层次化任务结构

**关联模块**: `src/exec_session/node.rs`、`src/tools/meta/rlm/executor.rs`

**风险**: 状态机复杂度增加，需要严格的深度限制和测试覆盖

### P2-2 统一并行编排路径

**现状**
- RLM Executor 直接 `tokio::spawn` 子代理 loop，没走 `AgentCoordinator` 的 permit 模型
- 两套并行编排路径：RLM 自管并行 + AgentCoordinator permit 模型
- RLM 可能绕过全局并发限制（max_concurrent=5）

**改进方案**
- RLM Executor 通过 AgentCoordinator 申请 permit，统一并发限制
- RLM 的 8 个子任务 + 其他子代理共享同一 permit 池
- 失败时复用结构性 fallback（permit 不足时 inline 自执行）

**收益**
- 避免资源爆炸（RLM 8 子任务 + 其他子代理同时跑）
- 统一并发治理

**关联模块**: `src/tools/meta/rlm/executor.rs`、`src/teams/coordinator.rs`

### P2-3 verify 契约充分性检查

**现状**
- verify-gate 信任 agent 声明的 `verify_commands` 和 `expected_files`
- 弱验证（只 lint 不测功能、只测 happy path）会假阳性
- `expected_files` 与 tasks.md 没有机制化对应，靠 agent 自觉

**改进方案**
- `begin_node` 时对 verify_commands 做静态检查：
  - 是否包含测试命令（检测 `test` 关键词）
  - 是否只有 lint 没有功能测试
  - 空列表告警（无验证命令）
- `expected_files` 与 tasks.md 机制化对应：
  - begin_node 可选接收 `task_ids`，runtime 从 tasks.md 提取相关文件清单交叉校验
  - 不一致时返回警告（不阻断，agent 可确认或修正）
- comet build 阶段 skill 指令强制要求 verify_commands 覆盖 design.md 的验收标准

**收益**
- 兜底"信任 agent 声明"模式，不改信任模型但加静态校验
- 减少假阳性 verified

**关联模块**: `src/exec_session/node.rs`、`src/tools/meta/begin_node.rs`、`src/tasks/`

## 6. P3：可以做（代码质量/解耦）

### P3-1 NodeRuntime 与 SessionCoordinator 解耦

**现状**
- NodeRuntime 和 SessionCoordinator 共享同一个 `Arc<RwLock<SessionCoordinator>>`
- 不是纯 trait 解耦，有锁竞争风险
- NodeRuntime 难以独立测试

**改进方案**
- 定义 `NodeCoordinatorPort` trait，NodeRuntime 依赖 trait 而非具体类型
- 测试时可注入 mock，不依赖真实 SessionCoordinator

**收益**
- 可测试性提升
- 锁竞争风险降低

**关联模块**: `src/exec_session/node_runtime.rs`、`src/exec_session/coordinator.rs`

### P3-2 LoopHooks trait 化

**现状**
- `LoopHooks` 是具体 struct，扩展靠加字段
- 新增 hook 破坏现有实现

**改进方案**
- 改成 trait + 默认实现（NoHooks 默认 no-op）
- 新增 hook 不破坏现有实现

**收益**
- 扩展性提升

**关联模块**: `src/exec_session/hooks.rs`

### P3-3 verify-fail 调试 subagent 化

**现状**
- verify_node 失败后，agent 在主会话自修正
- 失败日志和调试推理占主会话上下文

**改进方案**
- verify 失败 -> 主会话派发 debugging subagent（加载 systematic-debugging skill）
- subagent 定位根因 + 提出修复方案摘要
- 主会话只收摘要，决定是否采纳 + 执行修复
- 与 P1-1 comet verify subagent 化对称：build 阶段 verify-fail 调试也隔离上下文

**收益**
- build 阶段上下文节省
- 调试过程隔离，主会话聚焦决策

**关联模块**: `src/exec_session/node_runtime.rs`、`src/teams/`、systematic-debugging skill

## 7. 改进路线图

按依赖关系和收益排序：

```
第一阶段（修 bug + 补核心缺口）
  ├── P0-1 记忆去重修复        ← 最紧急，global-memory 已膨胀
  ├── P0-2 RLM replan 能力     ← 长程任务核心缺口
  └── P1-3 跨会话 resume        ← #2 change 落地，依赖 node 状态机

第二阶段（验证体系优化）
  ├── P1-1 comet verify subagent化  ← 上下文节省 + 审查独立性
  ├── P2-3 verify 契约充分性检查    ← 兜底防编造
  └── P3-3 verify-fail 调试subagent化

第三阶段（上下文管理优化）
  ├── P1-2 micro-compaction    ← 配合 node 边界做混合压缩
  └── P2-1 node 层次化         ← 解决 RLM + node 协同

第四阶段（架构打磨）
  ├── P2-2 统一并行编排路径
  ├── P3-1 NodeRuntime trait 解耦
  └── P3-2 LoopHooks trait 化
```

## 8. 三个最值得做的

如果资源有限只能做三件事：

**1. P0-1 修记忆去重**
- 当前 global-memory 严重影响可用性，每轮注入上千条重复 "test memory"
- 纯 bug，投入小收益大
- 不修的话其他改进都被记忆污染拖累

**2. P0-2 RLM replan**
- 长程自主能力的核心缺口
- node 状态机补了 node 级 AutoRetry，但任务级 replan 还是空白
- 没有它，RLM 一次分解失败就废，无法应对复杂任务的动态调整

**3. P1-1 comet verify subagent 化**
- 上下文节省 80%+，代码审查独立性提升
- 与现有 subagent-driven-development 模式对称
- 技术现成（Verification subagent 类型 + AgentCoordinator 已存在）

这三个做完，项目的长程自主任务能力会有质的提升：记忆可用、任务可 replan、验证不爆上下文。

## 9. 不变量约束

所有改进必须延续以下设计不变式：

- `src/exec_session/` 代码无 "comet" 字符串（除 `SessionSource::Comet` 枚举变体与注释）
- 内层不重做 CheckpointStore 机制（复用现有）
- 外层不重做内层机制（复用 VerifyGate / SessionCoordinator::rollback_to）
- verify-gate 失败不自动回退（保留错误状态，agent 显式 rollback）
- 契约由 agent 声明，runtime 亲自执行 verify（防编造）
- 崩溃一致性：session.json 原子写（tmp+rename）

## 10. 验收标准

每个改进点作为独立 change 推进时，需满足：

- [ ] 不破坏现有不变式（§9）
- [ ] 不影响内层 L1 现有行为（CheckpointStore / turn 级 verify_and_complete / undo 不变）
- [ ] 配置项有合理默认值，向后兼容
- [ ] 非 git 项目降级路径明确
- [ ] 相关测试覆盖（含崩溃恢复场景）
- [ ] 更新 WGENTY.md 架构文档（如涉及新模块/配置）
