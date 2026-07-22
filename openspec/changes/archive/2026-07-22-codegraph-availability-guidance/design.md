# Design: CodeGraph 安装/初始化差异化引导

## Context

CodeGraph 作为第三方本地 MCP server，在 `connect_configured_tools()` (`src/mcp/mod.rs:258`) 中被**无条件自动注入**：若用户配置无 `codegraph` 条目，则追加 `McpConfig::codegraph()` (`src/config/mcp_config.rs:119`)，命令为 `codegraph serve --mcp`。连接由 `connect_server()` (`mod.rs:313`) 以 15s 超时执行 spawn + initialize + tools/list。

当前问题：

1. **状态粒度太粗**。`McpServerStatus` 只有 `Running/Starting/Error/Stopped/Unknown` 五档。codegraph 未安装（binary 不在 PATH）与已安装但未初始化，最终都坍缩成 `Error` 或 `Running`，用户无法区分该 `npm install` 还是 `codegraph init`。
2. **关键盲点**：`codegraph serve --mcp` 在**未 `codegraph init` 时仍能正常启动**（tools/list 成功），只在查询时返回 "uninitialized"。因此"未初始化"**无法从 MCP 连接状态推断**，必须独立探测 `.codegraph/` 索引目录标记。
3. **引导靠 prompt 兜底**。`base.md:147` 仅给 agent 一句泛化指令"工具缺失时回退 grep/lsp，可安装 `@colbymchenry/codegraph`"，没有具体状态注入，agent 无法知道当前到底是哪种缺失。
4. **TUI 已有指示器但不可靠**。`detect_codegraph_status()` (`tui/app/mod.rs:974`) 只读配置里的 `status` 字段，不做真实探测；`codegraph_status_span()` (`render.rs:312`) 只能渲染 5 档粗图标。

已有可复用资产：`which` crate（依赖内，探测 PATH）、`.codegraph/` 目录作为 init 标记、`base.md` 的 grep/lsp 回退机制、TUI 状态栏 CG 指示器。

## Goals & Non-Goals

**Goals**
- 区分四种可用性状态：`NotInstalled` / `NotInitialized` / `Connected` / `ConnectionError`，外加 `Dismissed`（用户已免打扰）。
- 双渠道引导：启动时 CLI 一行通知（即时反馈）+ prompt 注入实时状态（agent 可在对话中复述/补充）。
- 按项目持久化免打扰：agent 在需要代码导航时用 `ask_user_question` 询问，用户选"不再提示"则写入该 working_dir，之后 CLI 通知与 agent 询问均静默。
- `NotInstalled` 时短路跳过 MCP spawn，避免无谓的失败连接尝试。
- 无启动摩擦：启动时不弹交互式 y/n，仅打印信息行。

**Non-Goals**
- 代用户执行 `npm install` 或 `codegraph init`（变更系统状态，属用户决策；agent 仅口播命令，经批准后可由 exec_command 执行）。
- 替换 `base.md` 既有的 grep/lsp 回退语义。
- 按仓库语言条件化引导（codegraph 基于 tree-sitter 自带多语言支持，无需宿主判定语言）。
- 校验 `.codegraph/index.db` 完整性（目录存在即视为已初始化；损坏索引属运行时回退范畴）。

## Design Decisions

### D1: 独立 `CodegraphAvailability` 枚举，新建 `src/mcp/codegraph.rs`

不扩展通用的 `McpServerStatus`（它是所有 MCP server 共享的类型，`NotInstalled`/`NotInitialized` 对 filesystem 等其他 server 无意义，会造成语义污染）。新建模块定义：

```rust
pub enum CodegraphAvailability {
    /// 二进制已装、索引存在、MCP 连接成功
    Connected,
    /// codegraph 不在 PATH 上
    NotInstalled,
    /// 二进制已装，但 working_dir 下无 .codegraph/ 索引目录
    NotInitialized,
    /// 二进制+索引都在，但 MCP spawn/initialize/tools-list 失败
    ConnectionError(String),
    /// 用户已对该项目关闭引导
    Dismissed,
}
```

探测分两阶段：
- **同步预探测** `probe_install_state(settings) -> InstallState`（廉价，启动期同步执行）：
  1. working_dir 规范化路径 ∈ `settings.integrations.codegraph.dismissed_paths` → `Dismissed`
  2. `which::which("codegraph")` 失败 → `NotInstalled`
  3. `!working_dir.join(".codegraph").exists()` → `NotInitialized`
  4. 否则 → `Ready`
- **最终可用性** 在 `connect_configured_tools` 之后派生：`Ready` + MCP `Running` → `Connected`；`Ready` + MCP `Error` → `ConnectionError(last_error)`；`NotInstalled`/`NotInitialized`/`Dismissed` 直接透传。

### D2: `NotInstalled` 短路 + `NotInitialized` 仍连接

在 `connect_configured_tools()` (`mod.rs:258`) 现有的 codegraph 自动注入逻辑后，插入预探测：
- `NotInstalled`：从 `auto_start_configs` 中**剔除** codegraph，跳过 spawn（`codegraph serve` spawn 会立即 `command not found`，虽快但产生噪音日志；短路更干净）。记录 `CodegraphAvailability::NotInstalled`。
- `NotInitialized`：**仍纳入连接**。理由：`serve --mcp` 能启动，agent 可通过 `codegraph_init` 工具（非只读，经 guardian 审查）自助初始化。连接成功但可用性标记为 `NotInitialized`，用于引导层。
- `Dismissed`：剔除出连接（用户已放弃），静默。
- `Ready`：正常连接，派生 `Connected`/`ConnectionError`。

### D3: 按项目免打扰持久化

在 `IntegrationsConfig` (`src/config/services.rs:79`) 新增子结构：

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CodegraphSettings {
    /// 已对该 working_dir 关闭安装/初始化引导（规范化绝对路径，去重）
    #[serde(default)]
    pub dismissed_paths: Vec<PathBuf>,
}
```

`IntegrationsConfig` 加 `#[serde(default)] pub codegraph: CodegraphSettings`。`#[serde(default)]` 保证旧 settings.json 向后兼容。

判定逻辑：启动预探测用 `std::fs::canonicalize(working_dir)` 规范化后做包含检查；canonicalize 失败时回退原始路径字符串比较。

### D4: CLI 启动通知（非交互）

在 REPL 启动路径（`cli/args.rs` 的 `run_repl`，MCP connect 完成后）调用 `codegraph::print_availability_notice(availability)`：
- `Connected` / `Dismissed` → 静默
- `NotInstalled` → `⚠ CodeGraph 未安装，代码导航已降级到 grep/lsp。安装: npm i -g @colbymchenry/codegraph`
- `NotInitialized` → `⚠ CodeGraph 已安装但当前仓库未初始化。在项目根运行: codegraph init`
- `ConnectionError(msg)` → `⚠ CodeGraph 连接失败 ({msg})，已降级到 grep/lsp`

通知写入 stderr（不污染 stdout 管道）。`query` 单次模式同样打印；daemon 模式改为 `tracing::warn!` 日志（无终端）。通知受 `Dismissed` 抑制。

### D5: Prompt 注入实时状态

在 prompt 组装的 environment 层（动态上下文）注入一行 codegraph 可用性结论，让 agent 拿到**具体状态**而非泛化指令：

```
CodeGraph status: not_initialized (binary installed; run `codegraph init` to enable; grep/lsp fallback active)
```

并补充 agent 行为指令（替换 `base.md:147` 的泛化文案）：
- `NotInstalled`/`NotInitialized` 且非 `Dismissed`：当**即将进行代码导航**（需调 `codegraph_node`/`codegraph_explore`）时，先用 `ask_user_question` 提供「立即安装/初始化 | 不再提示 | 本次跳过」三选。
  - 选「立即安装/初始化」：口播命令，经用户批准后用 `exec_command` 执行；成功后提示重连（`/mcp restart`）。
  - 选「不再提示」：调用 `dismiss_codegraph_guidance` 工具（见 D6）持久化，此后静默。
  - 选「本次跳过」：本次回退 grep/lsp，不持久化。
- `Connected`：照常优先 codegraph。
- `Dismissed`：不询问，直接 grep/lsp 回退。

### D6: `dismiss_codegraph_guidance` 元工具

为让 agent 可靠写入免打扰状态（`config set` 对数组字段不友好），新增一个轻量元工具（归 meta 类，类似 `update_plan`/`note_edit`）：

- `name()`: `dismiss_codegraph_guidance`
- `is_read_only()`: `false`（修改 settings.json）
- 行为：取当前 working_dir，canonicalize，去重追加到 `settings.integrations.codegraph.dismissed_paths`，保存 settings.json，返回确认。
- input_schema: `{ "path": { "type": "string", "description": "可选，默认当前 working_dir" } }`

替代方案（不采用）：扩展 `config set` 支持数组 append——改动面更大且 `config set` 语义是单键值，引入数组语义不一致。元工具更内聚、可单测。

### D7: TUI 状态栏指示器升级

`detect_codegraph_status()` (`tui/app/mod.rs:974`) 改为返回 `CodegraphAvailability`（而非读配置 status 字段）。`codegraph_status_span()` (`render.rs:312`) 增加映射：

| Availability | 图标 | 颜色 |
|---|---|---|
| Connected | ● | SUCCESS |
| NotInstalled | ⚠ | WARNING |
| NotInitialized | ⚠ | WARNING |
| ConnectionError | ✗ | ERROR |
| Dismissed | ○ | DIM |

`App.codegraph_status` 字段类型随之改为 `CodegraphAvailability`，`event.rs:364` 的刷新点同步更新。

## Edge Cases

- **NotInstalled + 用户已 Dismissed**：`Dismissed` 优先（预探测第 1 步先判），全静默、不 spawn。
- **会话中途用户手动 `codegraph init`**：可用性在启动期探测并缓存，mid-session 不会自动刷新 prompt 注入。用户可 `/mcp restart` 触发重连+重探测；下次会话自然刷新。可接受。
- **working_dir 不存在/canonicalize 失败**：回退原始路径字符串做 dismissed 比较与 `.codegraph/` 拼接。
- **二进制在 PATH 但损坏/不可执行**：`which::which` 成功（找到路径）→ 预探测 `Ready` → spawn 失败 → `ConnectionError(msg)`。正确归类。
- **`.codegraph/` 存在但空/损坏**：预探测 `Ready`（目录存在即视为已初始化），运行时查询失败由 `base.md` 回退 grep/lsp 兜底。索引完整性校验超出范围。
- **非交互模式**：`query` 模式通知打印到 stderr；daemon 模式走 `tracing::warn!`；agent 的 `ask_user_question` 在无交互上下文时跳过（直接回退 grep/lsp）。
- **dismissed_paths 膨胀**：长期累积多个项目路径；设计为简单 Vec，去重保证不重复。若未来需清理，可加 `config` 子命令，非本期范围。

## Components Touched

| File | Change |
|------|--------|
| `src/mcp/codegraph.rs` (new) | `CodegraphAvailability` 枚举、`probe_install_state()`、`print_availability_notice()`、可用性派生逻辑 |
| `src/mcp/mod.rs` | 声明 `pub mod codegraph;`；`connect_configured_tools` 插入预探测，`NotInstalled`/`Dismissed` 短路剔除，`NotInitialized` 仍连接 |
| `src/config/services.rs` | `IntegrationsConfig` 加 `codegraph: CodegraphSettings`；新增 `CodegraphSettings { dismissed_paths }` |
| `src/cli/args.rs` | `run_repl` MCP connect 后调用 `print_availability_notice` |
| `src/prompts/base.md` | `:147` 泛化文案替换为引用实时注入状态 + D5 的 ask_user_question 行为指令 |
| `src/prompts/` environment 层组装处 | 注入 `CodeGraph status: <state>` 动态行 |
| `src/tui/app/mod.rs` | `App.codegraph_status` 改类型；`detect_codegraph_status` 改返 `CodegraphAvailability` |
| `src/tui/app/render.rs` | `codegraph_status_span` 增加 ⚠/DIM 映射 |
| `src/tui/app/event.rs` | `:364` 刷新点类型同步 |
| `src/tools/meta/` (new tool) | `dismiss_codegraph_guidance` 元工具 + ToolRegistry 注册 |
| `tests/` | `probe_install_state` 分类单测（NotInstalled/NotInitialized/Dismissed/Ready）；通知在 Dismissed 下静默；dismiss 工具去重写入 |

## Resolved Questions

- **D6 写免打扰机制**：采用 `dismiss_codegraph_guidance` 元工具（评审确认）。理由：内聚、可单测，不引入 `config set` 的数组语义复杂度。
