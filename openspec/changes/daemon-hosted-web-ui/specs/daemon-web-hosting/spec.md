# daemon-web-hosting Specification（Delta）

## Purpose

daemon 将预构建的 Web UI 静态资源在编译时嵌入二进制并直接托管，使浏览器无需 Node/Vite 环境即可通过 daemon 地址使用完整界面；配套同源 bootstrap 认证端点、缓存策略与缺失降级行为。

## ADDED Requirements

### Requirement: 编译时嵌入静态资产

daemon 二进制 SHALL 在编译时嵌入 Web UI 生产构建产物（`web/dist`）。嵌入内容为空（未预构建）时 MUST 触发降级行为，且 MUST NOT 影响 daemon 其余功能。

#### Scenario: 预构建后启动

- **WHEN** `web/dist` 已预构建，daemon 以 daemon feature 编译并启动
- **THEN** 浏览器访问 daemon 根路径返回 Web UI 的 `index.html`，界面完整可用

#### Scenario: dist 缺失降级

- **WHEN** `web/dist` 未构建（嵌入为空），daemon 启动
- **THEN** daemon 正常启动且所有 API 可用，启动日志提示未打包 Web UI，根路径返回明确的提示响应（非 500 崩溃）

### Requirement: SPA fallback

daemon 托管的静态路由 SHALL 对不匹配静态资产与 API 路由的 GET 请求返回 `index.html`（前端路由深链可达）。API 路径（`/api/*`）MUST NOT 被 fallback 覆盖——未匹配的 API 路径返回 404 而非 HTML。

#### Scenario: 深链直达

- **WHEN** 浏览器直接请求一个非资产路径（前端路由地址）或刷新该路径
- **THEN** 返回 `index.html`，前端恢复对应路由视图

#### Scenario: API 404 不降级为 HTML

- **WHEN** 请求一个不存在的 `/api/v1/*` 端点
- **THEN** 返回 404 JSON 错误响应，而非 `index.html`

### Requirement: 同源 bootstrap 认证

daemon SHALL 提供一个 bootstrap 端点，向确认同源的请求返回当前 bearer token；对跨源请求（携带非同源 `Origin` 或 `Sec-Fetch-Site` 指示跨站的 CORS 请求）MUST 拒绝返回 token。托管的静态资产本身 MUST NOT 要求认证。

#### Scenario: 页面启动获取 token

- **WHEN** daemon-hosted 页面加载后以同源请求访问 bootstrap 端点
- **THEN** 返回 bearer token，页面后续 API 调用携带 Authorization 成功通过认证

#### Scenario: 跨源读取被拒

- **WHEN** 外部网页从其自身 origin 以 CORS 方式请求 bootstrap 端点
- **THEN** 请求被拒绝，token 不泄露给跨源页面

#### Scenario: 无 token 调 API 仍 401

- **WHEN** 任何客户端不携带 token 调用受保护 API
- **THEN** 返回 401，认证语义不变

### Requirement: 静态资源缓存策略

`index.html` MUST 以 no-cache 语义响应；带内容 hash 的静态资产 SHOULD 使用长缓存（immutable）。daemon 更换版本重启后，浏览器刷新 MUST 能获取新的 `index.html`。

#### Scenario: 部署更新后生效

- **WHEN** daemon 更换为新版本二进制并重启，浏览器刷新页面
- **THEN** 浏览器取到新 `index.html` 并加载新版本资产，旧缓存不残留

### Requirement: 启动 URL 日志

daemon 启动且托管可用时 SHALL 在启动日志中打印 Web UI 访问 URL。托管默认开启；降级时日志 SHALL 说明原因。

#### Scenario: 启动打印访问地址

- **WHEN** daemon 启动且嵌入资产非空
- **THEN** 启动日志输出形如 `http://127.0.0.1:<port>` 的 Web UI 访问 URL
