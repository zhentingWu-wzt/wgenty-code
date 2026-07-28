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
- 凭据 SHALL 按 `provider` 维度独立存储，互不干扰。
- 当目标平台无可用 keychain 后端时，系统 SHALL 降级（降级策略见 design；至少 SHALL 明确告知用户当前存储方式与风险）。
- 凭据 SHALL 仅在登录/登出/读取解析时访问，不在日志或终端回显完整值。

#### Scenario: 写入与读取 keychain
- **WHEN** 用户完成某供应商登录，随后发起一次 API 调用
- **THEN** 系统 SHALL 从 keychain 读回该供应商凭据用于鉴权，keychain 条目以 provider 标识

#### Scenario: 无 keychain 后端时降级
- **WHEN** 运行环境无可用 keychain（如无 D-Bus/无 Secret Service 的 Linux 容器）
- **THEN** 系统 SHALL 按降级策略处理并明确告知用户存储方式，不得静默回退到明文 settings.json 而不提示

### Requirement: Credential Resolution Priority

系统 SHALL 按以下优先级解析每个供应商的有效凭据：
**环境变量 > keychain 登录凭据 > settings.json `api_key`**
- 环境变量（`ANTHROPIC_API_KEY` / `OPENAI_API_KEY` / `DEEPSEEK_API_KEY` / `DASHSCOPE_API_KEY` / `MOONSHOT_API_KEY`）存在时 SHALL 优先使用，即使已通过 `login` 登录。
- 环境变量缺失时 SHALL 回退到 keychain 登录凭据。
- 两者均缺失时 SHALL 回退到 `settings.json` 的 `api_key`。
- 该优先级 SHALL 向后兼容：未使用 `login` 的现有用户工作流不受影响。

#### Scenario: 环境变量优先于登录凭据
- **WHEN** 用户已 `login anthropic`，但同时设置了 `ANTHROPIC_API_KEY` 环境变量
- **THEN** 系统 SHALL 使用环境变量中的 key 进行 API 鉴权

#### Scenario: 无环境变量时使用登录凭据
- **WHEN** 用户已 `login anthropic` 且未设置 `ANTHROPIC_API_KEY` 环境变量
- **THEN** 系统 SHALL 从 keychain 读取登录凭据进行鉴权

#### Scenario: 未登录时回退到 settings.json
- **WHEN** 用户未执行 `login` 且未设置环境变量，但 settings.json 配置了 `api_key`
- **THEN** 系统 SHALL 使用 settings.json 的 `api_key`（与现有行为一致）

### Requirement: Logout Command

系统 SHALL 提供 `wgenty-code logout [provider]` 命令移除已存储的登录凭据。
- 指定 `[provider]` 时 SHALL 仅移除该供应商的 keychain 凭据。
- 未指定 `[provider]` 时 SHALL 进入交互式选择已登录供应商（不直接清除全部，避免误删）；无可登出项时提示并正常退出。
- 登出 SHALL 不影响环境变量与 settings.json 中的配置。
- 登出未登录的供应商 SHALL 给出明确提示而非报错崩溃。

#### Scenario: 登出指定供应商
- **WHEN** 用户执行 `wgenty-code logout deepseek` 且 deepseek 已登录
- **THEN** 系统 SHALL 从 keychain 移除 deepseek 凭据并显示确认

#### Scenario: 登出未登录的供应商
- **WHEN** 用户执行 `wgenty-code logout openai` 但 openai 从未登录
- **THEN** 系统 SHALL 提示该供应商未登录，正常退出（退出码 0）

### Requirement: Auth Status Command

系统 SHALL 提供 `wgenty-code whoami`（等价 `wgenty-code auth status`）命令，展示当前登录状态。
- 输出 SHALL 列出每个支持供应商的登录状态（已登录 / 未登录）。
- 对已登录供应商 SHALL 显示脱敏标识（不输出完整凭据）。
- 对已登录供应商 SHALL 标注当前生效的凭据来源（env var / keychain / settings.json）。

#### Scenario: 查看登录状态
- **WHEN** 用户执行 `wgenty-code whoami`
- **THEN** 系统以表格或清单形式列出各供应商登录状态与生效凭据来源（脱敏）

### Requirement: Auth List Command

系统 SHALL 提供 `wgenty-code auth list` 命令，列出所有支持登录的供应商及其登录方式（API key / OAuth）。
- 该命令 SHALL 不泄露任何已存储凭据。
- 该命令 SHALL 标注每个供应商当前是否已登录。

#### Scenario: 列出支持的供应商
- **WHEN** 用户执行 `wgenty-code auth list`
- **THEN** 系统列出 anthropic / openai / deepseek / dashscope / moonshot / custom / copilot 及各自登录方式与登录状态，不输出任何凭据明文

### Requirement: Auth Feature Flag

系统 SHALL 通过 `auth` Cargo feature 控制登录能力的编译。
- 默认构建 SHALL 包含 `auth` feature（keychain 路径可用）。
- 关闭 `auth` feature 时 SHALL 不引入 `keyring` 等相关依赖，`login` / `logout` / `whoami` / `auth` 子命令 SHALL 以友好提示告知该能力未编译。
- 该约束 SHALL 满足 AGENTS.md「新功能通过 feature flag 控制以保持默认构建精简」的约定。

#### Scenario: 默认构建包含登录能力
- **WHEN** 以默认 feature 构建（`cargo build --release`）
- **THEN** `wgenty-code login` 命令 SHALL 可用且能访问 keychain

#### Scenario: 关闭 auth feature 时友好降级
- **WHEN** 以 `--no-default-features` 且未启用 `auth` 构建
- **THEN** 执行 `wgenty-code login` SHALL 提示「登录能力未编译，请启用 auth feature」并以非零退出码退出，不 panic
