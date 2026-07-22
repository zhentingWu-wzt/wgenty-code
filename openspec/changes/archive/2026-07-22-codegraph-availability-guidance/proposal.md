## Why

CodeGraph 作为第三方本地 MCP server，在 `connect_configured_tools()` (`src/mcp/mod.rs`) 中被**无条件自动注入**：若用户配置无 `codegraph` 条目则追加 `codegraph serve --mcp`。现有 `McpServerStatus` 只有 `Running/Starting/Error/Stopped/Unknown` 五档，导致三个问题：

1. **状态粒度太粗**：codegraph 未安装（binary 不在 PATH）与已安装但未初始化，最终都坍缩成 `Error`/`Running`，用户无法区分该 `npm install` 还是 `codegraph init`。
2. **关键盲点**：`codegraph serve --mcp` 在未 `codegraph init` 时仍能正常启动（tools/list 成功），只在查询时返回 "uninitialized"。"未初始化"无法从 MCP 连接状态推断，必须独立探测 `.codegraph/` 索引目录标记。
3. **引导靠 prompt 兜底**：`base.md` 仅给 agent 一句泛化指令"工具缺失时回退 grep/lsp"，没有具体状态注入，agent 无法知道当前到底是哪种缺失。TUI 状态栏指示器只读配置 `status` 字段，不做真实探测。

## What Changes

- **独立可用性状态模型**：新建 `src/mcp/codegraph.rs`，定义 `CodegraphInstallState`（`Ready`/`NotInstalled`/`NotInitialized`/`Dismissed`），不污染通用 `McpServerStatus`。同步预探测 `probe_install_state()` 廉价判定（dismissed_paths -> which -> `.codegraph/` 目录）。
- **NotInstalled/Dismissed 短路**：在 `connect_configured_tools()` 中，`NotInstalled`/`Dismissed` 剔除 codegraph 跳过无谓 spawn；`NotInitialized` 仍连接（serve 可启动，agent 可自助 init）。
- **按项目免打扰持久化**：`IntegrationsConfig` 新增 `CodegraphSettings { dismissed_paths }`，`#[serde(default)]` 向后兼容；canonicalize 规范化路径去重。
- **CLI 启动通知（非交互）**：REPL/query 启动时按状态打印一行 stderr 通知（`NotInstalled`/`NotInitialized`/`ConnectionError`）；daemon 模式走 `tracing::warn!`；`Dismissed`/`Connected` 静默。
- **Prompt 注入实时状态**：environment 层注入 `CodeGraph status: <state>` 具体结论，替换 `base.md` 泛化文案；agent 在即将代码导航时用 `ask_user_question` 提供「立即安装/初始化 | 不再提示 | 本次跳过」三选。
- **`dismiss_codegraph_guidance` 元工具**：轻量 meta 工具（非只读），可靠写入免打扰状态，替代 `config set` 的数组语义。
- **TUI 状态栏指示器升级**：`detect_codegraph_status()` 改为返回真实探测结果，`codegraph_status_span()` 增加 ⚠/✗/○ 映射。

## Capabilities

### New Capabilities

- `codegraph-availability-guidance`: CodeGraph 安装/初始化可用性的差异化探测（NotInstalled/NotInitialized/Ready/Dismissed）、NotInstalled/Dismissed 短路、按项目免打扰持久化、CLI 启动通知、prompt 实时状态注入与 agent 引导行为、`dismiss_codegraph_guidance` 元工具、TUI 指示器升级。

## Impact

- **代码**：新增 `src/mcp/codegraph.rs`；修改 `src/mcp/mod.rs`（短路逻辑）、`src/config/services.rs`（`CodegraphSettings`）、`src/cli/args.rs`（启动通知）、`src/prompts/`（environment 注入 + base.md 文案）、`src/tui/app/{mod,render,event}.rs`（指示器）、`src/tools/meta/`（dismiss 工具 + 注册）。
- **配置**：新增 `integrations.codegraph.dismissed_paths`（`Vec<PathBuf>`，默认空）。
- **依赖**：复用已有 `which` crate，无新外部依赖。
- **安全**：`dismiss_codegraph_guidance` 修改 settings.json，声明 `is_read_only() = false`，经 guardian 审查；启动通知仅打印信息行，不弹交互式 y/n。
- **兼容**：`#[serde(default)]` 保证旧 settings.json 无损升级；`NotInstalled` 短路减少无谓 spawn 噪音日志。
