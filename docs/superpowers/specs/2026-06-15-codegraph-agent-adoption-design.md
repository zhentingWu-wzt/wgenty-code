---
comet_change: codegraph-agent-adoption
role: technical-design
canonical_spec: openspec
archived-with: 2026-06-15-codegraph-agent-adoption
status: final
---

# Codegraph Agent Adoption — 技术设计

> 上游 OpenSpec 产物：`openspec/changes/codegraph-agent-adoption/`
> Brainstorming 决策记录：`.comet/handoff/brainstorm-summary.md`
> 基线报告（#0）：`openspec/changes/archive/2026-06-15-codegraph-baseline-spike/`

## 1. 概述

本 change 针对 #0 baseline-spike 根因 top 3（prompt 未提及 codegraph / tool description 缺场景引导 / lazy-init 无反馈），从三个独立层同步修复，每层可独立验证和回滚：

- **层 A（Prompt）**：修改 `src/prompts/base.md` — 加入 codegraph 工具列表 + 更新对比表 + 新增 playbook
- **层 B（Tool Description）**：修改 `src/tools/codegraph/tools.rs` — 重写 description 为 `PREFER FOR ... AVOID WHEN ...`
- **层 C（Error Feedback）**：修改 lazy-init ToolError message — 明确修复命令 + 耗时 + single-task fallback

验收标准：在 14 条标准代码导航任务上，强项类（8 条）≥ 60% 使用 codegraph，其他类（6 条）≥ 25%。

## 2. 架构与修改范围

```
┌─────────────────────────────────────────────────────┐
│  src/prompts/base.md                                 │
│  ┌─────────────────────────────────────────────────┐│
│  │ ## Search  (line 115-121)                       ││
│  │   - **codegraph_node**: Symbol lookup + callers ││ ← 层 A: 新增, 排 grep 之前
│  │   - **codegraph_explore**: Call graph explorer  ││ ← 层 A: 新增
│  │   - **grep**: (原有, 推后)                       ││
│  │   ...                                           ││
│  │ ## Code navigation playbook (新增段落)           ││ ← 层 A: OQ1+OQ2
│  │ ## When to use each tool (line 139-153)          ││
│  │   | Find function definition | codegraph_node  |│ ← 层 A: 更新
│  │   | Find callers            | codegraph_node  |│
│  │   | Find implementations    | codegraph_explore|│
│  └─────────────────────────────────────────────────┘│
└─────────────────────────────────────────────────────┘
┌─────────────────────────────────────────────────────┐
│  src/tools/codegraph/tools.rs                        │
│  ┌─────────────────────────────────────────────────┐│
│  │ codegraph_node.description()                     ││
│  │   "Look up a Rust symbol...                     ││ ← 层 B: 功能首句(保留)
│  │    PREFER FOR: ...                               ││ ← 层 B: 新增
│  │    AVOID WHEN: ..."                              ││ ← 层 B: 新增
│  │ codegraph_explore.description()                  ││
│  │   "Explore code symbols...                      ││ ← 层 B: 功能首句(保留)
│  │    PREFER FOR: ...                               ││ ← 层 B: 新增
│  │    AVOID WHEN: ..."                              ││ ← 层 B: 新增
│  │ get_engine() → ToolError                         ││
│  │   "No codegraph index found at ...              ││ ← 层 C: 重写
│  │    Run ... (typically <5s).                      ││ ← 层 C: 新增耗时提示
│  │    ... as temporary alternative for this task."  ││ ← 层 C: single-task fallback
│  └─────────────────────────────────────────────────┘│
└─────────────────────────────────────────────────────┘
       ↑                                    ↑
┌──────┴────────────────────────────────────┴──────────┐
│  scripts/codegraph-bench/bench-agent-replay.sh        │
│  ┌─────────────────────────────────────────────────┐│
│  │ 1. 启动 wgenty-code daemon                       ││
│  │ 2. 通过 API 触发 agent loop（14 tasks）          ││
│  │ 3. 扫描 sessions/ 找新 session JSON              ││
│  │ 4. 调用 bench-agent.sh 解析工具序列              ││
│  │ 5. 输出 agent-replay.json（分层统计）             ││
│  └─────────────────────────────────────────────────┘│
└─────────────────────────────────────────────────────┘
```

### 修改文件清单

| 文件 | 层 | 改动描述 | 行数估计 |
|------|-----|---------|---------|
| `src/prompts/base.md` | A | Search 段落 + 对比表 + playbook | ~35 行新增 |
| `src/tools/codegraph/tools.rs` | B+C | description + error message | ~15 行修改 |
| `scripts/codegraph-bench/bench-agent-replay.sh` | — | 新增回归脚本 | ~80 行新增 |
| `openspec/changes/codegraph-agent-adoption/specs/*/spec.md` | — | Spec patch（1 处） | ~5 行修改 |

## 3. 关键技术决策

### 3.1 三层独立验证（D1）

每层修改独立 commit，每层跑 `bench-agent-replay.sh` 验证。归因逻辑：
- 层 A 生效 → 层 B 加持 → 层 C 兜底
- 若 3 层全部生效仍不达阈值，说明需要 #2 query-explainability 增强查询能力
- 若仅层 A 就达阈值，仍提交 B+C（防御性：description 和 error 是长期质量保障）

### 3.2 Playbook 措辞（OQ2）

```markdown
### Code navigation playbook

When you need to understand code structure, follow this order:

1. **First, codegraph_node / codegraph_explore** — for any symbol-related question (definitions, callers, references, implementations, module structure). These return precise structured results.
2. **Then grep / lsp** — when the target is text patterns, comments, or non-symbol concepts; or when codegraph returns no results.
3. **Finally file_read** — only after locating relevant files via the above.

If `codegraph_node` returns "No codegraph index found", run `wgenty-code codegraph index` once, or fall back to grep for the current task.
```

### 3.3 Tool Description 格式（D3）

```
codegraph_node:
  [功能] Look up a Rust symbol by name. Returns definition, signature, references, callers/callees.
  [场景] PREFER FOR: finding symbol definitions, listing callers/callees, finding references.
  [边界] AVOID WHEN: searching for text patterns or non-symbol concepts (use grep instead).

codegraph_explore:
  [功能] Explore code symbols and their call relationships. Returns relevant symbols and call paths.
  [场景] PREFER FOR: exploring module structure, browsing call graphs, understanding cross-module relationships.
  [边界] AVOID WHEN: looking up a single known symbol (use codegraph_node) or searching text patterns (use grep).
```

### 3.4 Error Message（OQ5）

```
No codegraph index found at .codegraph/index.db. Run 'wgenty-code codegraph index' in this directory to build the index (typically takes <5s on a Rust project), then retry codegraph_node. If the index command fails or unavailable, you may use grep as a temporary alternative for this single task.
```

### 3.5 bench-agent-replay.sh 数据流（OQ3）

```
for each nav-*.yaml:
  POST daemon:agent/query {prompt: task.prompt}
    → daemon spawns agent loop
    → writes session JSON to ~/.wgenty-code/sessions/<uuid>.json
  find newest session (mtime after API call start)
  bench-agent.sh --session <new> → extract tool_calls[]
  record: {task_id, category, used_codegraph, tool_sequence}

aggregate by category → strong_categories vs other_categories
output agent-replay.json with per-task stats + aggregate comparison
```

**Build 阶段探针**：先确认 daemon 是否暴露 agent loop API。不存在则降级为 repl + expect 方案。

### 3.6 分层阈值（OQ4）

| 类别 | 任务 | 阈值 |
|------|------|------|
| **强项类（8 条）** | definition_lookup ×2, reference_lookup ×2, call_chain ×2, impl_enumeration ×2 | ≥ 60%（≥5/8） |
| **其他类（6 条）** | module_structure ×2, cross_module_path ×2, 复合 ×2 | ≥ 25%（≥2/6） |

## 4. 风险与权衡

| 风险 | 缓解 |
|------|------|
| Daemon agent loop API 不存在 | Build 阶段探针先行；备选 expect / 人工 |
| Prompt 修改影响其他任务类型工具选择 | 层 A 仅修改 Search 段落 + 对比表 + 新增 playbook，不改其他工具描述；S2 验收场景验证不破坏现有功能 |
| Agent 得到 fallback 提示后永远不修索引 | "single task" 措辞限定；playbook 引导先修复 |
| codegraph_explore 始终推不动（基线 0） | description 同样加 PREFER FOR；其他类阈值 25% 给空间 |
| 14 条任务集样本小 | 分层阈值已宽松；#2 可加入遥测 |

## 5. 测试策略

### 5.1 单元测试（cargo）

- 验证 `codegraph_node` description 包含 "PREFER FOR" / "AVOID WHEN"
- 验证 `codegraph_explore` description 包含区别 codegraph_node 的指引
- 验证 lazy-init ToolError message 包含 ".codegraph/index.db" + "wgenty-code codegraph index" + "<5s"
- 验证 base.md 修改不破坏 cargo build

### 5.2 集成测试（bench-agent-replay.sh）

- 在新 prompt + description 下跑 14 条 nav-XXX.yaml
- 验证强项类 ≥ 60% / 其他类 ≥ 25% 使用 codegraph
- 输出 JSON 含每条任务的 tool_sequence + category statistics

### 5.3 回归测试

- 抽 3-5 条非代码导航 session 验证 grep/file_read/glob 仍正常
- 验证 codegraph 索引未建时新 error 出现 + Agent 可 fallback

### 5.4 手动验证（Build 阶段）

- 实际跑一次完整的 agent loop（daemon 或 repl），观察 codegraph 调用行为
- 在 ripgrep 外部仓库上验证脚本不依赖 wgenty-code 自身索引

## 6. Spec Patch

仅 1 处需回写到 `specs/codegraph-lazy-init/spec.md` Scenario "Index absent"：

```diff
- The tool SHALL return a friendly error: "No codegraph index found. Run `wgenty-code codegraph index` first."
+ The tool SHALL return a `ToolError` whose message includes:
+   - The expected index path (`.codegraph/index.db`)
+   - The exact command to fix the issue (`wgenty-code codegraph index`)
+   - The expected cost of the fix (typically <5s on a Rust project)
+   - A fallback hint limiting grep to a temporary alternative for this single task
```

## 7. Migration / 兼容性

不适用 — 仅 prompt 与 description 调整。无运行时迁移、无架构变更、无新依赖。
