# WGENTY.md

此文件为 Wgenty Code（claude.ai/code）在此仓库中工作时提供指导。

## 项目元信息

- **名称**: `wgenty_code`
- **版本**: `0.1.0`
- **描述**: High-performance Rust implementation of Wgenty Code CLI
- **语言**: Rust 2021 edition (MSRV 1.75+)
- **许可证**: MIT
- **仓库**: https://github.com/zhentingWu-wzt/wgenty-code

## 构建与运行

```bash
cargo build                          # Debug
cargo build --release                # Release
cargo run -- repl                    # REPL（默认）
cargo run -- repl --prompt "分析项目"
cargo run -- query --prompt "hello"  # 单次查询
cargo run -- --version / --help
```

### CLI 子命令

`Repl | Query | Config | Mcp | Plugin | Memory | Voice | Init | Update | Services | Agent | MagicDocs | TeamSync | Skills | Sandbox | StressTest | Daemon`

- **Config**: `show | set <key> <value> | reset`
- **Mcp**: `add --name <n> [--path] | remove --name | list | restart`
- **Plugin**: `install | remove | update | enable | disable | search | list`
- **Memory**: `status | clear | dream | autodream`
- **Agent**: `--agent-type <explore|plan|general-purpose> --prompt <text>`
- **Skills**: `list | execute <name> [args...] | search <query>`
- **Sandbox**: `status | enable | disable`
- **Daemon**: `--port <port>`（默认 8371）

### Docker

```bash
docker build -t wgenty-code:latest .
docker run --rm wgenty-code --version
docker run -it --rm -v ~/.wgenty-code:/home/claude/.wgenty-code wgenty-code repl
```

---

## 测试、Lint 与格式化

```bash
cargo test                                     # 全部测试
cargo test --all                               # 所有 target
cargo test <test_name>                         # 单测过滤
cargo fmt --check                              # 格式检查（CI 强制）
cargo fmt                                      # 自动格式化
cargo clippy -- -D warnings                    # 零 warning（CI 强制）
cargo clippy --all-targets -- -D warnings
cargo clippy --fix -- -D warnings              # 自动修复
```

---

## Feature Flags

```toml
default = ["i18n", "daemon", "bundled-skills"]
wasm = ["wasm-bindgen", "wasm-bindgen-futures", "js-sys", "web-sys"]
daemon = ["axum", "tower", "tower-http", "tokio-stream"]
i18n = ["fluent", "fluent-bundle", "unic-langid", "rust-embed"]
bundled-skills = ["rust-embed"]
export-icon = ["image"]
bundled-sqlite = ["rusqlite/bundled"]
full = ["wasm", "i18n", "daemon", "bundled-skills", "export-icon", "bundled-sqlite"]
```

按需构建：`cargo build --release --no-default-features`(纯CLI)，`--features full`(全量)

### 多个二进制目标

| 目标 | 入口 | Required Features |
|------|------|------------------|
| `wgenty-code` | `src/main.rs` | 无（default-run） |

---

## 架构概述

基于 **Harness Component Model**（s01-s12 机制模块）：

```
前端层 (CLI/TUI/Daemon)
  -> Agent Loop (agent/)          s01+s02: 核心循环 + SSE 流
  -> Prompt Assembly (prompts/)   8 层指令注入
  -> 业务层
     tools/        s01: Agent 工具（文件/搜索/执行/元操作）
     context/      s06+s07: 记忆/会话/压缩
     tasks/        s03+s07: 任务追踪
     teams/        s04,s09-s12: 子代理/团队
  -> 安全层
     guardian/     命令安全审查（规则+LLM 两阶段）
     sandbox/      OS 进程隔离
  -> 基础设施层
     api/          多 Provider 客户端
     mcp/          MCP 协议
     plugins/      插件系统
     config/       配置管理
```

请求链路：`用户输入 -> CLI解析 -> Settings加载 -> Prompt组装(8层) -> API SSE -> 工具调用 -> Guardian审查 -> Sandbox执行 -> 流式返回`

Prompt 8 层：base_instructions → permissions → developer → environment → agents_md → collaboration → skills_inventory → wgenty_md_sections

---

## 核心模块

- **agent/**: `StreamProcessor` 共享 SSE 流解析，产生 `StreamEvent`(Chunk/ToolCall/Error/Done)
- **api/**: `ApiClient` 多 Provider 支持(Anthropic/DeepSeek/DashScope)，`detect_provider()` 自动路由；模型映射: sonnet->claude-3-5-sonnet-20241022
- **tools/**: `Tool` trait(name/description/input_schema/execute/is_read_only)，**`is_read_only()` 默认 false**，只读工具必须显式返回 true。25个内置工具：filesystem(read/write/edit/apply_patch/list/view)、search(grep/glob/search/web_search/web_fetch)、execution(exec_command/kill_session/git/run_test/background)、meta(think/lsp/ask_user/update_plan/note_edit/compact)、checkpoint。`with_settings()` 按 provider 动态移除不兼容工具
- **guardian/**: 两阶段审查（规则+LLM），RiskLevel: Low/Medium/High/Critical
- **sandbox/**: `SandboxBackend` trait，macOS(Seatbelt)/Linux(seccomp-bpf)/Windows(Job Objects)，无内核时降级 no-op
- **context/**: `ConsolidationEngine` 3层压缩，`ContextWindow`/`HistoryManager` 窗口管理
- **tasks/**: `TodoWrite` 会话清单(max 20, 1 in_progress)，`TaskManagement` 持久化 CRUD
- **teams/**: `AgentSession` 子代理(explore/plan/general-purpose)，`mailbox` 异步 JSONL
- **mcp/**: JSON-RPC 2.0 server + stdio client，支持外部 server 的 initialize/tools/list/tools/call，并将远程工具代理进统一 `ToolRegistry`
- **CodeGraph**: 通过本地第三方 `codegraph serve --mcp` 提供代码导航；项目内不再维护重复的 tree-sitter/SQLite 索引器。未安装或未初始化时降级到 grep/lsp
- **plugins/**: `PluginManifest`，热加载+隔离
- **services/**: `ServiceManager` 管理 auto_dream/voice/magic_docs/team_sync
- **i18n/**: Fluent 格式，10 语言，feature-gated (`i18n`)

---

## 配置

**文件**: `~/.wgenty-code/settings.json`（JSON 格式，首次自动生成）

| 配置路径 | 类型 | 默认值 | 说明 |
|---------|------|--------|------|
| `models.main.name` | String | `sonnet` | 主模型别名（sonnet/haiku/opus） |
| `models.main.api_key` | Option | env var | API 密钥（推荐用环境变量） |
| `models.main.base_url` | Option | `https://api.anthropic.com` | API 地址 |
| `models.small` | Option | None | 子代理用小模型端点 |
| `models.planner` | Option | None | 规划专用模型端点 |
| `models.transport.max_tokens` | usize | 4096 | 最大 token 数 |
| `models.transport.timeout` | u64 | 120 | 请求超时(秒) |
| `agent.plan_mode` | bool | false | 规划模式 |
| `agent.token_budget.main_k` | usize | 0 | 主 Agent Token 预算(千)，0=无限 |
| `agent.token_budget.subagent_default_k` | usize | 0 | 子代理默认 Token 预算(千) |
| `agent.subagent.max_depth` | usize | 3 | 子代理最大嵌套深度 |
| `agent.subagent.max_concurrent` | usize | 5 | 最大并发子代理 |
| `agent.subagent.timeout_secs` | u64 | 1800 | 子代理超时(秒) |
| `agent.rlm.enabled` | bool | true | RLM 管道主开关 |
| `prompt.developer_instructions` | Option | None | 用户自定义指令 |
| `prompt.model_instructions_file` | Option | None | 模型指令文件路径 |
| `prompt.collaboration_mode` | Option | None | 协作模式 |
| `integrations.guardian.enabled` | bool | true | 安全审查开关 |
| `integrations.guardian.llm_review` | bool | false | LLM 审查开关 |
| `integrations.guardian.auto_deny_critical` | bool | true | 自动拒绝 Critical |
| `storage.transcript.max_age_days` | u32 | 30 | 子代理记录保留天数 |

**环境变量优先级**: `ANTHROPIC_API_KEY` > `DASHSCOPE_API_KEY` > `DEEPSEEK_API_KEY`，`API_BASE_URL` 覆盖配置文件，`RUST_LOG` 控制日志级别

---

## 关键依赖

| 依赖 | 用途 |
|------|------|
| clap 4.5 | CLI 参数解析 |
| tokio 1.37 + futures 0.3 | 异步运行时 |
| reqwest 0.12 | HTTP 客户端（json+stream+rustls） |
| ratatui 0.29 + crossterm 0.28 | 终端 TUI |
| axum 0.7 + tower-http 0.5 | Daemon HTTP |
| serde + serde_json | 序列化 |
| tracing 0.1 + tracing-subscriber 0.3 | 日志 |
| fluent 0.16 + unic-langid 0.9 | 国际化 |
| pulldown-cmark 0.10 | Markdown 解析 |
| rusqlite 0.31 | SQLite 应用存储（默认系统库，`bundled-sqlite` feature 切换内置编译） |
| walkdir 2.5 + glob 0.3 | 文件系统遍历 |
| regex 1.10 + nom 7.1 | 解析 |
| similar 2.5 | Diff 算法 |
| thiserror 1.0 + anyhow 1.0 | 错误处理 |
| dashmap 5.5 + lru 0.12 | 并发缓存 |
| config 0.14 + toml 0.8 + dirs 5.0 | 配置管理 |
| sha2 0.10 + base64 0.22 + jsonwebtoken 9.3 | 加密鉴权 |
| async-trait 0.1 | 异步 trait |
| uuid 1.8 + chrono 0.4 | UUID + 时间 |
| which 6.0 + notify 6.1 | 进程查找 + 文件系统监控 |
| http 1 + fs_extra 1.3 + image 0.25 | HTTP 类型 + 文件操作 + 图像处理 |
| textwrap 0.16 + tui-textarea 0.7 | TUI 文本排版 |
| tempfile 3.10 + mockall 0.12 | 测试工具(dev) |

---

## CI/CD

`.github/workflows/ci.yml` — push main/develop 或 PR 触发：

| Job | 命令 |
|-----|------|
| check | `cargo check --all-targets` |
| test | `cargo test --all` |
| fmt | `cargo fmt -- --check` |
| clippy | `cargo clippy --all-targets -- -D warnings` |
| build | `cargo build --release`（ubuntu/windows/macos 三平台） |

`.github/workflows/release.yml` — push `v*` tag 触发：三平台 Release 构建 + Docker 镜像

---

## 设计决策与已知限制

1. **子代理限制**: max_subagent_depth=3, max_concurrent_subagents=5
2. **token_budget_k=0**: 无限，可设置累计 token 上限
3. **API key 运行时重新读取**: 每次调用从环境变量重新读取，支持切换不重启
4. **Prompts 8 层可选**: 各 include_xxx 开关控制，优雅降级
5. **Sandbox 多平台**: 统一 SandboxBackend trait，无内核支持降级 no-op
6. **技能按需加载**: 仅注入名称+描述到 Layer 7，完整内容由 agent 动态获取
7. **多 Provider API 路由**: 根据 base_url 自动检测，透明转换请求格式
8. **模型名简写映射**: sonnet/haiku/opus 自动映射完整 Anthropic model ID
9. **CI 中 binary_name**: release.yml 使用 `wgenty_code_rs`，与 Cargo.toml 的 `wgenty-code` 不一致（历史遗留）
10. **待补充文档**: `PERFORMANCE_BENCHMARKS.md`、`MIGRATION_GUIDE.md`、`src/README.md`、`docs/API.md` 在 CHANGELOG 中引用但尚未创建
11. **SQLite 系统库优先**: 默认链接系统 SQLite（macOS/Linux），避免 ~60s C 编译；Windows 和无系统库环境通过 `bundled-sqlite` feature 内置编译

## Context injection channels

wgenty-code 提供两层用户级上下文通道，自动随每轮 user message 注入：

- `~/.wgenty-code/WGENTY.md` — 用户级全局指令（对所有项目生效）。
- `~/.wgenty-code/rules/*.md` — 用户级规则文件（顶层 `.md`，按文件名字母序拼入）。

加上项目根的 `WGENTY.md` / `AGENTS.md`，共 4 个静态源；UserPromptSubmit hook 的 `InjectContext` 动态注入也走同一通道。每轮内容会以 `<system-reminder>` 块拼到 user message 头部。
