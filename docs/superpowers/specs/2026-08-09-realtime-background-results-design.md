# 实时后台任务结果通知设计

## 目标

后台命令完成后，所属会话无需等待下一条用户消息即可看到结果，并由空闲主 agent 自动消费该结果；不同会话、项目之间绝不串送。

## 现状与问题

`BackgroundManager` 完成命令后，daemon 已将结果保留并发布到全局 SSE 事件总线。然而 TUI 的订阅者只消费 `todos_changed`。后台结果只会在下一次 `AgentLoop::process_input_inner` 调用 `GET /background/results` 时进入历史，因此实时性取决于用户下一轮输入。

结果当前没有 `session_id`，全局保留队列也无法安全地按会话过滤。

## 方案比较

1. 继续轮询结果接口：改动最小，但带来无意义请求和最高 500ms 延迟，也没有利用已有 SSE 总线。
2. 新增专用 SSE endpoint：可以隔离协议，但复制现有全局总线的重连、排序与订阅逻辑。
3. 扩展现有全局 SSE 总线（采用）：结果写入时携带会话归属；TUI 通用订阅者按会话过滤并转换为 UI 事件。保留快照接口用于重连补偿。

## 架构与数据流

1. 任务发起时，`BackgroundManager::spawn` 接收并保存 `session_id`，完成后产生带该字段的 `BackgroundResult`。
2. daemon 在“保留后发布”的同一写路径中，把完整结果作为 `background_result` 事件发布。
3. TUI 的全局事件读取器在事件序号递增时解析该事件，仅接收 `session_id == App.session_id` 的结果，并发送 `AppEvent::BackgroundTaskResult`。
4. `AppEvent` 立即写入系统通知；若主 agent 空闲，事件处理器将结果放入一个隐藏 continuation 回合，以结构化 `user` 消息调用主 agent。若正在运行，则只显示通知，仍由下一次轮次的补偿拉取消费。
5. SSE 重连或漏事件时，下一次正常/continuation 回合仍从结果快照拉取；该补偿必须按当前 session 过滤，并对已消费结果去重。

## 约束与错误处理

- 结果的 `session_id` 是必须的归属字段，缺失的旧结果不得广播给任一会话。
- 全局 SSE 是 live-only；断线不应阻止之后的正常回合从快照获取结果。
- 全局事件读取器保持单订阅，继续处理 todo，并新增后台结果分支，避免建立第二个 SSE 连接。
- 事件处理仅入队到现有 `AppEvent` channel；不得在 SSE 任务中直接操作 `App` 状态。
- 任务启动、运行中进度没有可用的底层状态源；本变更实时通知终态（成功、失败、超时）。启动状态继续由工具返回值展示。

## 验证

- daemon 测试：结果先保留后广播，广播数据保留会话 ID。
- TUI 测试：其他会话的 `background_result` 被忽略；本会话的结果转为 `AppEvent::BackgroundTaskResult`。
- TUI 测试：收到本会话终态事件时空闲 agent 被安排 continuation；忙碌时不抢占现有回合。
- 回归：todos 事件订阅与原有 task-group continuation 测试仍通过。
