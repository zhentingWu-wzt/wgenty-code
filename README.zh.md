[![Rust](https://img.shields.io/badge/Rust-1.75%2B-orange?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Platform](https://img.shields.io/badge/Platform-Windows%20%7C%20Linux%20%7C%20macOS-lightgrey.svg)]()

# Wgenty Code 🦀

> 一个运行在终端里的高性能 AI 编码 Agent。用自然语言探索、编辑、搜索、重构整个代码库——启动快、二进制小、零运行时依赖。

Wgenty Code 是用 Rust 编写的 LLM 驱动编程助手。你不必再把代码片段复制到聊天框里，而是直接把它指向一个真实项目：它会读文件、跑搜索、执行命令、应用修改，并不断迭代直到任务完成——全部来自一个自包含二进制，无需 Node.js 或 Python 运行时。

它内置 **25 种工具**（文件系统、代码搜索、命令执行、网页访问……），配备**两级命令 Guardian 审查**和**全平台 OS 级沙箱**，让 Agent 在自主行动的同时默认安全。支持多 AI 提供商自动路由——**Anthropic (Claude)**、**OpenAI**、**DeepSeek**，以及任何 OpenAI 兼容端点（DashScope、Ollama、vLLM……）——模型别名 `sonnet`、`haiku`、`opus` 会被透明映射。

[English](README.md)

---

## 功能特性

- **交互式 TUI** - 基于 Turn 的聊天、结构化计划面板、可折叠的工具输出、Agent 模式切换（`Normal / Plan / Accept Edits / Yolo`）
- **Plan 模式** - Agent 先探索代码库并提出计划，*再*执行任何修改（`Ctrl+P` 切换）；在你批准前不会改动任何东西
- **25 种内置工具** - 文件读/写/编辑、代码搜索（grep/glob/LSP）、命令执行、网页搜索/获取等
- **多提供商路由** - 根据 base_url 自动检测提供商；改一个设置即可在 Claude、OpenAI、DeepSeek 或自托管端点之间切换
- **默认安全** - 每条命令都经过两级 Guardian 审查（规则 + 可选 LLM 审查）；严重风险操作自动拒绝；全平台 OS 级沙箱：macOS Seatbelt、Linux seccomp-bpf、Windows Job Objects
- **子代理委派** - 复杂任务自动分解为并行子任务，带递归控制（RLM 管道：Planner -> Executor -> Aggregator）
- **会话与记忆管理** - 保存/加载/搜索历史会话；项目级 + 全局记忆，带 TF-IDF 召回
- **MCP 支持** - 连接外部 MCP 服务器，在 Agent 循环中透明使用其工具
- **i18n** - 通过 Fluent 本地化支持 10 种语言

---

## 为什么用 Rust？

原始 TypeScript 实现携带了整个 Node.js 运行时——164 MB 的依赖、100 MB 的空闲内存、每次调用的 JIT 预热延迟。用 Rust 重写消除了这一切：

| 指标 | Rust | TypeScript | 提升 |
|:-----|:----:|:---------:|:----:|
| 冷启动 | **58 ms** | 152 ms | **快 2.6 倍** |
| 二进制大小 | **5 MB** | 164 MB | **缩小 97%** |
| 空闲内存 | **10 MB** | 100 MB | **减少 90%** |
| 配置读取 | **6 ms** | 150 ms | **快 25 倍** |
| REPL 按键响应 | **<1 ms** | 100 ms | **即时** |

超出数字之外，Rust 的所有权模型消除了整类 bug：没有空指针异常、没有数据竞争、没有 GC 暂停。编译器在构建时证明内存安全与线程安全——在二进制运行之前。

详见 [PERFORMANCE_BENCHMARKS.md](PERFORMANCE_BENCHMARKS.md)。

---

## 工作原理

### 🔒 默认安全

Agent 要执行的每条命令都经过**两级 Guardian 审查**：

1. **规则过滤** - 静态模式阻止明显危险的操作（如 `rm -rf /`、`curl | sh`）
2. **LLM 审查**（可选）- 模型评估模糊命令的风险，分类为 `低 / 中 / 高 / 严重`

严重风险操作自动拒绝。执行面还通过 **OS 级沙箱**进一步隔离（macOS Seatbelt、Linux seccomp-bpf、Windows Job Objects），无内核支持时优雅降级为 no-op。

### 🧩 25 种工具，一个抽象

所有 Agent 能力——文件操作、代码搜索、命令执行、网页访问——都实现单一 `Tool` trait。关键设计选择：**`is_read_only()` 默认为 `false`**。每个只读工具必须显式声明自己是安全的，这样 Guardian 始终偏向谨慎。

### 📐 8 层 Prompt 组装

系统 prompt 由 8 个可独立开关的层组装而成：

```
base_instructions -> permissions -> developer -> collaboration
  -> environment -> skills -> agents_md -> wgenty_md
```

### 👥 RLM - 递归任务分解

复杂任务通过 **Planner -> Executor -> Aggregator** 管道处理：

- `task` 工具 - 简单的单次委派；自动将复杂 prompt 路由到 RLM 管道
- `delegate` 工具 - 将任务分解为结构化子任务，按依赖层级并行执行并合并结果
- 递归由 `agent.subagent.max_depth` 硬限制（默认 `1`）

### 🏗️ Plan 模式

在配置中开启 `plan_mode`，或在 REPL 中按 `Ctrl+P`：

1. Agent 探索代码库，阅读相关文件，提出澄清问题
2. 调用 `update_plan` 在 UI 面板渲染结构化计划
3. 等待你批准后才做任何修改

计划面板展示每步状态：`○ 待办 / ◐ 进行中 / ✓ 已完成`。

### 🖥️ TUI

基于 [ratatui](https://ratatui.rs/) 构建的终端界面：

- **基于 Turn 的聊天** - Turn 之间实线分隔，Turn 内虚线分隔
- **结构化 Plan 面板** - 带状态标记的内联计划渲染
- **折叠的工具结果** - `Ctrl+O` 展开，减少噪音
- **Agent 模式切换** - `Normal / Plan / Accept Edits / Yolo` 带颜色编码标签
- **多行输入** - `Shift+Enter` 换行，完整 IME/CJK 支持

---

## 快速开始

### 通过 npm 安装（推荐）

需要 [Node.js](https://nodejs.org/) 14+。npm 包会自动为你下载对应平台的预编译二进制——无需 Rust 工具链。

```bash
npm install -g wgenty-code
wgenty-code --version     # 验证安装
```

支持平台：`linux-x64`、`linux-arm64`、`darwin-x64`（Intel macOS）、`darwin-arm64`（Apple Silicon）、`win32-x64`。

### 从源码构建

需要 **Rust** 1.75+（[rustup.rs](https://rustup.rs/)）和 **Git**。

```bash
git clone https://github.com/zhentingWu-wzt/wgenty-code.git
cd wgenty-code
cargo build --release
```

二进制位于 `./target/release/wgenty-code`（Windows 下为 `.exe`）。

### 设置 API key 并运行

```bash
# 设置你的 API key（以下任选其一）
export ANTHROPIC_API_KEY="sk-ant-..."    # Anthropic Claude
# export DEEPSEEK_API_KEY="sk-..."       # DeepSeek
# export DASHSCOPE_API_KEY="sk-..."      # DashScope（阿里云）

# 开始编码
wgenty-code                            # 通过 npm 安装时
# ./target/release/wgenty-code         # 从源码构建时
```

> 也可以在 `~/.wgenty-code/settings.json` 中设置 `api_key`。环境变量优先于配置文件。

### Docker

```bash
docker build -t wgenty-code:latest .
docker run -it --rm -v ~/.wgenty-code:/root/.wgenty-code wgenty-code:latest repl
```

### 配置

配置文件位于 `~/.wgenty-code/settings.json`（首次运行自动生成）。关键选项：

| 配置键 | 默认值 | 用途 |
|:-------|:-------|:-----|
| `models.main.name` | `sonnet` | 主模型别名（自动映射） |
| `models.small.name` | *(无)* | 委托子任务的小型/廉价模型 |
| `models.planner.name` | *(无)* | 生成计划专用模型 |
| `models.transport.max_tokens` | `4096` | 单次请求最大 token |
| `agent.plan_mode` | `false` | 启用先计划后执行模式 |
| `agent.subagent.max_depth` | `1` | 嵌套子 agent 最大深度（1 = 子代理不能再派生子代理；调大以允许递归） |
| `agent.subagent.max_concurrent` | `5` | 并行子 agent 最大数量 |
| `agent.token_budget.main_k` | `0` | 累计 token 限制（0 = 无限制） |
| `integrations.guardian.enabled` | `true` | 命令安全审查开关 |
| `storage.transcript.max_age_days` | `30` | 子代理记录保留天数 |

> 使用 `wgenty-code config set <dotted.path> <value>` 修改任意配置，例如 `config set agent.subagent.max_depth 5`。

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

完整命令参考：`wgenty-code --help`

### REPL 快捷键

| 按键 | 功能 |
|:-----|:-----|
| `Ctrl+P` | 切换 Plan 模式 |
| `Ctrl+O` | 展开/折叠工具输出 |
| `Shift+Enter` | 输入中换行 |
| `Enter` | 提交输入 |
| `Ctrl+C`（双击） | 退出 |

---

## 开发

```bash
cargo build                           # Debug 构建
cargo test --all                      # 完整测试套件
cargo clippy --all-targets -- -D warnings  # 零 warning（强制）
cargo fmt                             # 自动格式化
```

分支约定、提交格式与 PR 流程见 [CONTRIBUTING.md](CONTRIBUTING.md)。

---

## 文档

- [QUICKSTART.md](QUICKSTART.md) - 上手实践指南
- [INSTALL.md](INSTALL.md) - 各平台安装说明
- [PERFORMANCE_BENCHMARKS.md](PERFORMANCE_BENCHMARKS.md) - 完整性能数据
- [MIGRATION_GUIDE.md](MIGRATION_GUIDE.md) - 从 TypeScript 版本迁移
- [CHANGELOG.md](CHANGELOG.md) - 发布历史
- [CONTRIBUTING.md](CONTRIBUTING.md) - 如何贡献

---

## License

MIT - 详见 [LICENSE](LICENSE)。

**仓库**: [github.com/zhentingWu-wzt/wgenty-code](https://github.com/zhentingWu-wzt/wgenty-code)
