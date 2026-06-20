#!/bin/bash
# ============================================================
# 📊 Wgenty Code Rust - macOS Performance Benchmark
# ============================================================
# 用法: chmod +x benchmark.sh && ./benchmark.sh
# ============================================================

set -e

# 颜色定义
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m' # No Color

# 配置
RUNS=5
RUST_BIN="./target/release/wgenty-code"
RESULTS_DIR="./benchmark_results"

# 创建结果目录
mkdir -p "$RESULTS_DIR"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
RESULT_FILE="$RESULTS_DIR/benchmark_${TIMESTAMP}.txt"

# Python helper functions for robust statistics (via stdin — avoids bash quoting issues)
py_avg() {
  python3 -c "import sys; data=[float(x) for x in sys.stdin.read().split()]; print(round(sum(data)/len(data),1))"
}
py_min() {
  python3 -c "import sys; data=[float(x) for x in sys.stdin.read().split()]; print(min(data))"
}
py_max() {
  python3 -c "import sys; data=[float(x) for x in sys.stdin.read().split()]; print(max(data))"
}
py_std() {
  python3 -c "
import sys, statistics
data = [float(x) for x in sys.stdin.read().split()]
print(round(statistics.stdev(data), 1)) if len(data) > 1 else print(0)
"
}
py_divide() {
  local a="$1" b="$2"
  python3 -c "print(round($a / $b, 1))" 2>/dev/null || echo 0
}

echo -e "${BOLD}${CYAN}"
echo "========================================"
echo "  📊 Wgenty Code Rust - macOS Benchmark"
echo "========================================"
echo -e "${NC}"

# 检测系统信息
echo -e "${BOLD}${BLUE}[系统信息]${NC}"
echo "  操作系统: $(sw_vers -productName) $(sw_vers -productVersion)"
echo "  芯片:     $(sysctl -n machdep.cpu.brand_string 2>/dev/null || echo 'Apple Silicon')"
echo "  内存:     $(( $(sysctl -n hw.memsize) / 1024 / 1024 / 1024 )) GB"
echo "  CPU核心:  $(sysctl -n hw.ncpu) 核"
echo "  日期:     $(date '+%Y-%m-%d %H:%M:%S')"
echo ""

# 检查 Rust 二进制
if [ ! -f "$RUST_BIN" ]; then
    echo -e "${YELLOW}⚠️  未找到 release 二进制，正在编译...${NC}"
    cargo build --release 2>&1 | tail -5
    if [ ! -f "$RUST_BIN" ]; then
        echo -e "${RED}❌ 编译失败，请检查代码后重试${NC}"
        exit 1
    fi
    echo -e "${GREEN}✅ 编译完成${NC}"
    echo ""
fi

# 获取二进制大小
RUST_SIZE=$(du -m "$RUST_BIN" | awk '{print $1}')
echo -e "${BOLD}${BLUE}[二进制信息]${NC}"
echo "  路径: $RUST_BIN"
echo "  大小: ${RUST_SIZE} MB"
echo ""

# ============================================================
# Test 1: 启动速度 (冷启动)
# ============================================================
echo -e "${BOLD}${GREEN}[Test 1] 启动速度测试 (--version)${NC}"
echo "  运行 $RUNS 次..."

RUST_TIMES=()
for i in $(seq 1 $RUNS); do
    # 清除磁盘缓存 (macOS)
    # sudo purge 2>/dev/null || true
    START=$(python3 -c 'import time; print(time.time())')
    "$RUST_BIN" --version > /dev/null 2>&1
    END=$(python3 -c 'import time; print(time.time())')
    ELAPSED=$(python3 -c "print(round(($END - $START) * 1000, 1))")
    RUST_TIMES+=($ELAPSED)
    echo "  Run $i: ${ELAPSED}ms"
done

# 计算统计 (via stdin helpers — avoids bash quoting issues)
RUST_AVG=$(echo "${RUST_TIMES[*]}" | py_avg)
RUST_MIN=$(echo "${RUST_TIMES[*]}" | py_min)
RUST_MAX=$(echo "${RUST_TIMES[*]}" | py_max)
RUST_STD=$(echo "${RUST_TIMES[*]}" | py_std)

echo -e "  ${GREEN}平均: ${RUST_AVG}ms | 最快: ${RUST_MIN}ms | 最慢: ${RUST_MAX}ms | 标准差: ${RUST_STD}ms${NC}"
echo ""

# ============================================================
# Test 2: Help 命令
# ============================================================
echo -e "${BOLD}${GREEN}[Test 2] Help 命令测试 (--help)${NC}"
echo "  运行 $RUNS 次..."

HELP_TIMES=()
for i in $(seq 1 $RUNS); do
    START=$(python3 -c 'import time; print(time.time())')
    "$RUST_BIN" --help > /dev/null 2>&1
    END=$(python3 -c 'import time; print(time.time())')
    ELAPSED=$(python3 -c "print(round(($END - $START) * 1000, 1))")
    HELP_TIMES+=($ELAPSED)
    echo "  Run $i: ${ELAPSED}ms"
done

HELP_AVG=$(echo "${HELP_TIMES[*]}" | py_avg)
HELP_MIN=$(echo "${HELP_TIMES[*]}" | py_min)
HELP_MAX=$(echo "${HELP_TIMES[*]}" | py_max)
HELP_STD=$(echo "${HELP_TIMES[*]}" | py_std)

echo -e "  ${GREEN}平均: ${HELP_AVG}ms | 最快: ${HELP_MIN}ms | 最慢: ${HELP_MAX}ms | 标准差: ${HELP_STD}ms${NC}"
echo ""

# ============================================================
# Test 3: 配置查询
# ============================================================
echo -e "${BOLD}${GREEN}[Test 3] 配置查询测试 (config show)${NC}"
echo "  运行 $RUNS 次..."

CONFIG_TIMES=()
for i in $(seq 1 $RUNS); do
    START=$(python3 -c 'import time; print(time.time())')
    "$RUST_BIN" config show > /dev/null 2>&1
    END=$(python3 -c 'import time; print(time.time())')
    ELAPSED=$(python3 -c "print(round(($END - $START) * 1000, 1))")
    CONFIG_TIMES+=($ELAPSED)
    echo "  Run $i: ${ELAPSED}ms"
done

CONFIG_AVG=$(echo "${CONFIG_TIMES[*]}" | py_avg)
CONFIG_MIN=$(echo "${CONFIG_TIMES[*]}" | py_min)
CONFIG_MAX=$(echo "${CONFIG_TIMES[*]}" | py_max)
CONFIG_STD=$(echo "${CONFIG_TIMES[*]}" | py_std)

echo -e "  ${GREEN}平均: ${CONFIG_AVG}ms | 最快: ${CONFIG_MIN}ms | 最慢: ${CONFIG_MAX}ms | 标准差: ${CONFIG_STD}ms${NC}"
echo ""

# ============================================================
# Test 4: 内存占用
# ============================================================
echo -e "${BOLD}${GREEN}[Test 4] 内存占用测试${NC}"

# 启动进程并测量内存
"$RUST_BIN" --version > /dev/null 2>&1 &
RUST_PID=$!
sleep 0.5

if [ -n "$RUST_PID" ] && kill -0 "$RUST_PID" 2>/dev/null; then
    # macOS 使用 footprint 或 rss
    MEM_RSS=$(ps -o rss= -p "$RUST_PID" 2>/dev/null | awk '{print $1}')
    if [ -n "$MEM_RSS" ]; then
        MEM_MB=$((MEM_RSS / 1024))
        echo "  Rust 进程 RSS 内存: ${MEM_MB} MB"
    else
        echo "  无法获取内存数据"
    fi
    kill "$RUST_PID" 2>/dev/null || true
else
    echo "  进程已退出，无法测量内存"
fi
echo ""

# ============================================================
# Test 5: 并发启动测试
# ============================================================
echo -e "${BOLD}${GREEN}[Test 5] 并发启动测试 (10 个实例)${NC}"

CONCURRENT=10
START=$(python3 -c 'import time; print(time.time())')
PIDS=()
for i in $(seq 1 $CONCURRENT); do
    "$RUST_BIN" --version > /dev/null 2>&1 &
    PIDS+=($!)
done

# 等待所有进程完成
for pid in "${PIDS[@]}"; do
    wait "$pid" 2>/dev/null || true
done
END=$(python3 -c 'import time; print(time.time())')
CONCURRENT_TIME=$(python3 -c "print(round(($END - $START) * 1000, 1))")

echo "  ${CONCURRENT} 个并发实例总耗时: ${CONCURRENT_TIME}ms"
echo "  平均每个实例: $(py_divide "$CONCURRENT_TIME" "$CONCURRENT")ms"
echo ""

# ============================================================
# Test 6: TypeScript 对比 (如果安装了 Node.js 版本)
# ============================================================
TS_AVG="N/A"
TS_HELP_AVG="N/A"

if command -v npx &> /dev/null; then
    echo -e "${BOLD}${GREEN}[Test 6] TypeScript 对比测试${NC}"
    echo "  检测到 Node.js: $(node --version)"
    
    # 检查是否有 wgenty-code 的 TS 版本
    if command -v claude &> /dev/null; then
        echo "  检测到 claude CLI，运行对比..."
        
        TS_TIMES=()
        for i in $(seq 1 $RUNS); do
            START=$(python3 -c 'import time; print(time.time())')
            claude --version > /dev/null 2>&1
            END=$(python3 -c 'import time; print(time.time())')
            ELAPSED=$(python3 -c "print(round(($END - $START) * 1000, 1))")
            TS_TIMES+=($ELAPSED)
            echo "  TS Run $i: ${ELAPSED}ms"
        done
        
        TS_AVG=$(echo "${TS_TIMES[*]}" | py_avg)
        echo -e "  TypeScript 平均启动: ${TS_AVG}ms"
    else
        echo "  未检测到 claude CLI，跳过 TS 对比"
        echo "  提示: 安装 wgenty code TS 版后可自动对比"
    fi
else
    echo -e "${YELLOW}[Test 6] 跳过 - 未安装 Node.js${NC}"
fi
echo ""

# ============================================================
# 汇总报告
# ============================================================
echo -e "${BOLD}${CYAN}"
echo "========================================"
echo "  📊 BENCHMARK SUMMARY"
echo "========================================"
echo -e "${NC}"

echo -e "${BOLD}系统环境:${NC}"
echo "  OS:       $(sw_vers -productName) $(sw_vers -productVersion)"
echo "  Chip:     $(sysctl -n machdep.cpu.brand_string 2>/dev/null || sysctl -n hw.optional.arm64 2>/dev/null && echo 'Apple Silicon' || echo 'Intel')"
echo "  Memory:   $(( $(sysctl -n hw.memsize) / 1024 / 1024 / 1024 )) GB"
echo ""

echo -e "${BOLD}Rust 版本测试结果:${NC}"
echo "┌─────────────────────┬──────────┬──────────┬──────────┬──────────┐"
echo "│ 测试项              │ 平均(ms) │ 最快(ms) │ 最慢(ms) │ 标准差   │"
echo "├─────────────────────┼──────────┼──────────┼──────────┼──────────┤"
printf "│ --version           │ %8s │ %8s │ %8s │ %8s │\n" "$RUST_AVG" "$RUST_MIN" "$RUST_MAX" "$RUST_STD"
printf "│ --help              │ %8s │ %8s │ %8s │ %8s │\n" "$HELP_AVG" "$HELP_MIN" "$HELP_MAX" "$HELP_STD"
printf "│ config show         │ %8s │ %8s │ %8s │ %8s │\n" "$CONFIG_AVG" "$CONFIG_MIN" "$CONFIG_MAX" "$CONFIG_STD"
echo "└─────────────────────┴──────────┴──────────┴──────────┴──────────┘"
echo ""

echo -e "${BOLD}部署体积:${NC}"
echo "  Rust 二进制: ${RUST_SIZE} MB"
echo ""

echo -e "${BOLD}并发性能:${NC}"
echo "  ${CONCURRENT} 并发实例总耗时: ${CONCURRENT_TIME}ms"
echo ""

if [ "$TS_AVG" != "N/A" ]; then
    echo -e "${BOLD}Rust vs TypeScript 对比:${NC}"
    SPEEDUP=$(py_divide "$TS_AVG" "$RUST_AVG")
    echo "  Rust 启动:        ${RUST_AVG}ms"
    echo "  TypeScript 启动:  ${TS_AVG}ms"
    echo -e "  ${GREEN}性能提升: ${SPEEDUP}x ⚡${NC}"
    echo ""
fi

# 保存结果到文件
{
    echo "Wgenty Code Rust Benchmark Results"
    echo "Date: $(date '+%Y-%m-%d %H:%M:%S')"
    echo "OS: $(sw_vers -productName) $(sw_vers -productVersion)"
    echo ""
    echo "--version: avg=${RUST_AVG}ms min=${RUST_MIN}ms max=${RUST_MAX}ms std=${RUST_STD}ms"
    echo "--help: avg=${HELP_AVG}ms min=${HELP_MIN}ms max=${HELP_MAX}ms std=${HELP_STD}ms"
    echo "config show: avg=${CONFIG_AVG}ms min=${CONFIG_MIN}ms max=${CONFIG_MAX}ms std=${CONFIG_STD}ms"
    echo "binary size: ${RUST_SIZE} MB"
    echo "concurrent ${CONCURRENT}: ${CONCURRENT_TIME}ms"
    [ "$TS_AVG" != "N/A" ] && echo "TS comparison: Rust ${RUST_AVG}ms vs TS ${TS_AVG}ms = ${SPEEDUP}x"
} > "$RESULT_FILE"

echo -e "${BOLD}${BLUE}结果已保存到: ${RESULT_FILE}${NC}"
echo ""
echo -e "${BOLD}${CYAN}Made with ❤️ and Rust 🦀${NC}"
