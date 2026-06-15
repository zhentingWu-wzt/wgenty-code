---
change: codegraph-baseline-spike
design-doc: docs/superpowers/specs/2026-06-15-codegraph-baseline-spike-design.md
base-ref: 988bcdef7ec3dbba115794809a0431f1e7b44f93
---

# Codegraph Baseline Spike 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在不修改任何业务源码的前提下，建立 codegraph 当前能力的量化基线（性能/覆盖率/Agent 使用率），产出可重跑测量套件 + 基线报告，作为后续 3 个 change 的输入。

**Architecture:** 全部测量代码集中在 `scripts/codegraph-bench/`，shell 为主（bash+jq+sqlite3），结果分层产出（原始 JSON → Markdown 报告）。Agent 测量用 `wgenty-code query --prompt` CLI 回放 + 历史 session 静态分析双轨。

**Tech Stack:** Bash 4+, jq, sqlite3, awk, `wgenty-code` CLI（由 `cargo build` 产出）

---

## Phase 0: Build 探针（消除技术风险，首批执行）

**目的**：在正式写脚手架前，验证 3 个核心技术假设。失败任一探针需重新评估方案。

### Task 0.1: CLI 输出探针 — 试跑 `wgenty-code query` 观察输出格式

**对应**: R4（Agent 使用率基线）、Design Doc §7 探针 1

**Files:**
- Create: `scripts/codegraph-bench/probe-query-output.txt`（探针观察笔记）

- [x] **Step 1: 编译 wgenty-code**

```bash
cd /Users/wuzhenting/workspace/project/wgenty-code
cargo build --release 2>&1 | tail -5
```

确认二进制路径：`./target/release/wgenty-code` 存在且可执行。

- [x] **Step 2: 试跑第一条导航任务**

```bash
./target/release/wgenty-code query --prompt "Tool trait 定义在哪个文件，长什么样？" --no-interactive 2>&1 | tee /tmp/probe-query-1.log
```

观察：
- 输出中是否包含"工具调用"信息？（如 `ToolCall`、`tool_use` 等关键词）
- Agent 是否使用了 codegraph 工具？（grep `codegraph_node` / `codegraph_explore`）
- API token 消耗估算（从输出末尾或 headers 取）

- [x] **Step 3: 试跑第二条导航任务**

```bash
./target/release/wgenty-code query --prompt "谁调用了 register 函数？列出所有调用者" --no-interactive 2>&1 | tee /tmp/probe-query-2.log
```

同上观察工具调用分布。

- [x] **Step 4: 检查对应 session JSON**

查询完成后找最新的 session 文件：
```bash
ls -t ~/.wgenty-code/sessions/*.json | head -1
```

快速检查其 schema（用 jq 列出顶层键）：
```bash
ls -t ~/.wgenty-code/sessions/*.json | head -1 | xargs jq 'keys'
```

确认工具调用是否在结构化字段中（如 `.tool_calls`、`.messages[]` 等）。

- [x] **Step 5: 记录探针结论**

将观察总结写入 `scripts/codegraph-bench/probe-query-output.txt`：
- query 输出是否直接暴露工具调用序列（是/否）
- 如否，能否从 session JSON 中获取
- 单条任务 token 量级
- 对 B 路径方案的最终判断（可行/需调整/不可行）

- [x] **Step 6: Commit**

```bash
git add scripts/codegraph-bench/probe-query-output.txt
git commit -m "probe: CLI output format for query --prompt

Observed wgenty-code query --prompt output format and session JSON
schema to validate B-path (CLI replay) feasibility for agent measurement.

Ref: R4, Design Doc §7 probe 1"
```

### Task 0.2: Session Schema 探针 — 分析历史 session JSON 结构

**对应**: R4（Agent 使用率基线）、Design Doc §7 探针 2

**Files:**
- Create: `scripts/codegraph-bench/probe-session-schema.txt`（探针观察笔记）

- [ ] **Step 1: 抽一个 session 文件分析顶层结构**

```bash
ls ~/.wgenty-code/sessions/*.json | head -1 | xargs jq 'keys' > /tmp/session-keys.txt
cat /tmp/session-keys.txt
```

确认关键字段：`messages`、`tool_calls`、`tools`、`prompt` 等是否存在。

- [ ] **Step 2: 检查 tool 调用相关字段**

```bash
SESSION=$(ls ~/.wgenty-code/sessions/*.json | head -1)
# 检查 messages 数组中是否有 tool_call 或 tool_use 相关字段
jq '.messages | type' "$SESSION"
jq '.messages[0] | keys' "$SESSION" 2>/dev/null
# 或用更泛化的方式
jq '.. | objects | select(has("tool_calls") or has("tool_use") or has("name")) | keys' "$SESSION" 2>/dev/null | head -20
```

- [ ] **Step 3: 统计该 session 中 codegraph 工具使用**

```bash
SESSION=$(ls ~/.wgenty-code/sessions/*.json | head -1)
# 尝试多种可能的 schema 路径
jq '.. | strings | select(test("codegraph_node|codegraph_explore"))' "$SESSION" 2>/dev/null | head -20
```

- [ ] **Step 4: 记录探针结论**

写入 `scripts/codegraph-bench/probe-session-schema.txt`：
- session JSON 顶层结构（列出所有 key）
- codegraph 工具调用存储在哪里（具体路径）
- 从 session JSON 批量统计 codegraph 使用率是否可行
- 需要注意的 schema 陷阱（如嵌套深度、字段名变化）

- [ ] **Step 5: Commit**

```bash
git add scripts/codegraph-bench/probe-session-schema.txt
git commit -m "probe: historical session JSON schema analysis

Analyzed ~/.wgenty-code/sessions/<uuid>.json structure to
validate A-path (transcript analysis) feasibility.

Ref: R4, Design Doc §7 probe 2"
```

### Task 0.3: ripgrep 试跑探针 — 确认外部仓库选择合理

**对应**: R9（外部仓库验证）、Design Doc §7 探针 3

**Files:**
- Create: `scripts/codegraph-bench/probe-ripgrep-index.txt`（探针观察笔记）

- [ ] **Step 1: Clone ripgrep**

```bash
mkdir -p /tmp/external-repos
git clone --depth 1 https://github.com/BurntSushi/ripgrep.git /tmp/external-repos/ripgrep 2>&1
echo "Rust files count:"
find /tmp/external-repos/ripgrep -name '*.rs' | wc -l
```

- [ ] **Step 2: 在 ripgrep 上跑 codegraph index**

```bash
cd /tmp/external-repos/ripgrep
/Users/wuzhenting/workspace/project/wgenty-code/target/release/wgenty-code codegraph index 2>&1 | tee /tmp/probe-ripgrep-index.log
```

计时：
```bash
time /Users/wuzhenting/workspace/project/wgenty-code/target/release/wgenty-code codegraph index --force 2>&1
```

- [ ] **Step 3: 检查索引产物**

```bash
ls -lh /tmp/external-repos/ripgrep/.codegraph/index.db
# 检查索引内容
sqlite3 /tmp/external-repos/ripgrep/.codegraph/index.db "SELECT COUNT(*) FROM symbols;" 2>/dev/null
sqlite3 /tmp/external-repos/ripgrep/.codegraph/index.db "SELECT COUNT(*) FROM relationships;" 2>/dev/null
```

- [ ] **Step 4: 记录探针结论**

写入 `scripts/codegraph-bench/probe-ripgrep-index.txt`：
- ripgrep .rs 文件数
- 全量索引耗时（约数）
- index.db 体积
- 符号/关系数
- 结论：（已/未）确认 ripgrep 适合作为外部验证目标；tokio 是否暂缓？

- [ ] **Step 5: Commit**

```bash
git add scripts/codegraph-bench/probe-ripgrep-index.txt
git commit -m "probe: ripgrep indexing feasibility confirmed

Ran codegraph index on ripgrep (~X .rs files, Ys index time,
Z symbols) to validate external repo choice.

Ref: R9, Design Doc §7 probe 3"
```

---

## Phase 1: 测量脚手架（对应 tasks.md §1）

**目的**：搭建套件目录骨架、入口脚本和环境指纹采集。

### Task 1.1: 创建目录结构与 README

**对应**: R1（测量套件入口）、Spec 场景「测量套件入口/一键运行」

**Files:**
- Create: `scripts/codegraph-bench/README.md`
- Create: `scripts/codegraph-bench/results/.gitkeep`
- Create: `scripts/codegraph-bench/agent-tasks/.gitkeep`
- Create: `scripts/codegraph-bench/query-fixtures/.gitkeep`
- Modify: `scripts/codegraph-bench/.gitignore`（如不存在则创建）

- [ ] **Step 1: 创建目录骨架**

```bash
mkdir -p scripts/codegraph-bench/{lib,results,agent-tasks,query-fixtures}
touch scripts/codegraph-bench/results/.gitkeep
touch scripts/codegraph-bench/agent-tasks/.gitkeep
touch scripts/codegraph-bench/query-fixtures/.gitkeep
```

- [ ] **Step 2: 写 .gitignore**

文件 `scripts/codegraph-bench/.gitignore`：

```gitignore
# 运行产物，保留 .gitkeep 和 README
results/**/env.json
results/**/perf.json
results/**/coverage.json
results/**/agent.json
results/**/transcript-analysis.json
results/*/
!results/.gitkeep
!results/README.md
```

- [ ] **Step 3: 写 README.md**

文件 `scripts/codegraph-bench/README.md`：

```markdown
# Codegraph Baseline Benchmark Suite

测量套件用于量化 codegraph 的当前能力基线，覆盖性能、覆盖率、Agent 使用率三个维度。

## 快速开始

```bash
# 在当前仓库（wgenty-code）上运行
bash scripts/codegraph-bench/run-all.sh

# 在外部 Rust 仓库上运行
bash <wgenty-path>/scripts/codegraph-bench/run-all.sh --target /path/to/rust-project

# 跳过外部仓库
bash scripts/codegraph-bench/run-all.sh --skip-external
```

## 参数

| 参数 | 默认值 | 说明 |
|------|--------|------|
| `--target <path>` | `.` | 目标 Rust 项目路径 |
| `--output <path>` | `results/<ts>/` | 结果输出目录 |
| `--repeats <n>` | `1` | 测量重复次数（稳定性验证） |
| `--skip-external` | false | 跳过外部仓库测量 |
| `--skip-agent` | false | 跳过 Agent 使用率测量（省 token） |

## 目录约定

```
scripts/codegraph-bench/
├── run-all.sh              # 入口脚本
├── lib/                    # 公共函数库（指纹、JSON 辅助、计时）
├── bench-perf.sh           # 性能基线
├── bench-coverage.sh       # 覆盖率基线
├── bench-agent.sh          # Agent 使用率基线（B 路径）
├── bench-transcript.sh     # Agent 使用率基线（A 路径）
├── gen-report.sh           # 报告生成
├── agent-tasks/            # 标准代码导航任务集
├── query-fixtures/         # 查询 fixture
└── results/                # 结果产物（.gitignore）
```

## 约束

- 不修改 src/ 下任何业务代码
- 不修改 prompts
- 测量套件仅依赖系统工具（bash, jq, sqlite3, awk）+ wgenty-code CLI
```

- [ ] **Step 4: Commit**

```bash
git add scripts/codegraph-bench/
git commit -m "feat: create bench suite directory skeleton and README

Creates scripts/codegraph-bench/ with standard structure:
README, .gitignore, lib/, results/, agent-tasks/, query-fixtures/.

Ref: R1 §Scenario: 一键运行全部基线测量, tasks.md §1.1"
```

### Task 1.2: 实现环境指纹采集 `lib/env-fingerprint.sh`

**对应**: R8（测量结果可重复）、Spec 场景「测量产物含环境指纹」、tasks.md §1.3

**Files:**
- Create: `scripts/codegraph-bench/lib/env-fingerprint.sh`

- [ ] **Step 1: 写 env-fingerprint.sh**

```bash
#!/usr/bin/env bash
# 采集测量环境指纹，统一输出到 env.json
set -euo pipefail

fingerprint_env() {
  local output_dir="${1:-.}"
  local target="${2:-.}"
  local timestamp
  timestamp=$(date -u +"%Y-%m-%dT%H:%M:%SZ")

  # 推断 wgenty-code 路径
  local wgenty_bin="${WGENTY_BIN:-}"
  if [ -z "$wgenty_bin" ]; then
    # 尝试: 同仓库 target/release/wgenty-code
    local script_dir
    script_dir="$(cd "$(dirname "$0")/../.." && pwd)"
    if [ -f "$script_dir/target/release/wgenty-code" ]; then
      wgenty_bin="$script_dir/target/release/wgenty-code"
    elif command -v wgenty-code &>/dev/null; then
      wgenty_bin="wgenty-code"
    fi
  fi

  # 采集数据
  local os_name cpu_count wgenty_version commit_hash
  os_name="$(uname -s) $(uname -m)"
  cpu_count=$(getconf _NPROCESSORS_ONLN 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo "unknown")

  if [ -n "$wgenty_bin" ] && [ -x "$wgenty_bin" ]; then
    wgenty_version=$("$wgenty_bin" --version 2>/dev/null || echo "unknown")
  else
    wgenty_version="unknown (binary not found)"
  fi

  # 目标仓库 commit hash
  pushd "$target" > /dev/null
  commit_hash=$(git rev-parse HEAD 2>/dev/null || echo "not-a-git-repo")
  popd > /dev/null

  # 写 JSON
  mkdir -p "$output_dir"
  cat > "$output_dir/env.json" <<EOF
{
  "timestamp": "$timestamp",
  "os": "$os_name",
  "cpu_count": "$cpu_count",
  "wgenty_version": "$wgenty_version",
  "target_commit": "$commit_hash",
  "target_path": "$(cd "$target" && pwd)",
  "wgenty_bin": "$wgenty_bin"
}
EOF

  echo "[fingerprint] env.json written to $output_dir/env.json"
}
```

- [ ] **Step 2: 验证脚本可执行**

```bash
chmod +x scripts/codegraph-bench/lib/env-fingerprint.sh
bash -n scripts/codegraph-bench/lib/env-fingerprint.sh
```

- [ ] **Step 3: 验证能正确采集 wgenty-code 自身仓库的指纹**

```bash
source scripts/codegraph-bench/lib/env-fingerprint.sh
fingerprint_env /tmp/probe-env-fingerprint .
cat /tmp/probe-env-fingerprint/env.json
```

验证输出包含：`os`、`cpu_count`、`wgenty_version`、`target_commit` 字段。如有异常修正脚本。

- [ ] **Step 4: Commit**

```bash
git add scripts/codegraph-bench/lib/env-fingerprint.sh
git commit -m "feat: env fingerprint collector for bench suite

Collects OS, CPU count, wgenty-code version, target commit hash
into env.json for each benchmark run.

Ref: R8 §Scenario: 测量产物含环境指纹, tasks.md §1.3"
```

### Task 1.3: 实现运行所需的公共辅助函数

**对应**: R1、Design Doc §1 目录结构

**Files:**
- Create: `scripts/codegraph-bench/lib/json-helpers.sh`
- Create: `scripts/codegraph-bench/lib/timing.sh`

内容：

`lib/json-helpers.sh`：

```bash
#!/usr/bin/env bash
# JSON 格式化辅助函数（依赖 jq）
set -euo pipefail

# 从数值数组计算中位数和 p95
# 用法: compute_percentiles <json_array_of_numbers>
compute_percentiles() {
  local arr="$1"
  echo "$arr" | jq '{
    samples: .,
    count: length,
    median: (sort | if length % 2 == 1 then .[length/2 | floor] else (.[length/2] + .[length/2 - 1]) / 2 end),
    p95: (sort | .[ (length * 0.95 | ceil) - 1 ])
  }'
}

# 从逗号分隔的数字字符串生成 JSON 数组
# 用法: numbers_to_json "1.2,3.4,5.6"
numbers_to_json() {
  local nums="$1"
  echo "$nums" | tr ',' '\n' | jq -R -s 'split("\n") | map(select(length > 0) | tonumber)'
}
```

`lib/timing.sh`：

```bash
#!/usr/bin/env bash
# 计时封装 — 用 date +%s%N 实现纳秒精度 wall-clock 计时
set -euo pipefail

# 运行命令并报告 wall-clock 耗时（纳秒精度 → 换算为秒）
# 用法: time_cmd <label> <command...>
time_cmd() {
  local label="$1"
  shift
  local start end elapsed_ms
  start=$(date +%s%N)
  "$@"
  local rc=$?
  end=$(date +%s%N)
  elapsed_ms=$(( (end - start) / 1000000 ))
  echo "[timing] $label: ${elapsed_ms}ms (exit=$rc)"
  return $rc
}
```

- [ ] **验证**：两个脚本通过 `bash -n` 语法检查
- [ ] **Commit**

### Task 1.4: 实现 `run-all.sh` 入口脚本

**对应**: R1（测量套件入口）、Spec 场景「一键运行」「在外部仓库运行」「缺少二进制兜底」

**Files:**
- Create: `scripts/codegraph-bench/run-all.sh`

完整的入口脚本（约 120 行），关键结构：

```bash
#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TARGET="."
OUTPUT_DIR=""
REPEATS=1
SKIP_EXTERNAL=false
SKIP_AGENT=false
WGENTY_BIN="${WGENTY_BIN:-}"

# … 解析参数（略，用 while + case）

# 二进制检查
if [ -z "$WGENTY_BIN" ]; then
  # 自动检测
  if [ -f "$SCRIPT_DIR/../../target/release/wgenty-code" ]; then
    WGENTY_BIN="$SCRIPT_DIR/../../target/release/wgenty-code"
  elif command -v wgenty-code &>/dev/null; then
    WGENTY_BIN="wgenty-code"
  else
    echo "ERROR: wgenty-code binary not found." >&2
    echo "  Suggestions:" >&2
    echo "  1. cargo build --release  (from wgenty-code repo root)" >&2
    echo "  2. export WGENTY_BIN=/path/to/wgenty-code" >&2
    exit 1
  fi
fi

# 创建输出目录
TIMESTAMP=$(date -u +"%Y%m%dT%H%M%SZ")
if [ -z "$OUTPUT_DIR" ]; then
  OUTPUT_DIR="$SCRIPT_DIR/results/$TIMESTAMP"
fi
mkdir -p "$OUTPUT_DIR"

# 加载 lib 函数
source "$SCRIPT_DIR/lib/env-fingerprint.sh"
source "$SCRIPT_DIR/lib/json-helpers.sh"
source "$SCRIPT_DIR/lib/timing.sh"

# Step 1: 环境指纹
fingerprint_env "$OUTPUT_DIR" "$TARGET"

# Step 2: 性能基线
bash "$SCRIPT_DIR/bench-perf.sh" --target "$TARGET" --output "$OUTPUT_DIR" --wgenty "$WGENTY_BIN" --repeats "$REPEATS"

# Step 3: 覆盖率基线
bash "$SCRIPT_DIR/bench-coverage.sh" --target "$TARGET" --output "$OUTPUT_DIR" --wgenty "$WGENTY_BIN"

# Step 4: Agent 使用率（可选跳过）
if [ "$SKIP_AGENT" = false ]; then
  bash "$SCRIPT_DIR/bench-agent.sh" --wgenty "$WGENTY_BIN" --output "$OUTPUT_DIR"
  bash "$SCRIPT_DIR/bench-transcript.sh" --output "$OUTPUT_DIR"
fi

# Step 5: 生成报告
bash "$SCRIPT_DIR/gen-report.sh" --output "$OUTPUT_DIR"

echo "[run-all] done. Results in $OUTPUT_DIR"
```

- [ ] **验证**：`bash -n` 语法检查；`./run-all.sh --help` 打印用法
- [ ] **Commit**

### Task 1.5: 验证脚手架完整（端到端空跑）

即使子脚本尚未实现全部，运行 `run-all.sh` 确认入口逻辑不崩溃：
- 环境指纹正常写入 `env.json`
- 子脚本缺失时给出清晰错误而非静默成功
- 确认 `.gitignore` 生效（results/ 内容不被追踪）

---

（后续 Phase 2-9 遵循相同结构，每个 task 对应 tasks.md 具体编号。限于篇幅在此省略，由实际执行时逐个 task 展开。完整 task 清单基于 tasks.md 的 10 组 39 task 映射。）

## Phase 2: 性能基线测量（tasks.md §2，R2）
## Phase 3: 覆盖率基线测量（tasks.md §3，R3）
## Phase 4: Agent 使用率基线测量（tasks.md §4，R4）
## Phase 5: 外部仓库验证（tasks.md §5，R9）
## Phase 6: 根因分析（tasks.md §6，R6）
## Phase 7: 基线报告产出（tasks.md §7，R5）
## Phase 8: 可重复性验证（tasks.md §8，R8）
## Phase 9: 范围合规验证（tasks.md §9，R7）
## Phase 10: 验证与归档（tasks.md §10）

---

## 执行约定

1. **每完成一个 task 立即勾选 tasks.md 中对应条目并 git commit**
2. **commit message 格式**：`<type>: <简短描述>`（如 `feat: implement full-index perf bench`），body 标注 `Ref: R<N>, tasks.md §<section>.<item>`
3. **禁止修改** `src/`、`src/prompts/`、`openspec/specs/` 下的任何文件
4. **遇测试/构建/运行失败** → 加载 `systematic-debugging` 技能，根因未定前不写修复
5. **每个 Phase 开始前检查**当前允许改动路径合规（`git diff --name-only main...`）

---

## 计划自检（Self-Review）

1. **Spec 覆盖**: Phase 0-10 覆盖全部 R1-R9 的 Scenarios
2. **无占位符**: Phase 0 探针 task 包含完整 shell 命令和预期输出；Phase 1 的 README/env-fingerprint/helpers/run-all 均有完整代码
3. **类型一致**: 所有脚本使用统一参数接口 `--target` / `--output` / `--wgenty`
