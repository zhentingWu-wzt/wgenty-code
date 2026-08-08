# Tasks: gui-desktop-foundation

> 前置依赖：daemon-session-orchestration 完成（提供编排命令端点与会话事件流）。

## 1. 会话编排客户端（复用 web/）

- [x] 1.1 复用 web/ 的 `DaemonClient`（命令通道：发起 turn / 中断 / 审批应答）—— spike 已验证 webview 内直接可用
- [x] 1.2 复用 web/ 的 SSE 订阅（seq 跟踪、断线续传、失步回退由 daemon-session-orchestration + web/ sessionRunner 实现）—— spike 已验证流式渲染正常
- [x] 1.3 daemon 发现与连接：Tauri Rust 侧发现机制（常驻实例优先，spawn 兜底）—— daemon_manager.rs 实现 discovery + Command spawn + health 轮询，OnceCell 防 StrictMode 双调
- [x] 1.4 修复 daemon::run 的 token/discovery 写入竞态：bind 端口成功后再写 token/discovery 文件（原先 bind 前写，端口占用时 bind 失败退出但 token 已覆盖在用 daemon 的 token，导致全端鉴权失败；同一 bug 也导致 release build 的 discovery writer 首次写入被进程退出打断，daemon.json 不生成）

## 2. GUI 应用骨架（Tauri 壳）

- [x] 2.1 创建 Tauri 2.0 壳项目（`desktop/src-tauri/`，独立 crate），token 注入 + CORS 已由 spike 验证 —— 见 `desktop/README.md`
- [x] 2.2 Tauri 主窗口入口（`desktop/src-tauri/src/lib.rs`），窗口启动与关闭时的资源清理
- [ ] 2.3 platform/ Adapter 层：web/ 加薄抽象（`web/src/platform/`），desktop/ 提供 Tauri 实现，隔离浏览器与桌面特有能力
- [x] 2.4 连接失败的错误展示与重试机制（复用 web/ health 轮询 + disconnected toast + ensureDaemon 失败时桌面端显示 spawn 错误）
- [x] 2.5 导航 + 多面板布局（复用 web/ 的 LeftSidebar + RightRail + SessionTabBar 布局，webview 内直接渲染）

## 3. 核心对话界面（复用 web/）

- [x] 3.1 事件流驱动的对话渲染（复用 web/ 的 ChatView + sessionRunner，spike 已验证流式渲染正常）
- [x] 3.2 中断命令支持（复用 web/ 的 stopSessionTurn）
- [x] 3.3 markdown 渲染（复用 web/ 的 react-markdown + remark-gfm）与代码块语法高亮（复用 shiki）—— spike 已验证
- [x] 3.4 工具调用展示（复用 web/ 的 ToolCallCard 可折叠卡片）
- [x] 3.5 权限审批界面（复用 web/ 的 PermissionModal + usePermissionTrace）
- [x] 3.6 多行输入区（复用 web/ 的 Composer，快捷键提交 + 409 语义）

## 4. 验证

- [ ] 4.1 端到端验收：打开窗口完成一轮含工具调用与权限审批的完整对话（对应 spec 场景）
- [ ] 4.2 多屏同步验收：GUI 与 TUI 连接同一 daemon 同一会话，内容与状态一致；他端发起的 turn/审批在本端正确呈现
- [ ] 4.3 断线恢复验收：重连续传与失步回退符合 spec 场景
- [ ] 4.4 性能验证：默认构建启动时间/内存/二进制大小满足 AGENTS.md 约束；GUI 构建单独记录
- [ ] 4.5 跨平台编译验证：linux/macos/windows
