## Context

#0 codegraph-baseline-spike 的根因分析（`scripts/codegraph-bench/root-cause-analysis.md`）确定 Agent 不主动使用 codegraph 的 top 3 根因，并量化了基线（codegraph 调用率 0.05%）。本 change 是后续 3 个改进 change 中风险最低、改动最少的，作为第一个修复打头阵。

现状关键事实：
- codegraph 工具（`codegraph_node`、`codegraph_explore`）已通过 MCP 暴露
- 索引能力已就绪（`.codegraph/index.db`）
- `src/prompts/base.md:117-119` 工具列表中 grep 排第一位、codegraph 完全不在列表
- `src/prompts/base.md:141-153` 「When to use each tool」表的"查找函数定义"推荐 grep
- `src/tools/codegraph/tools.rs:61-63, 175-177` description 是纯功能描述，无场景引导
- `src/tools/codegraph/tools.rs:10-34` lazy-init 错误信息为 `"No codegraph index found. Run 'wgenty-code codegraph index' first."` —— 信息存在但未在 prompt 层规约 Agent 收到该错误时该如何处理

约束：
- 不修改 codegraph 索引引擎、查询逻辑、MCP 层
- 不修改 TUI、不修改 OnceLock 架构
- 不引入新 Cargo 依赖

## Goals / Non-Goals

**Goals:**

- 通过 prompt + tool description + 错误文案三层修复，让 Agent 在合适场景主动选用 codegraph 替代 grep
- 在 14 条标准任务集上，达到分层阈值：强项类 ≥60%、其他类 ≥25% 使用 codegraph
- 不破坏现有非代码导航工作流（grep、file_read、glob 仍正常工作）
- 提供可重跑的回归测试脚本 `bench-agent-replay.sh`，让后续每次 prompt 修改都能自测

**Non-Goals:**

- 不修改 codegraph 业务逻辑（索引、查询、解析） —— 归 #2/#3
- 不增加 TUI 状态显示 —— 避免 src/tui/ 改动
- 不修改 OnceLock 初始化架构 —— 仅文案优化
- 不强制 Agent 100% 用 codegraph —— 保留 grep/file_read/glob 作为兜底
- 不增量索引、不预热索引 —— 这些是性能优化，归 #2

## Decisions

### D1：分三层修复，每层独立验证

**决策**：将修复分为 3 个独立可验证层：
- 层 A — Prompt 层：修改 `src/prompts/base.md`（工具列表 + 对比表）
- 层 B — 工具描述层：修改 `src/tools/codegraph/tools.rs` 的 description
- 层 C — 错误反馈层：优化 lazy-init 的 ToolError 文案

**理由**：
- 三层修复对应根因 1/2/3，独立验证便于归因（哪一层贡献了多少采纳率提升）
- 每层都可单独 commit + 跑 bench-agent-replay.sh 看效果
- 失败可独立回滚

**替代方案**：
- 一次性修改三层 — 拒绝。无法归因，回滚粒度粗
- 仅做层 A — 拒绝。description 不改，Agent 即使知道 codegraph 存在也不知何时用

### D2：代码导航 playbook 放在 base.md 而非新文件

**决策**：「代码导航 playbook」作为新段落加入 `src/prompts/base.md`，不新增独立 prompt 文件。

**理由**：
- base.md 是 Agent 系统 prompt 的核心，新文件需要额外的加载机制
- playbook 内容与「Search」段落 + 「When to use each tool」表逻辑相邻
- 新增独立文件会触发架构变更，超出本 change 边界

**替代方案**：
- 放在 `src/prompts/collaboration.md`（如存在）— 待 build 阶段确认是否有该文件
- 放在新文件 `src/prompts/code-navigation.md` — 拒绝，需要修改加载逻辑

### D3：tool description 用「PREFER FOR ... AVOID WHEN ...」结构

**决策**：description 重写为以下格式：

```
codegraph_node: Look up a Rust symbol by name. Returns definition, signature, references, callers/callees.
PREFER FOR: finding symbol definitions, listing callers/callees, finding references.
AVOID WHEN: searching for text patterns or non-symbol concepts (use grep instead).
```

**理由**：
- 显式列出场景，让 Agent 通过模式匹配做决策（Anthropic / OpenAI 工具调用模型的标准对策）
- AVOID WHEN 提供反向引导，避免 codegraph 被滥用到 grep 适合的场景
- 与 prompt 中「When to use each tool」表互相印证

**替代方案**：
- 仅加 PREFER FOR — 拒绝。无反向引导可能导致 codegraph 被滥用
- 用自然段落 — 拒绝。结构化关键词更易被 Agent 识别

### D4：lazy-init 错误文案优化范围最小化

**决策**：仅修改 `src/tools/codegraph/tools.rs:61-63` 处 ToolError 的 message 字段：
- 原：`"No codegraph index found. Run 'wgenty-code codegraph index' first."`
- 新：`"No codegraph index found at .codegraph/index.db. To enable: run 'wgenty-code codegraph index' in this directory, then retry. Falling back to grep is acceptable for now."`

**理由**：
- 明确告知 Agent：(1) 索引位置；(2) 修复命令；(3) 若不修复，可以 grep 兜底（避免 Agent 因失败而陷入死循环）
- 不改 OnceLock 架构、不改成功反馈
- 减少 build 阶段意外引入回归

**替代方案**：
- 同时改 OnceLock 成功反馈 — 拒绝。增加 src/tools/codegraph/tools.rs 改动行数与风险
- 加 ToolOutput metadata `{"index_ready": true}` — 拒绝。需要修改更多地方且 Agent 可能不读 metadata

### D5：bench-agent-replay.sh 复用现有 bench-agent.sh + 真实 query 调用

**决策**：新增 `scripts/codegraph-bench/bench-agent-replay.sh`，逻辑：
1. 对每条 nav-XXX.yaml，用 `wgenty-code repl` 或 daemon 模式运行 prompt
2. 等待 session 写入 `~/.wgenty-code/sessions/`
3. 调用 `bench-agent.sh --session <new-session>` 解析工具序列
4. 按任务 category 分类聚合，生成分层 JSON 报告

**理由**：
- 复用现有 bench-agent.sh 的 session 解析能力
- 真实 query 而非模拟，确保结果反映真实 Agent 行为
- 输出格式与 #0 报告一致，可直接对比基线

**替代方案**：
- 用 `wgenty-code query --prompt`（已知 bug：query 不创建 session JSON）— 拒绝
- 模拟工具调用 — 拒绝。无法验证真实 Agent 决策

**Open**：repl 模式如何在脚本中自动注入 prompt 并退出，需要 build 阶段试探（可能需要 `expect` / `script` 工具，或临时使用 daemon 模式）

### D6：分层阈值的强项类 vs 其他类划分

**决策**：14 条任务按以下分类对照阈值：

| 任务类别 | nav-IDs | 阈值 |
|----------|---------|------|
| **强项类** | nav-001 (definition_lookup), nav-002 (definition_lookup), nav-003 (reference_lookup), nav-004 (reference_lookup), nav-007 (impl_enumeration), nav-008 (impl_enumeration) | ≥ 60% (4/6 用 codegraph) |
| **其他类** | nav-005, nav-006 (call_chain), nav-009, nav-010 (module_structure), nav-011, nav-012 (cross_module_path), nav-013, nav-014 (复合) | ≥ 25% (2/8 用 codegraph) |

**理由**：
- 强项类是 codegraph_node 直接擅长的场景（定义、引用、impl 枚举），prompt 修改后应高比例切换
- 其他类需要 codegraph_explore 或多步推理，模型选择会更分散，宽松阈值
- call_chain 实际上也是强项，但样本数 2 太少，归入其他类增加余量

## Risks / Trade-offs

| 风险 | 缓解 |
|------|------|
| Prompt 修改影响其他类型任务的工具选择 | S2 验收场景要求"不破坏现有功能"；分层 commit 让每层修改可回滚 |
| 14 条任务集样本量小，统计意义弱 | 阈值已宽松到首轮 prompt 调整即可达；真实使用监控留给后续 change（自动遥测可在 #2 加入） |
| `wgenty-code repl` 自动化注入困难 | D5 备选 daemon 模式；build 阶段先试探，失败则降级为人工跑 14 条任务 |
| Agent 模型行为不稳定 | bench-agent-replay.sh 支持多次运行取均值；阈值定为下限而非平均值 |
| description 修改可能与 OpenSpec spec 不同步 | 列入 modified capabilities，build 阶段同步更新 spec |
| codegraph_explore 推不动（基线为 0） | description 同样加入 PREFER FOR；其他类阈值放宽到 25%，给模型探索空间 |
| 错误文案"falling back to grep is acceptable"可能被滥用 | 文案中明确"for now"暗示这是临时降级；后续 change 可加 hard error |

## Migration Plan

不适用 — 仅 prompt 与 description 调整。无运行时迁移、无回滚需求。
归档时按 OpenSpec delta 语义把 modified capabilities 同步到主 spec。

## Open Questions

以下问题需在 build 阶段 brainstorming 解决：

1. **代码导航 playbook 的最终位置**：base.md 内嵌段落 vs collaboration.md 引用 vs 新文件。倾向 base.md 内嵌（D2），但需 build 阶段读 src/prompts/ 全部确认无更合适位置
2. **playbook 的具体措辞**：codegraph→grep→file_read 的优先级如何用 prompt 自然语言表达，避免与「When to use each tool」表重复
3. **bench-agent-replay.sh 自动化方式**：repl 自动化 vs daemon API vs 人工运行 + 脚本聚合
4. **强项类 nav-005/006 (call_chain) 是否归入强项**：build 阶段先试跑两轮，看 codegraph_explore 是否真能解决 call_chain
5. **错误文案"acceptable to fallback to grep"是否过于宽容**：可能让 Agent 永远不修索引；备选措辞由 brainstorming 决定
