# Tasks: daemon-session-orchestration

> 前提：server-side loop（run_session_turn/RunRegistry/SessionEventHub）已随 feature/web-ui-redesign 合并落地。本 change 只做可靠性缺口补强。

## 1. 事件流重放与失步信号

- [x] 1.1 per-session 定长环形事件缓冲（容量可配置，默认千级）
- [x] 1.2 `GET /sessions/:id/events?after=<seq>`：缓冲重放 + 接入实时流
- [x] 1.3 失步信号：seq 淘汰与 Lagged 时向客户端发送 SyncLost，定义客户端全量恢复约定

## 2. 全局事件总线

- [x] 2.1 全局事件类型与序号空间定义（todos/task-group/背景结果/模式/模型变更）
- [x] 2.2 `GET /events` 全局 SSE 端点，多订阅者 fan-out
- [x] 2.3 背景任务结果改广播 + 可查询保留（废除 drain 抢占），轮询端点保留兼容
- [x] 2.4 dogfood：TUI todos 面板切换为 `GET /events` 订阅（保留轮询回退）

## 3. 审批语义收敛

- [x] 3.1 重复应答统一 409（interaction resolve、subagent resolve-permission）
- [x] 3.2 清理 handlers.rs 约 10 处 `"default"` 硬编码，server-side 路径归属真实 session（旧端点兼容映射保留）

## 4. 会话存储版本化

- [x] 4.1 `Session` 增加 version 字段，历史会话按 version=0 兼容
- [x] 4.2 覆盖写携带期望版本，冲突返回 409 + 当前版本；run 写盘推进版本

## 5. daemon 可发现部署

- [x] 5.1 全局发现文件 `~/.wgenty-code/daemon.json`（端口/token/pid/心跳 30s 更新、120s 过期），原子写入 + 退出清理
- [x] 5.2 `utils::discover_daemon()` 复用逻辑：读文件 → token 匹配 + 心跳未过期 → 复用或回退拉起；TUI 启动接入验证

## 6. 验证

- [ ] 6.1 验收：断线按 after=seq 续传；缓冲淘汰/Lagged 收到失步信号并正确回退全量恢复
- [ ] 6.2 验收：todos/模式/模型变更推送到达全部订阅者；背景结果多端可见且离线可查
- [ ] 6.3 验收：重复审批应答 409；并发会话写冲突 409；多会话审批隔离
- [ ] 6.4 验收：两个 UI 经发现文件复用同一 daemon；失效发现文件不误连
- [ ] 6.5 TUI 双模式回归 + 现有 daemon/session 测试套件通过
