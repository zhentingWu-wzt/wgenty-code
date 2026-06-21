# 快速开始指南

欢迎！这个指南将帮助你快速开始使用 Wgenty Code。

## 📋 目录

- [安装](#安装)
- [基本用法](#基本用法)
- [配置](#配置)
- [常见任务](#常见任务)
- [下一步](#下一步)

---

## 安装

### 选项 1: 从源代码编译（推荐）

```bash
# 克隆仓库
git clone https://github.com/zhentingWu-wzt/wgenty-code.git
cd wgenty-code

# 构建（需要 Rust 1.75+）
cargo build --release

# 可执行文件位于: ./target/release/wgenty-code (Linux/macOS) 或 wgenty-code.exe (Windows)
./target/release/wgenty-code --version
```

### 选项 2: 直接下载二进制

从 [GitHub Releases 页面](https://github.com/zhentingWu-wzt/wgenty-code/releases) 下载预编译的二进制文件。

```bash
# 手动下载后，添加到 PATH
chmod +x wgenty-code  # Linux/macOS
./wgenty-code --version
```

### 选项 3: Docker

```bash
# 构建本地镜像
docker build -t wgenty-code .

# 运行容器
docker run -it --rm wgenty-code --version
docker run -it --rm -v ~/.wgenty-code:/root/.wgenty-code wgenty-code repl
```

### 验证安装

```bash
wgenty-code --version
# 输出: wgenty-code 0.1.0
```

---

## 基本用法

### 1. 第一次运行

```bash
# 显示帮助
wgenty-code --help

# 显示版本
wgenty-code --version

# 显示系统信息
wgenty-code --info
```

### 2. 单次查询

```bash
# 使用 --prompt 标志
wgenty-code query --prompt "分析这个项目的结构"

# 或使用 -p 缩写
wgenty-code query -p "Explain async/await in Rust"

# 使用特定模型（sonnet / opus / haiku）
wgenty-code --model haiku query -p "Summarize this code"
```

### 3. REPL 交互模式

```bash
# 启动交互式 REPL
wgenty-code repl

# 带初始 prompt 启动
wgenty-code repl --prompt "分析当前项目"
```

REPL 快捷键：

| 按键 | 功能 |
|:-----|:-----|
| `Ctrl+P` | 切换 Plan 模式 |
| `Ctrl+O` | 展开/折叠最后一条工具输出 |
| `Ctrl+E` | 展开/折叠全部工具输出 |
| `Ctrl+T` | 切换任务面板 |
| `Ctrl+Shift+T` | 切换子代理监控面板 |
| `Ctrl+S` | 会话管理 |
| `Ctrl+L` | 清屏 |
| `Shift+Enter` | 输入中换行 |
| `Enter` | 提交输入 |
| `Ctrl+C`（双击） | 退出 |

### 4. 配置管理

```bash
# 查看当前配置
wgenty-code config show

# 设置模型
wgenty-code config set models.main.name haiku

# 设置 API 密钥
wgenty-code config set models.main.api_key "sk-ant-..."

# 重置配置到默认值
wgenty-code config reset
```

---

## 配置

### 快速配置

#### 1. 获取 API 密钥

访问 [Anthropic 控制台](https://console.anthropic.com/keys) 获取你的 API 密钥。

#### 2. 设置环境变量

```bash
# Linux/macOS
export ANTHROPIC_API_KEY="sk-ant-..."

# Windows (PowerShell)
$env:ANTHROPIC_API_KEY="sk-ant-..."
```

支持的环境变量（按优先级）：
- `ANTHROPIC_API_KEY` — Anthropic API 密钥
- `DASHSCOPE_API_KEY` — 阿里云 DashScope API 密钥
- `DEEPSEEK_API_KEY` — DeepSeek API 密钥
- `API_BASE_URL` — 自定义 API 端点

#### 3. 或通过配置文件

配置文件位于 `~/.wgenty-code/settings.json`（JSON 格式，首次运行自动生成）：

```json
{
  "models": {
    "main": {
      "name": "sonnet",
      "api_key": "sk-ant-..."
    },
    "transport": {
      "max_tokens": 4096,
      "timeout": 120
    }
  }
}
```

### 常用配置项

| 配置键 | 类型 | 默认值 | 说明 |
|:-------|:-----|:-------|:-----|
| `models.main.name` | string | `sonnet` | 主模型别名（sonnet/haiku/opus） |
| `models.main.api_key` | string | — | API 密钥（推荐用环境变量） |
| `models.main.base_url` | string | `https://api.anthropic.com` | API 端点 |
| `models.small` | object | — | 子代理专用小模型 |
| `models.transport.max_tokens` | number | 4096 | 单次请求最大 token |
| `models.transport.timeout` | number | 120 | 请求超时（秒） |
| `agent.plan_mode` | bool | false | Plan 模式开关 |
| `agent.subagent.max_depth` | number | 3 | 子代理最大嵌套深度 |
| `agent.subagent.max_concurrent` | number | 5 | 最大并发子代理数 |
| `agent.token_budget.main_k` | number | 0 | Token 预算（千），0=无限 |
| `integrations.guardian.enabled` | bool | true | 命令安全审查开关 |

使用 `wgenty-code config set <dotted.key> <value>` 修改任意配置项。例如：

```bash
wgenty-code config set agent.subagent.max_depth 5
wgenty-code config set agent.plan_mode true
```

---

## 常见任务

### 📝 代码分析

```bash
# 分析代码文件
wgenty-code query -p "分析 src/main.rs 的架构并指出潜在问题"
```

### 🐛 调试帮助

```bash
# 获取错误帮助
wgenty-code query -p "我遇到这个 Rust 编译错误: [粘贴错误信息]。如何修复？"
```

### 🚀 项目初始化

```bash
# 初始化新项目
wgenty-code init --name my-project
cd my-project
```

### 🔌 MCP 服务器管理

```bash
# 列出已配置的 MCP 服务器
wgenty-code mcp list

# 添加 MCP 服务器
wgenty-code mcp add --name filesystem --path /path/to/allowed/dir

# 移除 MCP 服务器
wgenty-code mcp remove --name filesystem

# 重启 MCP 服务器
wgenty-code mcp restart --name filesystem
```

### 🧩 插件管理

```bash
# 列出已安装插件
wgenty-code plugin list

# 安装插件
wgenty-code plugin install plugin-name

# 搜索插件
wgenty-code plugin search "关键词"

# 移除插件
wgenty-code plugin remove --name plugin-name

# 启用/禁用插件
wgenty-code plugin enable --name plugin-name
wgenty-code plugin disable --name plugin-name

# 更新所有插件
wgenty-code plugin update
```

### 🧠 内存与技能管理

```bash
# 查看内存状态
wgenty-code memory status

# 列出可用技能
wgenty-code skills list

# 搜索技能
wgenty-code skills search "关键词"

# 安装内置技能
wgenty-code skills install
```

### 🛡️ 沙箱管理

```bash
# 查看沙箱状态
wgenty-code sandbox status

# 启用/禁用沙箱
wgenty-code sandbox enable
wgenty-code sandbox disable
```

---

## 下一步

### 📚 更多资源

- [完整文档](./README.md)
- [安装指南](./INSTALL.md)
- [性能基准](./PERFORMANCE_BENCHMARKS.md)
- [变更日志](./CHANGELOG.md)

### 🆘 获取帮助

- GitHub Issues: [报告问题](../../issues)
- Discussions: [讨论和建议](../../discussions)

### 💡 学习更多

- Rust: https://www.rust-lang.org/learn
- Tokio: https://tokio.rs/
- Claude API: https://docs.anthropic.com

---

**祝你使用愉快！** ⚡
