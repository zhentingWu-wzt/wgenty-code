# Brainstorm Summary

- Change: codegraph-agent-adoption
- Date: 2026-06-15
- 状态：5 个 OQ 全部确认 ✅

## 已确认决策

### OQ1 — 代码导航 playbook 位置：A 内嵌 base.md「Search」段落之后

- 不新增 prompt 文件（D2 已锁定）
- base.md 是核心系统 prompt，所有 session 自动加载
- 与 Search 工具列表 + When to use 表逻辑相邻，最自然
- base.md 增量约 1KB，可接受

### OQ2 — playbook 措辞：A 决策树式（短小、规则化）

```markdown
### Code navigation playbook

When you need to understand code structure, follow this order:

1. **First, codegraph_node / codegraph_explore** — for any symbol-related question (definitions, callers, references, implementations, module structure, call graphs). These return precise structured results.
2. **Then grep / lsp** — when the target is text patterns, comments, or non-symbol concepts; or when codegraph returns no results.
3. **Finally file_read** — only after locating relevant files via the above. Reading whole files without first locating symbols wastes context.

If `codegraph_node` returns "No codegraph index found", run `wgenty-code codegraph index` once, or fall back to grep for the current task.
```

- 三层优先级与 OQ5 错误文案呼应（错误时回退 grep "for the current task"）
- 不与「When to use each tool」表重复（playbook 是流程；表是工具索引）

### OQ3 — bench-agent-replay.sh 自动化方式：A daemon 模式

**核心方案**：
- 启动 `wgenty-code daemon`（参考 bench-query.sh 已有的 daemon 客户端代码）
- 通过 daemon API 触发 agent loop（具体 API 端点 build 阶段确认）
- 等待 API 完成，扫描 `~/.wgenty-code/sessions/` 找出新生成的 session（按 mtime + before/after diff）
- 调用 `bench-agent.sh --session <new>` 解析工具序列

**待 build 阶段确认**：
- daemon 是否已暴露 agent loop 入口（不止 `/api/v1/tools/execute`，需要类似 `/api/v1/agent/query`）
- 如不存在该 API：备选 B（repl + expect）；最终降级 C（人工跑）

**降级链**：A daemon API → B expect → C 人工

### OQ4 — call_chain 升级到强项类：B（8 强项 + 6 其他）

| 类别 | nav-IDs | 阈值 |
|------|---------|------|
| **强项类（8 条）** | 001/002 (definition_lookup), 003/004 (reference_lookup), 005/006 (call_chain), 007/008 (impl_enumeration) | ≥ 60%（≥5/8 用 codegraph） |
| **其他类（6 条）** | 009/010 (module_structure), 011/012 (cross_module_path), 013/014 (复合) | ≥ 25%（≥2/6 用 codegraph） |

理由：call_chain 是 codegraph 比 grep 显著优秀的场景，归其他类反而错失激励信号。

**Spec patch 候选**：design.md D6 的分层表更新；OpenSpec delta spec 不需要改阈值（spec 写验收方法不写数字）。

### OQ5 — 错误文案：B 强制建议先修索引 + 受限 fallback

```
No codegraph index found at .codegraph/index.db. Run 'wgenty-code codegraph index' in this directory to build the index (typically takes <5s on a Rust project), then retry codegraph_node. If the index command fails or unavailable, you may use grep as a temporary alternative for this single task.
```

要点：
- 明确"修复成本低（<5s）"降低 Agent 借口
- fallback 受限于"this single task"，避免长期回退
- 与 OQ2 playbook 的"fall back to grep for the current task"措辞对齐

**Spec patch 候选**：`specs/codegraph-lazy-init/spec.md` Scenario "Index absent" 中的"acceptable temporary alternatives"措辞需要更新为"may be used as a temporary alternative for this single task"，与 OQ5 对齐。

## 关键取舍与风险

| 风险 | 缓解 |
|------|------|
| daemon agent loop API 可能不存在 | build 阶段先探针，确认后再实现；备选 expect / 人工 |
| call_chain 任务模型可能"偷懒"用 grep | playbook 的决策树明确指引；阈值定为下限而非平均 |
| Agent 收到 fallback 提示后永远不修索引 | "single task" 措辞限定；prompt 层 playbook 引导先修复 |
| 强项类 8 条 ≥60% 比 6 条 ≥60% 更严格 | 可在 verify 阶段调整（spec R5 留 ±20% 空间） |
| Daemon API 启动慢 / 14 任务串行耗时 | 14 条预计 <30 秒；可并行（API 支持时） |

## 测试策略

### 单元层
- prompt 修改无单元测试需求（纯文本）
- tool description 修改通过 cargo test 验证 description() 输出符合 spec scenario "Tool description includes scenario guidance"
- 错误文案修改通过 cargo test 验证 ToolError message 包含规定字段

### 集成层
- bench-agent-replay.sh 在 wgenty-code 自身仓库跑一次，验证 14 任务全跑通
- 验证强项类 ≥60% / 其他类 ≥25% 使用 codegraph

### 回归层
- 抽 3-5 条非代码导航 session 验证不破坏现有功能（grep/file_read 仍合理使用）

## Spec Patch（待回写到 specs）

1. **`specs/codegraph-lazy-init/spec.md`** Scenario "Index absent"：
   - 把"acceptable temporary alternatives"改为"may be used as a temporary alternative for this single task"
   - 增加"包含修复命令耗时提示（typically takes <5s on a Rust project）"

2. **`specs/symbol-query/spec.md`** Scenario "Tool description includes scenario guidance"：
   - 已含 PREFER FOR / AVOID WHEN 要求，无需更改

3. **`specs/call-graph/spec.md`** Scenario "Tool description includes scenario guidance"：
   - 已含 PREFER FOR / AVOID WHEN 要求，无需更改

4. **新增 spec / capability**：本次 change 决定**不**新增 capability。playbook 是对既有 prompt 的修改，归 prompt 层；不需要单独的 capability spec。

## 待 build 阶段试探的技术风险

1. daemon agent loop API 的可用性（OQ3 的关键 unknown）
2. cargo test 验证 description() 输出的现有测试 fixture 是否存在；不存在则需新增
3. base.md 修改后会不会触发其他测试（prompt snapshot test）
