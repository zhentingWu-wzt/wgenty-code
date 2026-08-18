# Tasks: daemon-hosted-web-ui

## 1. Daemon 静态托管基础（Rust）

- [x] 1.1 新增 `build.rs`：编译前确保 `web/dist/` 存在（`create_dir_all` + `.gitkeep` 占位，纯 std，不依赖 Node）
- [x] 1.2 `Cargo.toml`：确认 `rust-embed` 依赖挂到 `daemon` feature（沿用 `src/knowledge/embedded.rs` 先例）
- [x] 1.3 新增 `src/daemon/web_ui.rs`：`RustEmbed`（`web/dist`）+ 扩展名→MIME 映射 + `GET /` 与 `GET /assets/*` 处理器（`/assets/*` immutable 长缓存，其余 no-cache）
- [x] 1.4 降级路径：嵌入资产无 `index.html` 时返回 Rust 内联的最小提示页（非 500）
- [x] 1.5 路由接线（`routes.rs`/`mod.rs`）：静态路由并入 public（health）路由组；最终 app 设置 `/api/` 前缀感知的 fallback（API 未匹配 → JSON 404；其余 GET → `index.html`）

## 2. 同源 Bootstrap 认证端点（Rust）

- [x] 2.1 `GET /auth/bootstrap` 处理器：同源判定（`Origin` host 白名单 + `Sec-Fetch-Site ∈ {same-origin, none}` + `Host` 校验），返回 `{ token }`，`Cache-Control: no-store`，跨源 403
- [x] 2.2 bootstrap 单元/边界测试：同源放行、跨源 Origin 拒绝、DNS-rebinding Host 拒绝、无 token 调 API 仍 401

## 3. Daemon 测试与启动日志（Rust）

- [x] 3.1 托管边界测试：index 可达、未知 `/api/v1/*` 返回 404 JSON 非 HTML、SPA 深链 fallback、缓存头正确、dist 为空时降级页
- [x] 3.2 启动日志：正常打印 `Web UI: http://127.0.0.1:<port>`；降级打印未打包提示

## 4. Web 客户端适配（TS）

- [x] 4.1 `resolveDaemonDirect` 扩展（`web/src/api/client.ts`）：`/__daemon-info` 不可用时尝试同源 `GET /auth/bootstrap`，成功返回 `{ base: origin + "/api/v1", token }`；失败返回 null（dev/旧 daemon 行为不变）
- [x] 4.2 `DaemonClient` 受保护调用附加凭证：引入内部 authed-fetch 包装，token 可用时统一注入 `Authorization`（`this.base` 相对路径调用全量覆盖；`fetchStream`/wsChannel 经 4.1 自动获得）
- [x] 4.3 vitest：bootstrap 解析回退链、authed 头注入、无 bootstrap 时零行为变化

## 5. 构建流水线与文档

- [x] 5.1 发布脚本（如有）增加 `npm --prefix web run build` 预构建步骤；确认 `.gitignore` 对 `web/dist` 的处理
- [ ] 5.2 更新 `web/README.md`（daemon-hosted 运行方式）与根 README（如涉及）

## 6. 端到端验证

- [ ] 6.1 手动 E2E：`npm --prefix web run build` → `cargo run --features daemon -- daemon` → 浏览器打开 daemon 地址完成流式对话、权限弹窗、会话列表
- [ ] 6.2 降级启动：无 dist 时 daemon 正常、API 可用、日志提示未打包
- [ ] 6.3 dev 回归：`npm run dev` 工作流不受影响
