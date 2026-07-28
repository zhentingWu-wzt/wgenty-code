# Design: add-multi-provider-login

## Context

wgenty-code 当前通过两条路径获取 AI 供应商凭据：
1. 环境变量：`ANTHROPIC_API_KEY` / `DASHSCOPE_API_KEY` / `DEEPSEEK_API_KEY`（见 `src/config/api_config.rs:41-47` `ApiConfig::get_api_key`，顺序为 ANTHROPIC > DASHSCOPE > DEEPSEEK）。
2. `settings.json` 中 `models.{main,small,planner}.api_key` 明文字段（`ModelEndpoint::api_key`）。

`ApiConfig::get_api_key()` 是 **provider-agnostic** 的（不论实际用哪个 provider，都先查 Anthropic 环境变量）。实际 provider 路由由 `models.rs` 的 `resolve_provider()` 依据 `base_url` 判定。CLI 命令定义于 `src/cli/mod.rs:57` `Commands` 枚举，已有 `Config` / `Mcp` / `Plugin` 等子命令模式可参照。

**Brainstorming 确认的范围**：登录 = 向大模型供应商认证以调用其 API。技术现实是「订阅账号登录省去 API key」**仅对提供 OAuth-for-API 的供应商成立**（如 GitHub Copilot）；key 型供应商（Kimi/Moonshot、OpenAI、Anthropic、DeepSeek、DashScope）的聊天订阅与 API 平台是两套账号，无原生 OAuth 把订阅转为 API 访问。经与用户确认，采用 **仅 OAuth + 手动 key** 方案（放弃浏览器辅助取 key 与托管中台）。OAuth 设计为**可扩展**的账号登录路径，Copilot 为首个实现，未来可加 Google/Azure 等 OAuth 供应商。

本设计新增 `auth/` 模块，提供多供应商登录、keychain 安全存储、OAuth device flow 与 provider 感知的凭据解析链路，并将 keychain 作为凭据源插入解析优先级，保持 env var 优先、零破坏。

## Goals / Non-Goals

**Goals**
- 交互式 `login [provider]` 命令，支持两种登录方式：API key 交互输入（key 型供应商）、OAuth device flow（OAuth 型供应商）
- OAuth 作为**可扩展**的账号登录路径，首期支持 GitHub Copilot，trait 设计支持未来加 Google / Azure 等
- 凭据经 OS keychain 安全存储，跨 macOS / Windows / Linux
- OAuth token 自动刷新，无需重新登录
- 凭据解析优先级 env var > keychain > settings.json，向后兼容
- `logout` / `whoami` / `auth list` 完整 CLI 体验
- `auth` feature flag 控制，关闭时不引入 keyring 依赖

**Non-Goals**
- 不做多账号/多 profile 切换（留待后续 change）
- 不改动 daemon / web-ops-console 的用户鉴权（独立 change）
- **不做浏览器辅助取 key**（脆弱、供应商改 UI 即失效，已确认放弃）
- **不做托管账号中台**（需云端后端，远超 CLI change 范围，已确认放弃）
- 不为 key 型供应商（Kimi/OpenAI/Anthropic/DeepSeek/DashScope）引入 OAuth（其 API 用 key 鉴权，无 OAuth-for-API）
- 不迁移或删除现有 settings.json `api_key` 字段

## Decisions

### D1: 凭据存储后端选用 `keyring` crate

**Rationale**: `keyring` 是 Rust 生态成熟的多后端 keychain 抽象，统一封装 macOS Keychain、Windows Credential Manager、Linux Secret Service（libsecret/D-Bus），与本项目跨平台目标一致，避免为每个平台手写 FFI。

**Alternatives**:
- 手写平台 FFI：维护成本高、易出错，放弃。
- 仅加密文件：不依赖系统 keychain，但需自管主密钥派生与防 dump，安全性弱于 OS keychain，作为降级方案保留（见 D2）。

### D2: 无 keychain 时的降级策略

**Rationale**: Linux 容器 / 无 D-Bus / headless 环境常无 Secret Service。策略：检测到 keychain 后端不可用时，**降级到本地加密文件**（`~/.wgenty-code/credentials.enc`，AES-256-GCM），并**明确告知用户**当前存储方式与风险；用户可显式拒绝降级（`--no-fallback` 标志）仅用 keychain。

**主密钥派生（OQ2 已定）**：采用**双因子**--默认机器绑定（hostname + OS install UUID 派生，用户无感），首次 `login` 可用 `--password` 启用口令保护（口令 + salt 经 Argon2id 派生，安全性更高）。两条路径统一为「机器标识 (+ 可选口令)」派生主密钥。

**Alternatives**:
- 直接拒绝登录：安全性最高，但 CI/容器场景完全不可用，体验差。
- 静默回退明文 settings.json：违反「不明文落盘」目标，且 spec 要求不得静默回退，否决。

### D3: OAuth device flow 实现（可扩展账号登录）

**Rationale**: OAuth 是「订阅账号登录」的干净实现路径。**手写轻量 device flow 客户端**（基于现有 `reqwest`），定义 `OAuthDeviceFlow` trait（请求 device code / 轮询 token / 刷新），GitHub Copilot 为首个实现，未来可加 Google（Gemini OAuth）/ Azure（Entra ID）等 OAuth 供应商。避免引入 `oauth2` crate 全量抽象与额外编译开销（与项目「保持二进制精简」约束一致）。

**Copilot client_id 来源**：GitHub Copilot 的 VS Code 扩展使用公开 client_id（`Iv1.b507a08c87ecfe98`）。**内置该公开 client_id**（OAuth 公共客户端模式，无 client_secret），与官方扩展一致。

**供应商分类**（登录方式）：
| 供应商 | 登录方式 | 说明 |
|--------|---------|------|
| `copilot` | OAuth device flow | 订阅账号登录，token 即 API 访问凭据 |
| `anthropic` / `openai` / `deepseek` / `dashscope` / `moonshot` / `custom` | 手动 API key | 无 OAuth-for-API，交互输入 key |

**Alternatives**:
- 引入 `oauth2` crate：功能全但编译体积与抽象成本不匹配本场景，否决。
- 浏览器辅助取 key（覆盖 key 型供应商的"账号登录"）：脆弱、安全敏感，已确认放弃。

> **Open Question OQ1**：内置 Copilot public client_id 合规性需在 verify 阶段复核（属公开客户端，合法，但仍需确认）。

### D4: 凭据数据模型

每个 provider 在 keychain 中存储一条记录（service = `wgenty-code`, username = `<provider>`，命名方案见 OQ5），值为 JSON：

```rust
enum StoredCredential {
    ApiKey { key: String, base_url: Option<String> },       // custom/moonshot 等可带 base_url
    OAuth  { access_token: String, refresh_token: String,
             expires_at: DateTime<Utc>, token_type: String },
}
```

**Rationale**: 单条 JSON 记录便于扩展（未来多账号可改为 username = `<provider>:<profile>`），且 keychain API 本就是「service+username -> 单值」模型。

### D5: provider 感知的凭据解析集成点

**Rationale**: 现有 `ApiConfig::get_api_key()` 不感知 provider，无法表达「该 provider 的 keychain 凭据」。新增 `CredentialResolver`：

```rust
impl CredentialResolver {
    fn resolve(&self, provider: Provider) -> Option<ResolvedCredential> {
        // 1. 该 provider 对应的 env var（如 anthropic -> ANTHROPIC_API_KEY）
        // 2. keychain 中该 provider 的登录凭据（含 token 惰性刷新）
        // 3. settings.json ModelEndpoint::api_key
    }
}
```

在 **API client 构造路径**（`ApiClient::new` / `ModelEndpoint` 解析处）调用 `resolve()`，替换原 `get_api_key()` 调用。`ApiConfig::get_api_key()` 保留但标记 deprecated，内部委托 resolver 保持向后兼容。

**env var 映射（OQ4 已定）**：新增 `OPENAI_API_KEY` 支持；完整映射：`anthropic`->`ANTHROPIC_API_KEY`、`openai`->`OPENAI_API_KEY`、`deepseek`->`DEEPSEEK_API_KEY`、`dashscope`->`DASHSCOPE_API_KEY`、`moonshot`->`MOONSHOT_API_KEY`。映射表集中维护。

### D6: Token 刷新策略 = 惰性按需

**Rationale**: 不引入后台刷新任务（增加运行时复杂度与状态）。在 `CredentialResolver::resolve()` 中检测 OAuth 凭据 `expires_at`，过期则同步用 refresh token 换新令牌、更新 keychain、返回新 token。刷新失败返回错误并提示重新 `login`。

**Alternatives**:
- 后台定时刷新：需管理任务生命周期与并发写 keychain，复杂度高，否决。
- 不自动刷新，过期即报错：体验差，否决。

### D7: `auth` feature flag 与命令降级

**Rationale**: 遵循 AGENTS.md「新功能通过 feature flag 控制」约定。`auth` feature 默认开启（含 `keyring` 依赖）；`cargo build --no-default-features` 时 `auth` 关闭，`login`/`logout`/`whoami`/`auth` 子命令注册为「桩版本」，执行时打印「登录能力未编译，请启用 auth feature」并退出码 2，不 panic、不引入 keyring。

### D8: CLI 命令结构

在 `Commands` 枚举（`cli/mod.rs:57`）新增：
- `Login { provider: Option<ProviderArg>, password: Option<bool> }`（`--password` 启用降级口令保护）
- `Logout { provider: Option<ProviderArg> }`
- `Whoami`（别名 `auth status`：新增 `Auth { action: AuthCommands }`，`AuthCommands::Status`）
- `Auth { action: AuthCommands }`，`AuthCommands = Status | List`

**logout 默认行为（OQ3 已定）**：未指定 provider 时进入**交互式选择**已登录供应商（而非直接清除全部），避免误删。

**Rationale**: `whoami` 作为顶层便捷别名，`auth` 作为命名空间收纳 `status`/`list`，与现有 `Config`/`Mcp` 子命令模式一致。

### D9: 凭据脱敏

**Rationale**: 所有面向用户的输出（whoami / login 确认 / 日志）对凭据脱敏：仅显示前缀 + 末 4 位（如 `sk-***…abcd`），完整值永不出现在终端或日志。OAuth token 同理脱敏。

### D10: 范围取舍--仅 OAuth + 手动 key（brainstorming 确认）

**Rationale**: 经 brainstorming 确认，"订阅账号登录省去 API key" 仅对 OAuth-for-API 供应商成立。采用 **仅 OAuth + 手动 key**：
- OAuth 供应商（Copilot，及未来 Google/Azure）-> device flow 账号登录
- key 型供应商（含 Kimi/Moonshot）-> 手动 API key（接受其无账号登录的取舍）

**放弃的方案**：
- 浏览器辅助取 key：脆弱（供应商改 UI 即失效）、安全敏感，放弃。
- 托管账号中台：需云端后端，远超 CLI change 范围，放弃。

## Risks / Trade-offs / Open Questions

- **风险 R1（跨平台 keychain 可用性）**：Linux 无 Secret Service 时需降级（D2）。缓解：加密文件降级 + 明确提示 + `--no-fallback`。
- **风险 R2（Copilot OAuth 端点稳定性）**：GitHub 可能调整 device flow 端点或 client_id 策略。缓解：端点与 client_id 集中常量化，便于后续更新。
- **风险 R3（安全敏感）**：涉及凭据存储与 OAuth，属 guardian/安全审查重点。缓解：脱敏（D9）、不明文落盘、token 惰性刷新限定在 resolver 内部。
- **权衡 T1**：`keyring` 新增依赖会轻微增加二进制体积，但 `auth` feature 可关闭以保持精简构建。
- **权衡 T2（key 型供应商无账号登录）**：Kimi 等 key 型供应商只能手动配 key，无法"订阅账号登录"。这是技术现实下的接受取舍（D10），通过 `moonshot` 命名 provider + 交互输入降低配置门槛。
- **Open Questions（剩余）**:
  - **OQ1（待 verify）**: 内置 Copilot public client_id 合规复核（verify 阶段）。
  - OQ2-OQ5 已在本设计中定稿（见 D2/D5/D8/OQ5）。
- **OQ5（已定）**: keychain 命名方案 = service `wgenty-code`，username `<provider>`。

## Migration Plan

- **零迁移**：现有 env var 与 settings.json `api_key` 用户无需任何改动即继续工作（解析优先级向后兼容，见 spec「Credential Resolution Priority」）。
- `login` 为增量能力：用户主动执行才启用 keychain 凭据。
- `ApiConfig::get_api_key()` 保留并委托 resolver，避免破坏外部调用方（若有）。
- 文档：QUICKSTART.md / README 增补 `login` 命令说明；WGENTY.md 配置表补 `auth` feature 与登录命令、新增 `MOONSHOT_API_KEY` / `OPENAI_API_KEY`。

## References

- 现有凭据解析：`src/config/api_config.rs:41-47`
- provider 路由：`src/config/models.rs` `resolve_provider()`
- CLI 命令枚举：`src/cli/mod.rs:57`
- AGENTS.md feature flag 与安全模块约定
- Brainstorming 取舍记录：见 D10（仅 OAuth + 手动 key）
