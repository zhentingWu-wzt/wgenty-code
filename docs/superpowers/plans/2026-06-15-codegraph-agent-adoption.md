---
change: codegraph-agent-adoption
design-doc: docs/superpowers/specs/2026-06-15-codegraph-agent-adoption-design.md
base-ref: c71014547af3d4997f421ac62fc0f22a58e7ddb8
archived-with: 2026-06-15-codegraph-agent-adoption
---

# Codegraph Agent Adoption 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement.

**Goal:** 通过三层修复（prompt/tool-desc/error）让 Agent 在代码导航任务中主动使用 codegraph，达到强项类≥60%、其他类≥25%。

**Architecture:** 修改 3 个现有文件 + 新增 1 个回归脚本。层 A Prompt → 层 B Description → 层 C Error，逐层 commit 并可独立回滚。

**Tech Stack:** Rust (cargo test) + Bash (bench-agent-replay.sh)

archived-with: 2026-06-15-codegraph-agent-adoption
---

## Phase 0: 探针 — 确认 Daemon API

### Task 0.1: Daemon agent loop API 探针

- [x] **Step 1: 检查 daemon API 端点**

```bash
cd /Users/wuzhenting/workspace/project/wgenty-code
cargo build --release 2>&1 | tail -3
# 启动 daemon 并查看 API 路由
./target/release/wgenty-code daemon --help 2>&1
# 或 grep 源码找 agent loop 端点
grep -rn 'agent.*loop\|agent.*query\|AgentRequest' src/ | head -10
grep -rn 'api/v1' src/ | head -10
```

观察 daemon 是否暴露 `/api/v1/agent/query` 或类似端点。

- [x] **Step 2: 确定 bench-agent-replay.sh 技术路径**

根据探针结果决定：
- 如果 daemon 有 agent loop API → 方案 A（daemon）
- 如果 daemon 无 agent loop API → 方案 B（repl + expect），或降级方案 C（人工）

- [x] **Step 3: Commit 探针结论**

```bash
git commit -m "probe: daemon agent loop API availability

Determines bench-agent-replay.sh implementation path:
A: daemon API / B: repl+expect / C: manual (Co-Authored-By: Claude <noreply@anthropic.com>)"
```

archived-with: 2026-06-15-codegraph-agent-adoption
---

## Phase 1: 层 A — Prompt 修改（base.md）

### Task 1.1: 修改「Search」段落 + 「When to use each tool」表

- [x] **Step 1: 在 Search 段落插入 codegraph 工具**

修改 `src/prompts/base.md` line ~117：在 `grep` 之前插入 codegraph_node 和 codegraph_explore 描述。

- [x] **Step 2: 更新「When to use each tool」表**

逐条更新 line 143 附近的对比表：
- "Find where a function is defined" → `codegraph_node`（保留 grep 作为 fallback 说明）
- 新增 "Find callers of a function" → `codegraph_node`
- 新增 "Find implementations of a trait" → `codegraph_explore`
- 新增 "Understand module structure" → `codegraph_explore`

- [x] **Step 3: 新增「Code navigation playbook」段落**

在 Search 段落之后插入 OQ2 确认的决策树式 playbook（codegraph → grep → file_read）。

- [x] **Step 4: 验证 cargo build 成功**

```bash
cargo build 2>&1 | tail -5
```

- [x] **Step 5: Commit**

```bash
git add src/prompts/base.md
git commit -m "feat(prompts): prioritize codegraph over grep for code navigation

Layer A: Add codegraph_node/codegraph_explore to Search section,
update When-to-use table, add Code navigation playbook.

Co-Authored-By: Claude <noreply@anthropic.com>"
```

archived-with: 2026-06-15-codegraph-agent-adoption
---

## Phase 2: 层 B — Tool Description 修改（tools.rs）

### Task 2.1: 重写 codegraph_node description

修改 `src/tools/codegraph/tools.rs` `CodegraphNodeTool::description()`：
- 首句保持功能描述
- 新增 `PREFER FOR: ...`
- 新增 `AVOID WHEN: ... use grep instead`

### Task 2.2: 重写 codegraph_explore description

修改 `CodegraphExploreTool::description()`：
- 首句保持功能描述
- 新增 `PREFER FOR: exploring module structure, browsing call graphs...`
- 新增 `AVOID WHEN: looking up a single known symbol (use codegraph_node) or searching text patterns (use grep)`

### Task 2.3: 验证 + Commit

```bash
cargo test -p wgenty_code -- codegraph 2>&1 | tail -10
cargo build 2>&1 | tail -5
git add src/tools/codegraph/tools.rs
git commit -m "feat(codegraph): add scenario-based PREFER FOR/AVOID WHEN to tool descriptions

Layer B: Rewrite codegraph_node and codegraph_explore descriptions
to guide Agent toward correct tool selection.

Co-Authored-By: Claude <noreply@anthropic.com>"
```

archived-with: 2026-06-15-codegraph-agent-adoption
---

## Phase 3: 层 C — Error Message 修改（tools.rs）

### Task 3.1: 更新 lazy-init ToolError message

修改 `src/tools/codegraph/tools.rs:get_engine()` 的错误信息为 OQ5 确认的文案：
```
No codegraph index found at .codegraph/index.db. Run 'wgenty-code codegraph index' in this directory to build the index (typically takes <5s on a Rust project), then retry codegraph_node. If the index command fails or unavailable, you may use grep as a temporary alternative for this single task.
```

### Task 3.2: 验证 + Commit

```bash
cargo build 2>&1 | tail -5
git add src/tools/codegraph/tools.rs
git commit -m "feat(codegraph): improve lazy-init error message

Layer C: Add fix cost estimate (<5s), exact index path, and
single-task fallback scope to prevent permanent fallback.

Co-Authored-By: Claude <noreply@anthropic.com>"
```

archived-with: 2026-06-15-codegraph-agent-adoption
---

## Phase 4: bench-agent-replay.sh 评测脚本

### Task 4.1: 实现脚本

新建 `scripts/codegraph-bench/bench-agent-replay.sh`：
- 按 Phase 0 确定的路径（daemon / expect / 人工）
- 对每条 nav-*.yaml 运行 agent loop
- 捕获新 session JSON
- 调用 bench-agent.sh 解析工具序列

### Task 4.2: 输出分层 JSON 报告

输出到 `results/<ts>/agent-replay.json`：
```json
{
  "per_task": [{"task_id": "nav-001", "used_codegraph": true, ...}, ...],
  "aggregate": {
    "strong_categories": {
      "total": 8, "codegraph_count": 6, "rate": 0.75
    },
    "other_categories": {
      "total": 6, "codegraph_count": 2, "rate": 0.33
    }
  }
}
```

### Task 4.3: Commit

```bash
chmod +x scripts/codegraph-bench/bench-agent-replay.sh
git add scripts/codegraph-bench/bench-agent-replay.sh
git commit -m "feat: bench-agent-replay.sh — regression test for codegraph adoption

Replays 14 nav-*.yaml tasks and measures codegraph usage rate
with layered threshold (strong ≥60%, other ≥25%).

Co-Authored-By: Claude <noreply@anthropic.com>"
```

archived-with: 2026-06-15-codegraph-agent-adoption
---

## Phase 5: 验收 + 依赖 benchmark

### Task 5.1: 跑回归测试

在新 prompt + description 下跑 benchmark-agent-replay.sh。

### Task 5.2: 验证分层阈值

- 强项类（nav-001/002/003/004/005/006/007/008）≥ 5/8 用 codegraph
- 其他类（nav-009/010/011/012/013/014）≥ 2/6 用 codegraph

### Task 5.3: 验证不破坏现有功能

- 抽 3-5 条非代码导航 session 验证 grep/file_read 仍正常
- 验证 codegraph 索引未建时新 error 出现 + fallback 到 grep

### Task 5.4: 运行完整 cargo build + test

```bash
cargo build && cargo test -- --skip test_skill_parameter_parsing 2>&1 | tail -10
```

archived-with: 2026-06-15-codegraph-agent-adoption
---

## 执行约定

1. 三层修复逐层 commit（A→B→C），每层独立可回滚
2. bench-agent-replay.sh 的技术路径由 Phase 0 探针决定
3. 遇失败 → 加载 systematic-debugging 技能
4. 每个 task 完成后立即勾选 tasks.md + commit
