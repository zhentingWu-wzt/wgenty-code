---
comet_change: codegraph-baseline-spike
role: technical-design
canonical_spec: openspec
archived-with: 2026-06-15-codegraph-baseline-spike
status: final
---

# Codegraph Baseline Spike — 技术设计

> 上游 OpenSpec 产物：`openspec/changes/codegraph-baseline-spike/`
> Brainstorming 决策记录：`.comet/handoff/brainstorm-summary.md`
> 本设计仅覆盖 HOW（实现方案、风险、测试），目标/范围/非目标以 OpenSpec proposal + delta spec 为准。

## 1. 测量套件目录结构

测量套件全部位于 `scripts/codegraph-bench/`，与业务源码、测试代码、capability spec 严格隔离。目录约定：

```
scripts/codegraph-bench/
├── README.md                       # 套件用途、入口、运行方法、约束
├── run-all.sh                      # 入口：解析 --target / --output / --repeats，调度所有子脚本
├── lib/
│   ├── env-fingerprint.sh          # 采集 OS/CPU/版本/commit hash → env.json
│   ├── json-helpers.sh             # 统一 JSON 输出格式（中位数、p95 计算用 awk/jq）
│   └── timing.sh                   # 计时封装（系统 time + wall-clock）
├── bench-perf.sh                   # 性能基线：全量/增量索引、查询延迟、index.db 体积
├── bench-coverage.sh               # 覆盖率基线：文件/符号/关系/解析失败
├── bench-agent.sh                  # Agent 使用率基线：B 路径（CLI 回放）
├── bench-transcript.sh             # Agent 使用率基线：A 路径（历史 session 静态分析）
├── gen-report.sh                   # 拼装 results/<ts>/*.json → Markdown 报告
├── agent-tasks/                    # 标准代码导航任务集（≥12 条 YAML 文件）
│   ├── README.md                   # 任务文件格式说明
│   ├── nav-001-trait-def.yaml
│   ├── nav-002-call-chain.yaml
│   └── ...
├── query-fixtures/                 # bench-perf.sh 用的 ≥20 条查询 fixture
│   └── codegraph-queries.txt
└── results/                        # 运行产物（.gitignore，仅保留 .gitkeep + README）
    └── <timestamp>/                # 每次运行一个独立子目录
        ├── env.json
        ├── perf.json
        ├── coverage.json
        ├── agent.json
        └── transcript-analysis.json
```

**关键约束：**
- `results/` 内容默认 `.gitignore`，避免噪声；保留 `.gitkeep` 和 README
- 报告写入 `docs/superpowers/specs/<YYYY-MM-DD>-codegraph-baseline-report.md`，文件名日期为生成日
- 所有脚本通过 `--target <repo-path>` 显式参数化，不硬编码 wgenty-code 自身路径

## 2. 数据流

```
┌──────────────────┐
│  run-all.sh      │  解析 --target / --output / --repeats
└────────┬─────────┘
         │
         ▼
┌────────────────────────────────────────────────────────────┐
│  env-fingerprint.sh   → results/<ts>/env.json              │
└────────────────────────────────────────────────────────────┘
         │
         ▼
┌────────────────────────────────────────────────────────────┐
│  bench-perf.sh        → results/<ts>/perf.json             │
│    • 全量索引 ≥5 次（每次清空 .codegraph/）                │
│    • 增量索引 1/10/100 文件，各 ≥3 次                      │
│    • index.db 字节数 / 项目 .rs 总字节数                   │
│    • codegraph_node / codegraph_explore 各 ≥20 次查询      │
└────────────────────────────────────────────────────────────┘
         │
         ▼
┌────────────────────────────────────────────────────────────┐
│  bench-coverage.sh    → results/<ts>/coverage.json         │
│    • find 统计 .rs 总文件 vs sqlite3 已索引文件            │
│    • SymbolKind / RelKind 分组计数                         │
│    • parse 失败文件 + 错误归类 top 3                       │
└────────────────────────────────────────────────────────────┘
         │
         ▼
┌────────────────────────────────────────────────────────────┐
│  bench-agent.sh       → results/<ts>/agent.json            │
│    • 对每条 agent-tasks/*.yaml：                           │
│        wgenty-code query --prompt "$PROMPT" --no-interactive│
│      → 解析输出 + 解析对应 session JSON 提取工具序列       │
│    • 每条记录: tool_sequence, used_codegraph,              │
│       fallback_path, success                               │
└────────────────────────────────────────────────────────────┘
         │
         ▼
┌────────────────────────────────────────────────────────────┐
│  bench-transcript.sh  → results/<ts>/transcript-analysis.json│
│    • 扫 ~/.wgenty-code/sessions/*.json                     │
│    • 按工具调用过滤代码导航类 session                      │
│    • 输出 codegraph 调用占比、grep/find/Read 占比          │
└────────────────────────────────────────────────────────────┘
         │
         ▼
┌────────────────────────────────────────────────────────────┐
│  gen-report.sh        → docs/superpowers/specs/             │
│                          <YYYY-MM-DD>-codegraph-           │
│                          baseline-report.md                │
│    • jq 拼装 4 份 JSON + env.json                          │
│    • 报告章节：环境指纹 → 性能 → 覆盖率 → Agent 使用率     │
│       → 根因 → 后续 change 对照表                          │
│    • 6 列对照表（指标/基线/目标/行业对标/验证/归属）       │
└────────────────────────────────────────────────────────────┘
```

每个子脚本均可独立运行，便于增量调试。`run-all.sh` 失败一个子脚本不阻塞后续子脚本运行（用退出码区分），但 `gen-report.sh` 检查所有必需 JSON 存在才生成报告。

## 3. 关键技术决策

### 3.1 Agent 驱动路径：B 主 + A 辅

**B 路径（核心）**：CLI 回放
- 入口：`wgenty-code query --prompt "..."`（已存在于 `src/cli/mod.rs:64-68`），配合全局 `--no-interactive` flag（`src/cli/mod.rs:38-39`）
- 工具调用序列来源：query 输出 + 对应 session JSON（`~/.wgenty-code/sessions/<uuid>.json`）
- 每条任务跑一次（必要时人工跑两次取众数，build 阶段试跑 2-3 条决定）
- 产物：`results/<ts>/agent.json`，每条任务记录工具序列、是否使用 codegraph、是否回退、最终是否完成

**A 路径（辅助）**：历史 session 静态分析
- 入口：扫 `~/.wgenty-code/sessions/*.json`（仓库当前已有 71 个历史 session）
- 用途：作为根因 top 3 的可追溯证据来源；不参与 B 路径主指标计算
- 产物：`results/<ts>/transcript-analysis.json`

**舍弃 C 路径（MCP 客户端模拟）**：绕过 agent 决策行为，无法回答「agent 是否选择用 codegraph」这一核心问题。

### 3.2 标准任务集：工程师手写 ≥12 条 / 6 类

任务集结构（每个任务一个 YAML 文件）：

```yaml
task_id: nav-001
category: definition_lookup
prompt: "Tool trait 定义在哪个文件，长什么样？"
expected_answer_anchor:
  file: src/tools/mod.rs
  contains: "trait Tool"
```

6 类各 ≥2 条：
- `definition_lookup` — 定义查找
- `reference_lookup` — 引用查找
- `call_chain` — 调用链探索
- `impl_enumeration` — 实现枚举
- `module_structure` — 模块结构
- `cross_module_path` — 跨模块路径

中英各 6 条以避免语言偏置（build 阶段可调整）。

### 3.3 稳定性窗口：分层阈值

按指标本征波动分三层（写入脚本注释 + 报告表格）：

| 层级       | 阈值   | 适用指标                                                   |
|------------|--------|------------------------------------------------------------|
| 恒定       | ±1%    | 索引体积、文件数、SymbolKind 计数、RelKind 计数            |
| 中等波动   | ±20%   | 全量索引耗时中位数、查询延迟中位数、agent codegraph 调用率 |
| 高波动     | ±50%   | 增量索引耗时、单次查询延迟、agent token 数                 |

超出阈值时 `run-all.sh --repeats 2` 输出告警；不阻塞流程。

### 3.4 建议目标值制定规则：b 主 + a 抽样 + c 补充

- **主规则（b）基线乘系数**：所有指标都给基于基线的目标值
  - agent codegraph 调用率：≥ 基线 + 30 个百分点
  - codegraph_node p95 延迟：≤ 基线 × 1.5（增加可解释性后允许变慢）
  - 全量索引耗时：≤ 基线 × 1.5（多语言成本可控）
  - 多语言覆盖率：≥ 70%（Java/Python 样例项目）
- **a 抽样**：仅对 2-3 个关键指标（索引耗时、查询延迟）做行业对标作为参考列（rust-analyzer / sourcegraph 公开 benchmark），不强制
- **c 补充**：每个目标值标注归属 change，给后续 change 留 1 次「在 design 阶段调整该目标值（不能放宽超过 ±20%）」的口子

报告 6 列对照表：`指标 | 基线 | 建议目标 | 行业对标（参考） | 验证方法 | 归属 change`。

### 3.5 外部仓库：ripgrep 必跑 + tokio 可选

- **ripgrep（必）**：~30K 行 Rust，社区 baseline，下载快，无 build.rs 重型操作
- **tokio（可选）**：~100K 行，async/macro 重度使用，规模压力测试；跑不通时记录原因，不阻塞 spike 完成
- **舍弃 tree-sitter 仓库**：含大量 C 代码会让覆盖率指标失真

agent 测量（B/A 路径）只在 wgenty-code 自身仓库跑，因为它依赖 wgenty-code 的 session 路径和工具行为。

## 4. 风险与权衡

| 风险/权衡 | 缓解 |
|-----------|------|
| B 路径每条任务消耗 API token，12 条成本未知 | build 阶段先跑 2-3 条试跑，估算成本后再决定是否跑两遍取众数 |
| A 路径 session JSON schema 可能变 | build 阶段先读一个 session 文件确认 schema；解析脚本对缺失字段宽容（warn 不 fail） |
| `wgenty-code query` 输出可能不直接含工具调用序列 | 通过解析对应 session JSON 兜底；如两者都不可得，B 路径降级为「成功率」单一指标，并在报告中说明限制 |
| 任务集仅 ≥12 条，代表性不足 | 报告显式标注「任务集规模 N 条，外推需谨慎」；后续 change 可追加 |
| shell `time` 毫秒级精度对查询延迟不够 | 查询延迟用 `date +%s%N` 计算纳秒差；如仍不够准考虑 hyperfine（可选依赖） |
| ripgrep / tokio 测量耗时长 | run-all.sh 提供 `--skip-external` 选项；外部仓库测量在独立 CI/手动场景跑 |
| 不修改 src/ vs Agent 测量精度 | 接受精度下降；如 build 阶段确认「不打点不可行」，需重新评估范围（升级为「打点 spike → 测量 spike」两步） |

## 5. Spec Patch（已回写到 delta spec）

本次设计回写以下变更到 `openspec/changes/codegraph-baseline-spike/specs/codegraph-baseline-bench/spec.md`：

1. **R4（Agent 使用率基线测量）** — 替换原 3 个 Scenarios 为 5 个：
   - `B 路径——CLI 回放标准任务集`（明确入口为 `wgenty-code query --prompt`）
   - `A 路径——历史 session 静态分析`（明确扫 `~/.wgenty-code/sessions/`）
   - `标准任务集规模与结构`（≥12 条 / 6 类 / YAML 字段约束）
   - 保留：`失败兜底路径`、`标准任务集可扩展`
2. **R5（基线报告产出）→ Scenario「后续 change 基线对照表」** — 升级为 6 列：`指标 / 当前基线值 / 建议目标值 / 行业对标参考 / 验证方法 / 归属 change`，并要求报告说明目标值制定规则
3. **R8（测量结果可重复）→ Scenario「同一仓库重跑」** — 单一 ±20% 改为分层阈值：恒定 ±1%、中等 ±20%、高波动 ±50%
4. **新增 R9（外部仓库验证）** — ripgrep 必跑 + tokio 可选 + 不绑死 wgenty-code 路径，覆盖原本仅在 R1/D5 隐含的约束

## 6. 测试策略

### 6.1 单元层（脚本本身）

- 测量脚本主要是 shell + jq + sqlite3，靠 dry-run + fixture 数据测试
- 准备 `scripts/codegraph-bench/test-fixtures/` 下的 mini index.db 验证 `bench-coverage.sh` 输出正确（已索引 N 文件、M 符号）
- 用一个固定时间戳 fixture 验证 `gen-report.sh` 拼装出的 markdown 结构稳定

### 6.2 集成层（在 wgenty-code 自身仓库）

- 跑 `run-all.sh --target . --repeats 2` 至少 2 次（含可重复性验证）
- 第一次跑作为基线归档；第二次跑对比波动是否在分层阈值内
- 确认所有 4 份 JSON + env.json 字段齐全

### 6.3 外部验证层（ripgrep）

- `git clone https://github.com/BurntSushi/ripgrep` 到 `/tmp/ripgrep`
- `bash scripts/codegraph-bench/run-all.sh --target /tmp/ripgrep --output ./scripts/codegraph-bench/results/<ts>-external-ripgrep`
- 验证 perf + coverage 跑通；agent 路径在外部仓库跳过（仅 wgenty-code 自身有 session）

### 6.4 报告内容验证

- markdown linter 检查格式
- 自定义 grep 检查必含章节存在（环境指纹、性能、覆盖率、Agent、根因、对照表）
- 检查对照表确实有 ≥3 行且每行 6 列

### 6.5 范围合规

- `git diff --name-only main...` 验证改动只出现在 `scripts/codegraph-bench/`、`docs/superpowers/specs/`、`openspec/changes/codegraph-baseline-spike/` 下
- 任何 `src/` 或 `openspec/specs/` 改动 = 立即视为违反

## 7. Build 阶段开局任务（探针）

进入 build 阶段后，首批任务用于消除当前未确认的技术风险，避免后期返工：

1. **试跑 2-3 条 B 路径任务**：用 `wgenty-code query --prompt "Tool trait 定义在哪？"` 验证：
   - query 输出能否直接拿到工具调用序列
   - 是否需要解析 session JSON 才能拿到完整序列
   - 单条任务的 token 成本量级
2. **读一个历史 session JSON**：从 `~/.wgenty-code/sessions/` 抽一个看 schema，验证 A 路径解析方案可行
3. **试跑 ripgrep 索引**：`wgenty-code codegraph index` 在 ripgrep 仓库的耗时和 index.db 体积，确认 ripgrep 选择合理

这三个探针完成后再进入正式 task 1.x（测量脚手架）实现，避免脚手架做完了发现 B 路径不可行。

## 8. Migration / 兼容性

不适用 —— spike 仅新增 `scripts/codegraph-bench/` 和 `docs/superpowers/specs/<date>-codegraph-baseline-report.md`，无运行时迁移、无回滚需求。归档时报告进入 docs，脚本继续保留以便后续 change 重跑对比。
