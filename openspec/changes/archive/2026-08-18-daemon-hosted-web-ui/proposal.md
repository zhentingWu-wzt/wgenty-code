# Proposal: daemon-hosted-web-ui

## Why

Web 客户端目前只能通过 Vite dev server 使用（`npm run dev` + 代理注入 token）：浏览器打开的是 5173 端口的 dev server，而非 daemon 本身。这意味着生产/日常使用 Web UI 必须先装 Node 环境并维护一套代理配置。`web-agent-frontend` spec 中预留的 "Optional daemon-hosted production build"（Tier 3, MAY）正是此缺口。让 daemon 直接托管编译好的 Web UI 静态资源，即可实现单二进制开箱即用：`wgenty-code daemon` 启动后浏览器打开 daemon 地址即得完整界面。

## What Changes

- daemon 新增静态资源托管：编译时用 `rust-embed` 将 `web/dist` 嵌入二进制（随 `daemon` feature），运行时从嵌入资产服务 `index.html` + hashed assets，含 SPA fallback
- 新增同源 bootstrap 认证端点（如 `GET /auth/bootstrap`）：仅对确认同源的请求返回 bearer token，页面启动时自动获取——替代 dev 模式下 Vite 代理的 token 注入；跨源网页无法读取
- 静态资源缓存策略：`index.html` no-cache，带内容 hash 的 assets 长缓存（immutable）
- daemon 启动日志打印 Web UI 访问 URL（如 `http://127.0.0.1:8371`）；托管默认开启
- dist 缺失时优雅降级：嵌入空资产目录，daemon 正常启动，日志提示"未打包 Web UI"，API 不受影响
- web 客户端适配生产模式：token 获取路径支持 daemon-hosted（同源 bootstrap）；`resolveDaemonDirect` 直连逻辑在同源场景下退化为同源请求
- cargo build 不依赖 Node：dist 预构建由发布脚本/开发者手动执行（`npm --prefix web run build`），cargo 仅负责嵌入
- **非 BREAKING**：dev 工作流（Vite 代理 + `/__daemon-info`）原样保留；所有现有 API 语义不变

## Capabilities

### New Capabilities

- `daemon-web-hosting`: daemon 托管 Web UI 静态资源 —— 编译时嵌入、SPA fallback、同源 bootstrap 认证端点、缓存头策略、dist 缺失降级、启动 URL 日志

### Modified Capabilities

- `web-agent-frontend`: "Optional daemon-hosted production build" 场景从 MAY 升级为具体要求——daemon-hosted 模式下 token 经同源 bootstrap 端点获取，页面与 API 同源时直连优化退化为同源请求

## Impact

- **daemon（Rust）**：`src/daemon/` 新增静态资源路由与 bootstrap 端点（考虑放在 health/public 路由侧或独立分层）；`Cargo.toml` 增加 `rust-embed` 依赖（daemon feature 下）；启动日志
- **web（TS/React）**：`src/api/client.ts` token 获取逻辑增加 daemon-hosted 分支；`resolveDaemonDirect` 适配
- **构建/发布**：发布脚本增加 `npm --prefix web run build` 预构建步骤；`.gitignore` 处理 `web/dist`
- **文档**：`web/README.md` 运行说明更新
- **安全**：bootstrap 端点必须防御跨源读取（Origin/`Sec-Fetch-Site` 校验），并与全局 CORS（`allow_origin Any`）共存——这是设计阶段的关键权衡点

## Non-goals

- 不改动 Vite dev 工作流（开发体验照旧）
- 不做 LAN/远程访问增强（维持 loopback-only）
- 不做自动打开浏览器
- 不做 build.rs 自动构建 web（cargo build 不依赖 Node）
- 不做移动端适配、subagent 异步权限队列等 web README 中其他 phase-2 项
