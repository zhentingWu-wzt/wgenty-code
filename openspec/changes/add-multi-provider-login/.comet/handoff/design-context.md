# Comet Design Handoff

- Change: add-multi-provider-login
- Phase: design
- Mode: compact
- Context hash: 94445f2c6c0a170f1281cd945cf8fb74d7ef531feaf98047bcf0c8cfb520887e

Generated-by: comet-handoff.sh

OpenSpec remains the canonical capability spec. This handoff is a deterministic, source-traceable context pack, not an agent-authored summary.

## openspec/changes/add-multi-provider-login/proposal.md

- Source: openspec/changes/add-multi-provider-login/proposal.md
- Lines: 1-32
- SHA256: 278c9faff8e97d008ff4e3a1bff898945a82e35226687d2be1a4a3e25e693802

```md
## Why

当前 wgenty-code 的 AI 供应商凭据只能通过环境变量（`ANTHROPIC_API_KEY` / `DASHSCOPE_API_KEY` / `DEEPSEEK_API_KEY`）或 `settings.json` 明文 `api_key` 配置。这带来三个问题：① 缺少统一登录入口，新用户配置门槛高；② 凭据明文落盘，存在安全风险；③ 无法支持 OAuth 类供应商（如 GitHub Copilot）。需要一个多供应商登录功能，提供交互式登录、安全凭据存储与统一解析链路，同时零破坏现有 env var 工作流。

## What Changes

- 新增 `wgenty-code login [provider]` 命令：交互式选择或直接指定供应商，支持 API key 交互输入与 OAuth device flow 两种登录方式
- 新增凭据安全存储：经 OS keychain（macOS Keychain / Windows Credential Manager / Linux secret-service）保存，无 keychain 环境降级
- 新增 OAuth device flow 登录与 token 自动刷新，首期支持 GitHub Copilot
- 新增 `logout [provider]`、`whoami`（即 `auth status`）、`auth list` 子命令
- 凭据解析优先级调整为：**env var > keychain 登录凭据 > settings.json api_key**（向后兼容，env var 仍可临时覆盖）
- 供应商范围：Anthropic / OpenAI / DeepSeek / DashScope / 自定义 OpenAI 兼容端点 + GitHub Copilot（OAuth）

## Capabilities

### New Capabilities

- `provider-auth`: 多供应商凭据登录能力。涵盖 keychain 安全存储、API key 交互登录、OAuth device flow 与 token 刷新、凭据解析优先级，以及 `login` / `logout` / `whoami` / `auth list` CLI 命令。

### Modified Capabilities

<!-- 无现有 spec 涉及凭据存储或认证行为；凭据解析链路当前仅为实现细节，无 spec 级契约，故全部作为新增 capability 处理。 -->

## Impact

- **新增模块** `auth/`：keychain 凭据存储、OAuth device flow、token 刷新调度、凭据解析器
- **修改** `config/`：`ApiConfig` / `Settings` 凭据读取链路插入 keychain 源（env var > keychain > settings.json）
- **修改** `cli/`：新增 `login` / `logout` / `whoami` / `auth` 子命令解析与交互流程
- **新增依赖**：`keyring` crate（跨平台 keychain 访问）；OAuth device flow 视实现选用 `oauth2` 或手写 device flow 客户端
- **Feature flag**：新增 `auth` feature（默认开启 keychain 路径，按 AGENTS.md 约定保持默认构建精简可控）
- **跨平台**：macOS / Linux / Windows keychain 适配，无 keychain 时降级到加密文件或拒绝登录（design 阶段决策）
- **安全敏感**：涉及凭据存储与 OAuth token 生命周期，需在 guardian / 安全审查中重点关注
```

## openspec/changes/add-multi-provider-login/design.md

- Source: openspec/changes/add-multi-provider-login/design.md
- Lines: 1-167
- SHA256: 0f0b63e70ffca9742fb5cc1a8bd6c6a3f71cf6d0a95c5c7f082d3fdf7b917159

[TRUNCATED]

```md
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
```

Full source: openspec/changes/add-multi-provider-login/design.md

## openspec/changes/add-multi-provider-login/tasks.md

- Source: openspec/changes/add-multi-provider-login/tasks.md
- Lines: 1-75
- SHA256: 73b22a19ababd04f040d14ef9d3429c8d549ab4f5b8ed838ad61d50e43a6b1bc

```md
# Tasks: add-multi-provider-login

> 分两阶段：P1（1-5 组）为核心 API key + keychain + 凭据解析；P2（6-7 组）为 OAuth device flow + token 刷新。8-9 组为测试与文档收尾。

## 1. 基础设施与 Feature Flag

- [ ] 1.1 在 `Cargo.toml` 新增 `auth` feature（默认开启），并在 `auth` 下声明 `keyring` 依赖
- [ ] 1.2 创建 `src/auth/` 模块骨架（`mod.rs` 导出公共类型），在 `lib.rs`/`main.rs` 按 feature 条件注册
- [ ] 1.3 定义 `Provider` 枚举（`Anthropic`/`Openai`/`Deepseek`/`Dashscope`/`Moonshot`/`Custom`/`Copilot`）与 `ProviderArg` clap 解析（小写不敏感）
- [ ] 1.4 定义 provider -> env var 映射表，新增 `OPENAI_API_KEY` 与 `MOONSHOT_API_KEY` 支持（OQ4 已定）

## 2. 凭据存储后端

- [ ] 2.1 定义 `StoredCredential` 枚举（`ApiKey` / `OAuth`）及其 serde 序列化（见 design D4）
- [ ] 2.2 实现 `KeychainStore`：基于 `keyring` crate 的 get/set/delete（service=`wgenty-code`, name=`<provider>`，OQ5）
- [ ] 2.3 实现后端可用性探测，无 keychain 时按 D2 降级到加密文件（AES-256-GCM）
- [ ] 2.4 实现加密文件降级路径（`~/.wgenty-code/credentials.enc`，主密钥派生见 OQ2）
- [ ] 2.5 实现 `--no-fallback` 标志：拒绝降级，仅用 keychain，不可用时明确报错
- [ ] 2.6 抽象 `CredentialStore` trait，keychain 与加密文件为两个实现

## 3. Provider 感知凭据解析

- [ ] 3.1 实现 `CredentialResolver::resolve(provider)`：env var > keychain > settings.json（design D5）
- [ ] 3.2 实现 `ResolvedCredential` 类型，供 `ApiClient` 消费（含 base_url 与 key 或 OAuth token）
- [ ] 3.3 在 API client 构造路径调用 `resolve()`，替换原 `ApiConfig::get_api_key()` 调用
- [ ] 3.4 将 `ApiConfig::get_api_key()` 标记 deprecated，内部委托 resolver 保持向后兼容
- [ ] 3.5 验证向后兼容：env var 与 settings.json `api_key` 用户工作流不受影响

## 4. CLI 登录/登出/状态命令（API key 路径，P1）

- [ ] 4.1 在 `Commands` 枚举新增 `Login`/`Logout`/`Whoami`/`Auth` 变体（design D8）
- [ ] 4.2 实现 `login [provider]`：未指定时交互式选择供应商
- [ ] 4.3 实现 API key 交互输入（关闭回显、非空校验、`custom` 额外提示 base_url）
- [ ] 4.4 实现登录成功脱敏确认输出（design D9）
- [ ] 4.5 实现 `logout [provider]`：移除 keychain 凭据，未登录时正常退出（OQ3 确认默认行为）
- [ ] 4.6 实现 `whoami` / `auth status`：列出各供应商登录状态与生效凭据来源（脱敏）
- [ ] 4.7 实现 `auth list`：列出支持供应商及登录方式（API key / OAuth）与登录状态
- [ ] 4.8 关闭 `auth` feature 时命令桩版本：友好提示 + 退出码 2，不 panic

## 5. 凭据脱敏与安全

- [ ] 5.1 实现凭据脱敏工具（前缀 + 末 4 位，如 `sk-***…abcd`）
- [ ] 5.2 审查所有面向用户输出与日志路径，确保完整凭据永不出现
- [ ] 5.3 在 guardian 安全审查中登记新增 `auth/` 模块与凭据存储操作

## 6. OAuth Device Flow + Copilot（P2）

- [ ] 6.1 定义 `OAuthDeviceFlow` trait（请求 device code / 轮询 token / 刷新）
- [ ] 6.2 实现 Copilot device flow 客户端（内置 public client_id `Iv1.b507a08c87ecfe98`，端点常量化）
- [ ] 6.3 实现 `login copilot`：展示 user_code + 验证 URL，轮询 token 端点，超时处理
- [ ] 6.4 将 OAuth 凭据（access/refresh token + expires_at）写入 keychain
- [ ] 6.5 将 Copilot 作为新 provider 接入 provider 路由（token 作为 Bearer 鉴权）

## 7. Token 刷新（P2）

- [ ] 7.1 在 `CredentialResolver::resolve()` 中检测 OAuth `expires_at`，过期触发惰性刷新（design D6）
- [ ] 7.2 实现用 refresh token 换新 access token，更新 keychain 记录
- [ ] 7.3 refresh token 失效时返回明确错误并提示重新 `login copilot`

## 8. 测试

- [ ] 8.1 `CredentialStore` 单元测试：keychain 与加密文件的 set/get/delete 往返
- [ ] 8.2 `CredentialResolver` 单元测试：env var > keychain > settings.json 优先级各分支
- [ ] 8.3 API key 登录交互流程测试（含非空校验、custom base_url、脱敏输出）
- [ ] 8.4 OAuth device flow 测试（mock 端点：成功 / 超时 / refresh 失败）
- [ ] 8.5 向后兼容回归测试：env var 与 settings.json 用户路径不变
- [ ] 8.6 `auth` feature 关闭时命令桩版本测试（退出码 2、不 panic）
- [ ] 8.7 运行 `cargo test --all`、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --check` 全绿

## 9. 文档与配置

- [ ] 9.1 QUICKSTART.md 增补 `login` / `logout` / `whoami` 命令说明与示例
- [ ] 9.2 README.md / README.zh.md CLI 速览补充登录命令
- [ ] 9.3 WGENTY.md 配置表补充 `auth` feature 与登录相关说明
- [ ] 9.4 验证性能约束：启动时间增量 ≤5%、内存增量 ≤2%、二进制增量 ≤500KB（关闭/开启 auth 两次测量）
```

## openspec/changes/add-multi-provider-login/specs/provider-auth/spec.md

- Source: openspec/changes/add-multi-provider-login/specs/provider-auth/spec.md
- Lines: 1-164
- SHA256: 5666e362a30a5d10bdeca54556e64b6036b3ebad5feab66f6e66ce03dc3a7da9

[TRUNCATED]

```md
# Delta Spec: provider-auth

## ADDED Requirements

### Requirement: Provider Login Command

系统 SHALL 提供 `wgenty-code login [provider]` 命令，用于向 AI 供应商发起登录。
- 未指定 `[provider]` 时 SHALL 进入交互式选择，列出所有支持登录的供应商供用户选择。
- 指定 `[provider]` 时 SHALL 直接进入该供应商的登录流程。
- `[provider]` 取值 SHALL 覆盖：`anthropic`、`openai`、`deepseek`、`dashscope`、`moonshot`（Kimi/Moonshot）、`custom`（自定义 OpenAI 兼容端点）、`copilot`（GitHub Copilot，OAuth）。
- 登录成功后 SHALL 在终端显示确认信息（脱敏，不回显完整凭据）。
- 重复登录同一供应商 SHALL 覆盖该供应商已存储的凭据。

#### Scenario: 交互式选择供应商登录
- **WHEN** 用户执行 `wgenty-code login` 且未指定 provider
- **THEN** 系统列出支持登录的供应商清单（anthropic / openai / deepseek / dashscope / moonshot / custom / copilot）供用户选择，选择后进入对应登录流程

#### Scenario: 直接指定供应商登录
- **WHEN** 用户执行 `wgenty-code login anthropic`
- **THEN** 系统直接进入 Anthropic 的 API key 交互输入流程，不显示供应商选择清单

#### Scenario: 不支持的供应商名称
- **WHEN** 用户执行 `wgenty-code login unknown-provider`
- **THEN** 系统以非零退出码报错，并列出支持的 provider 取值

### Requirement: API Key Interactive Login

对于 API key 类供应商（`anthropic`、`openai`、`deepseek`、`dashscope`、`moonshot`、`custom`），系统 SHALL 通过交互式提示读取 API key。
- 输入过程 SHALL 关闭终端回显（隐藏输入），避免明文泄露。
- `custom` 供应商 SHALL 额外提示输入 `base_url`。
- 系统 SHALL 对输入的 key 做非空校验；空值 SHALL 拒绝并提示重新输入或取消。
- 校验通过后 SHALL 将凭据写入 OS keychain。

#### Scenario: 成功输入并存储 API key
- **WHEN** 用户执行 `wgenty-code login deepseek` 并输入有效（非空）API key
- **THEN** 系统关闭回显读取 key，校验非空后将凭据写入 keychain，并显示脱敏确认（如 `已登录 deepseek (sk-***…abcd)`）

#### Scenario: 自定义供应商需提供 base_url
- **WHEN** 用户执行 `wgenty-code login custom`
- **THEN** 系统 SHALL 依次提示输入 base_url 与 API key，并将两者一并存储

#### Scenario: 空输入被拒绝
- **WHEN** 用户在 API key 提示处直接回车（空值）
- **THEN** 系统提示 key 不能为空，允许重新输入或取消；不写入 keychain

### Requirement: OAuth Device Flow Login

对于 OAuth 类供应商（首期 `copilot`），系统 SHALL 通过 OAuth device flow 完成登录：
- 系统 SHALL 向供应商的 device code 端点请求 device code，并向用户展示用户码（user_code）与验证 URL。
- 系统 SHALL 引导用户在浏览器完成授权，并以轮询方式等待 token 端点返回访问令牌。
- 取得令牌后 SHALL 将 access token、refresh token、过期时间一并写入 keychain。
- OAuth client 凭据（client_id 等）的来源 SHALL 在 design 阶段确定（内置或配置）。

#### Scenario: 成功完成 device flow
- **WHEN** 用户执行 `wgenty-code login copilot` 并在浏览器中完成授权
- **THEN** 系统展示 user_code 与验证 URL，轮询 token 端点，取得令牌后写入 keychain 并显示登录成功

#### Scenario: 用户授权超时
- **WHEN** 用户在 device code 有效期内未完成浏览器授权
- **THEN** 系统 SHALL 报告授权超时并以非零退出码退出，不写入任何凭据

### Requirement: OAuth Token Refresh

系统 SHALL 在 OAuth access token 过期时使用 refresh token 自动刷新，无需用户重新登录。
- 刷新 SHALL 在凭据被实际用于 API 调用前触发（惰性刷新）。
- 刷新成功 SHALL 用新令牌与过期时间更新 keychain 中的记录。
- refresh token 失效或刷新失败时 SHALL 返回明确错误，提示用户重新 `login`。

#### Scenario: 过期 token 自动刷新
- **WHEN** 已登录 copilot 的 access token 已过期且 refresh token 仍有效，系统需要调用 API
- **THEN** 系统 SHALL 自动用 refresh token 换取新 access token，更新 keychain，并继续原 API 调用

#### Scenario: refresh token 失效
- **WHEN** refresh token 已失效导致刷新失败
- **THEN** 系统 SHALL 返回明确错误并提示用户重新执行 `wgenty-code login copilot`

### Requirement: Credential Secure Storage

系统 SHALL 通过 OS keychain 持久化登录凭据：
- macOS SHALL 使用 Keychain，Windows SHALL 使用 Credential Manager，Linux SHALL 使用 Secret Service（libsecret）。
```

Full source: openspec/changes/add-multi-provider-login/specs/provider-auth/spec.md

