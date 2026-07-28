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
