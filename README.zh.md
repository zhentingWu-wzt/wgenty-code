[![Rust](https://img.shields.io/badge/Rust-1.75%2B-orange?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Platform](https://img.shields.io/badge/Platform-Windows%20%7C%20Linux%20%7C%20macOS-lightgrey.svg)]()

# Wgenty Code 🦀

> **高性能 Coding Agent CLI，用 Rust 重写** — 启动快 2.5 倍，二进制体积缩小 97%，零运行时依赖。

Wgenty Code 是一个 LLM 驱动的编程助手，通过终端界面读取、编写和重构代码。支持多种 AI 提供商（Anthropic、DeepSeek、DashScope），以单一自包含二进制发布，无需 Node.js 或 Python 运行时。

---

## 为什么用 Rust？

原始 TypeScript 实现携带了整个 Node.js 运行时 — 164 MB 的依赖、100 MB 的空闲内存、每次调用的 JIT 预热延迟。用 Rust 重写消除了这一切：

| 指标 | Rust | TypeScript | 提升 |
|:-----|:----:|:---------:|:----:|
| 冷启动 | **58 ms** | 152 ms | **2.6 倍** |
| 二进制大小 | **5 MB** | 164 MB | **缩小 97%** |
| 空闲内存 | **10 MB** | 100 MB | **减少 90%** |
| 配置读取 | **6 ms** | 150 ms | **快 25 倍** |
| REPL 按键响应 | **<1 ms** | 100 ms | **即时** |

超出数字之外，Rust 的所有权模型消除了整个类别的 bug：没有空指针异常、没有数据竞争、没有 GC 暂停。编译器在构建时证明内存安全和线程安全 — 在二进制运行之前。

详见 [PERFORMANCE_BENCHMARKS.md](PERFORMANCE_BENCHMARKS.md)。

---

## 设计亮点

### 🔒 默认安全

Agent 要执行的每条命令都经过**两级 Guardian 审查**：

1. **规则过滤** — 静态模式阻止明显危险的操作（如 `rm -rf /`、`curl | sh`）
2. **LLM 审查**（可选）— 模型评估模糊命令的风险，分类为 `低 / 中 / 高 / 严重`

严重风险操作自动拒绝。执行面还通过 **OS 级沙箱** 进一步隔离：macOS Seatbelt、Linux seccomp-bpf、Windows Job Objects。

### 🧩 25 种工具，一个抽象

所有 Agent 能力 — 文件操作、代码搜索、命令执行、网页访问 — 实现单一 `Tool` trait，关键设计选择：**`is_read_only()` 默认为 `false`**。每个只读工具必须显式声明自己是安全的。

### 📐 8 层 Prompt 组装

系统 prompt 按 8 层可独立开关的指令组装：

```
base_instructions → permissions → developer → collaboration
  → environment → skills → agents_md → wgenty_md
```

### 👥 RLM 架构 — 递归语言模型

复杂任务通过 **Planner → Executor → Aggregator** 管道自动分解为独立子任务：

```
模型 → task 工具（简单任务）
      → delegate 工具（复杂：自动分解 → 并行执行 → 合并）
      → dispatch 工具（map-reduce：grep 结果 → 逐项分析 → 聚合）
```

**RLM 管道（delegate 工具）：**
- Planner 调用 LLM 将任务分解为结构化 JSON 子任务
- Executor 按依赖层级并行运行子任务
- Aggregator 合并所有结果为一致性响应

**自动路由（task 工具）：**
`task` 工具检测复杂 prompt（>500 字符、多步骤指示）自动路由到 RLM 管道。

**递归控制：**
- 深度传播：每个子 agent 知道自身层级
- 硬限制：`max_subagent_depth`（默认 3）
- 自指：子 agent 可在深度允许时继续委托

### 🏗️ Plan Mode

配置中开启 `plan_mode` 或在 REPL 中按 `Ctrl+P`：

1. Agent 探索代码库，阅读相关文件，提出澄清问题
2. 调用 `update_plan` 在 UI 面板展示结构化计划
3. 等待用户批准后才执行变更

计划面板展示每步的状态标记：`○ 待办 / ◐ 进行中 / ✓ 已完成`。

### 🖥️ TUI 特性

基于 [ratatui](https://ratatui.rs/) 构建的终端界面：

- **基于 Turn 的聊天** — Turn 之间实线分隔，Turn 内虚线分隔
- **结构化 Plan 面板** — 带状态标记的内联计划渲染
- **折叠的工具结果** — 工具输出默认折叠（Ctrl+O 展开），减少噪音
- **Agent 模式切换** — `Normal / Plan / Accept Edits / Yolo` 带颜色编码标签
- **多行输入** — Shift+Enter 换行，完整 IME/CJK 支持
- **会话管理** — 保存/加载/删除/搜索会话

---

## 快速开始

### 前置条件
- **Rust** 1.75+ ([rustup.rs](https://rustup.rs/))
- **Git**

### 安装运行

```bash
git clone https://github.com/zhentingWu-wzt/wgenty-code.git
cd wgenty-code
cargo build --release

# 设置 API key
export ANTHROPIC_API_KEY="sk-ant-..."

# 开始编码
./target/release/wgenty-code repl
```

### Docker

```bash
docker build -t wgenty-code:latest .
docker run -it --rm -v ~/.wgenty-code:/root/.wgenty-code wgenty-code:latest repl
```

### 配置

配置文件位于 `~/.wgenty-code/settings.json`（自动生成）。关键选项：

| 配置键 | 默认值 | 用途 |
|:-------|:-------|:-----|
| `models.main.name` | `sonnet` | 主模型别名（自动映射） |
| `models.small.name` | *(无)* | 委托子任务的小型/廉价模型 |
| `agent.plan_mode` | `false` | 启用先计划后执行模式 |
| `agent.subagent.max_depth` | `3` | 嵌套子 agent 最大深度 |
| `agent.subagent.max_concurrent` | `5` | 并行子 agent 最大数量 |
| `agent.token_budget.main_k` | `0` | 累计 token 限制（0 = 无限制） |

> 使用 `wgenty-code config set <dotted.key> <value>` 修改配置，例如 `config set agent.subagent.max_depth 5`。

---

## CLI 速览

```bash
wgenty-code repl                      # 交互式 TUI 会话
wgenty-code query -p "重构这段代码"    # 一次性查询
wgenty-code config set models.main.name haiku    # 切换模型
wgenty-code mcp add --name fs         # 注册 MCP 服务器
wgenty-code sandbox status            # 检查沙箱状态
wgenty-code agent --agent-type plan --prompt "设计一个 API"
```

### REPL 快捷键

| 按键 | 功能 |
|:-----|:-----|
| `Ctrl+P` | 切换 Plan 模式 |
| `Ctrl+O` | 展开/折叠工具输出 |
| `Shift+Enter` | 输入中换行 |
| `Enter` | 提交输入 |
| `Ctrl+C` (双击) | 退出 |

---

## License

MIT — 详见 [LICENSE](LICENSE)。

**仓库**: [github.com/zhentingWu-wzt/wgenty-code](https://github.com/zhentingWu-wzt/wgenty-code)
