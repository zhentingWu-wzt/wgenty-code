#!/usr/bin/env bash
# gen-report.sh — Codegraph baseline report generator
#
# Assembles env.json + perf.json + coverage.json + agent.json into
# a 7-section Markdown report with a 6-column change comparison table.
#
# Usage:
#   gen-report.sh --output <dir>
#
# Output: <dir>/codegraph-baseline-report.md
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

# =====================================================================
# Usage & Arguments
# =====================================================================
usage() {
  cat <<'EOF'
Usage: gen-report.sh [OPTIONS]

Options:
  --output <dir>    Output directory (contains JSON files, also report output dir)
  --help            Show this help
EOF
  exit 0
}

OUTPUT_DIR=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --output) OUTPUT_DIR="$2"; shift 2 ;;
    --help) usage ;;
    *) echo "Unknown: $1" >&2; usage ;;
  esac
done

if [ -z "$OUTPUT_DIR" ]; then
  OUTPUT_DIR="results/latest"
fi

if [ ! -d "$OUTPUT_DIR" ]; then
  echo "ERROR: output directory not found: $OUTPUT_DIR" >&2
  exit 1
fi

if ! command -v jq &>/dev/null; then
  echo "ERROR: jq is required" >&2
  exit 1
fi

# =====================================================================
# Read JSON files with graceful fallbacks
# =====================================================================
ENV=$(cat "$OUTPUT_DIR/env.json" 2>/dev/null || echo '{"os":"N/A","cpu_count":"N/A","wgenty_version":"N/A","target_commit":"N/A","timestamp":"N/A"}')
PERF=$(cat "$OUTPUT_DIR/perf.json" 2>/dev/null || echo '{"full_index":{"median":"N/A","p95":"N/A","count":0,"samples":[]},"incremental_index":{"1_file":{"median":"N/A"},"10_files":{"median":"N/A"},"100_files":{"median":"N/A"}},"index_size":{"index_db_bytes":"N/A","source_rs_bytes":"N/A","ratio":"N/A"}}')
COV=$(cat "$OUTPUT_DIR/coverage.json" 2>/dev/null || echo '{"file_coverage":{"total_rs_files":"N/A","indexed_files":"N/A","coverage_pct":"N/A"},"symbols":{"total":"N/A"},"relationships":{"total":"N/A"},"parse_failures":{"total":"N/A","pct":"N/A"}}')
AGENT=$(cat "$OUTPUT_DIR/agent.json" 2>/dev/null || echo '{"summary":{"total_sessions":"N/A","sessions_with_codegraph":"N/A","adoption_rate_pct":"N/A","total_tool_calls":"N/A","codegraph_tool_calls":"N/A","codegraph_share_pct":"N/A"}}')

# =====================================================================
# Extract scalar values for clean template interpolation
# =====================================================================

REPORT_DATE=$(date +"%Y-%m-%d")

# env
ENV_OS=$(echo "$ENV" | jq -r '.os // "N/A"')
ENV_CPU=$(echo "$ENV" | jq -r '.cpu_count // "N/A"')
ENV_VERSION=$(echo "$ENV" | jq -r '.wgenty_version // "N/A"')
ENV_COMMIT=$(echo "$ENV" | jq -r '.target_commit // "N/A"')
ENV_TIMESTAMP=$(echo "$ENV" | jq -r '.timestamp // "N/A"')

# perf
PERF_MEDIAN=$(echo "$PERF" | jq -r '.full_index.median // "N/A"')
PERF_P95=$(echo "$PERF" | jq -r '.full_index.p95 // "N/A"')
PERF_SAMPLES=$(echo "$PERF" | jq -r '.full_index.samples | length // "N/A"')

PERF_INCR_1=$(echo "$PERF" | jq -r '.incremental_index["1_file"].median // "N/A"')
PERF_INCR_10=$(echo "$PERF" | jq -r '.incremental_index["10_files"].median // "N/A"')
PERF_INCR_100=$(echo "$PERF" | jq -r '.incremental_index["100_files"].median // "N/A"')

PERF_DB_BYTES=$(echo "$PERF" | jq -r '.index_size.index_db_bytes // "N/A"')
PERF_SRC_BYTES=$(echo "$PERF" | jq -r '.index_size.source_rs_bytes // "N/A"')
PERF_RATIO=$(echo "$PERF" | jq -r '.index_size.ratio // "N/A"')

# coverage
COV_TOTAL_RS=$(echo "$COV" | jq -r '.file_coverage.total_rs_files // "N/A"')
COV_INDEXED=$(echo "$COV" | jq -r '.file_coverage.indexed_files // "N/A"')
COV_PCT=$(echo "$COV" | jq -r '.file_coverage.coverage_pct // "N/A"')
COV_SYMBOLS=$(echo "$COV" | jq -r '.symbols.total // "N/A"')
COV_RELS=$(echo "$COV" | jq -r '.relationships.total // "N/A"')
COV_FAIL_PCT=$(echo "$COV" | jq -r '.parse_failures.pct // "N/A"')

# agent
AGENT_SESSIONS=$(echo "$AGENT" | jq -r '.summary.total_sessions // "N/A"')
AGENT_WITH_CG=$(echo "$AGENT" | jq -r '.summary.sessions_with_codegraph // "N/A"')
AGENT_ADOPTION=$(echo "$AGENT" | jq -r '.summary.adoption_rate_pct // "N/A"')
AGENT_SHARE=$(echo "$AGENT" | jq -r '.summary.codegraph_share_pct // "N/A"')

# =====================================================================
# Compute target values for the 6-column comparison table
# =====================================================================
# Rules:
#   - Agent adoption: baseline + 30pp
#   - Query latency / full-index duration: <= baseline x 1.5
#   - Multilang coverage: >= 70% (hardcoded; no baseline yet)

TARGET_ADOPTION=$(echo "$AGENT" | jq -r '
  if .summary.adoption_rate_pct | type == "number" then
    ((.summary.adoption_rate_pct | tonumber) + 30 | tostring) + "%"
  else
    "N/A"
  end
' 2>/dev/null || echo "N/A")

# =====================================================================
# Generate Markdown Report
# =====================================================================
REPORT_PATH="$OUTPUT_DIR/codegraph-baseline-report.md"

cat > "$REPORT_PATH" <<REPORT_HEADER
# Codegraph 基线报告

> 生成日期: $REPORT_DATE
> 测量环境: $ENV_OS / ${ENV_CPU} 核
> wgenty-code 版本: $ENV_VERSION
> 目标 commit: $ENV_COMMIT

## 1. 测量环境

| 字段 | 值 |
|------|-----|
| 操作系统 | $ENV_OS |
| CPU 核数 | $ENV_CPU |
| wgenty-code 版本 | $ENV_VERSION |
| 目标 commit | $ENV_COMMIT |
| 测量时间 | $ENV_TIMESTAMP |

## 2. 性能基线

### 全量索引

| 指标 | 值 |
|------|-----|
| 中位数 | $PERF_MEDIAN 秒 |
| p95 | $PERF_P95 秒 |
| 样本数 | $PERF_SAMPLES |

### 增量索引

| 规模 | 中位数 |
|------|--------|
| 1 文件 | ${PERF_INCR_1} 秒 |
| 10 文件 | ${PERF_INCR_10} 秒 |
| 100 文件 | ${PERF_INCR_100} 秒 |

### 索引体积

| 指标 | 值 |
|------|-----|
| index.db | $PERF_DB_BYTES 字节 |
| 源码 .rs | $PERF_SRC_BYTES 字节 |
| 比率 | $PERF_RATIO |

## 3. 覆盖率基线

| 指标 | 值 |
|------|-----|
| .rs 文件总数 | $COV_TOTAL_RS |
| 已索引文件 | $COV_INDEXED |
| 覆盖率 | ${COV_PCT}% |
| 符号总数 | $COV_SYMBOLS |
| 关系总数 | $COV_RELS |
| 解析失败率 | ${COV_FAIL_PCT}% |

## 4. Agent 使用率基线

| 指标 | 值 |
|------|-----|
| 总 session 数 | $AGENT_SESSIONS |
| 使用 codegraph 的 session | $AGENT_WITH_CG |
| codegraph 采纳率 | ${AGENT_ADOPTION}% |
| codegraph 工具调用占比 | ${AGENT_SHARE}% |

## 5. 根因分析

> 详见: scripts/codegraph-bench/root-cause-analysis.md

### Top 3 根因

1. **Prompt 优先级**: 系统 prompt 中 grep 排在 codegraph 之前，codegraph 未被列出
2. **工具描述缺乏场景引导**: tool description 纯功能描述，无"何时优先用"指示
3. **无懒初始化成功反馈**: Agent 不知道 codegraph 索引已就绪

## 6. 后续 change 基线 vs 目标对照表

| 指标 | 当前基线值 | 建议目标值 | 行业对标（参考） | 验证方法 | 归属 change |
|------|-----------|-----------|-----------------|---------|------------|
| agent codegraph 调用率 | ${AGENT_ADOPTION}% | ≥ ${TARGET_ADOPTION} | — | 重跑 agent-tasks | #1 codegraph-agent-adoption |
| codegraph_node p95 延迟 | ${PERF_P95} 秒 | ≤ 基线 × 1.5 | rust-analyzer: ~10ms | bench-query.sh | #2 codegraph-query-and-explainability |
| 全量索引耗时 | ${PERF_MEDIAN} 秒 | ≤ 基线 × 1.5 | sourcegraph: ~Xs | bench-perf.sh | #3 codegraph-multilang-and-deep-graph |
| 多语言覆盖率 | 0% | ≥ 70% | — | bench-coverage.sh | #3 codegraph-multilang-and-deep-graph |

**建议目标值制定规则**: 基线乘系数为主（b）、行业对标抽样为辅（a）、后续 change 在 design 阶段可调整不超过 ±20%（c）。

## 7. 附录

- 原始数据: $OUTPUT_DIR/
- env.json / perf.json / coverage.json / agent.json
- 根因分析: scripts/codegraph-bench/root-cause-analysis.md
- 稳定性窗口: 恒定指标 ±1%、中等波动 ±20%、高波动 ±50%
REPORT_HEADER

# Validate
if [ -f "$REPORT_PATH" ]; then
  echo "[gen-report] wrote $REPORT_PATH ($(wc -c < "$REPORT_PATH" | tr -d ' ') bytes)"
else
  echo "[gen-report] ERROR: failed to write $REPORT_PATH" >&2
  exit 1
fi
