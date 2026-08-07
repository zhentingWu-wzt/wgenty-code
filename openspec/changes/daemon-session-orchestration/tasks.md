# Tasks: daemon-session-orchestration

## 1. 事件总线基础设施

- [ ] 1.1 定义 `SessionEvent` / 全局事件类型与序号机制
- [ ] 1.2 per-session broadcast hub + 定长环形重放缓冲（仿 `TRACE_HUB`）
- [ ] 1.3 全局事件 hub（todos/模式/模型/背景任务变更）

## 2. 会话编排层

- [ ] 2.1 TurnRunner：daemon 内复用 `run_agent_loop`，进程内 LlmPort/ToolPort 接线
- [ ] 2.2 turn 命令端点：`POST /sessions/:id/turns`、`POST .../interrupt`、状态查询
- [ ] 2.3 turn 并发互斥（重复发起 409）
- [ ] 2.4 会话存储版本化，冲突写返回 409

## 3. 事件流端点

- [ ] 3.1 `GET /sessions/:id/events?after=<seq>`：重放 + 实时 SSE fan-out
- [ ] 3.2 缓冲淘汰的失步信号与客户端回退约定
- [ ] 3.3 全局事件流端点

## 4. 审批与全局状态修复

- [ ] 4.1 修掉审批 `"default"` 硬编码，归属真实 session
- [ ] 4.2 审批请求/决议事件化，任一客户端应答全局决议一次，重复应答 409
- [ ] 4.3 背景任务结果改广播（去除 drain 抢占语义）

## 5. 部署与兼容

- [ ] 5.1 daemon 常驻模式 + per-working-dir 端口/token 发现文件（含 pid/心跳存活校验）
- [ ] 5.2 UI 连接逻辑：先发现已驻留实例，失败回退进程内拉起
- [ ] 5.3 旧端点兼容回归：TUI 现有模式行为不变

## 6. 验证

- [ ] 6.1 验收：两个客户端订阅同一会话，一个发起 turn，双方看到相同流式输出
- [ ] 6.2 验收：断线重连按 seq 续传；缓冲淘汰后正确回退全量恢复
- [ ] 6.3 验收：任一客户端审批生效、重复应答 409、并发 turn 409、冲突写 409
- [ ] 6.4 TUI 全功能回归 + agent 测试套件通过
