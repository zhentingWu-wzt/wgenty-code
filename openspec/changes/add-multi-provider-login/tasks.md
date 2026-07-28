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
