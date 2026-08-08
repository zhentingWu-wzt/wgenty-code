# Tasks: inspector-perspective

## 1. Daemon 数据采集

- [x] 1.1 Memory recall 注入 run_session_turn（补 daemon 功能缺口，recall 返回类型扩展为 RecallResult）
- [x] 1.2 System prompt layers 保留（assemble_instructions 不再丢弃 layers）
- [x] 1.3 Token usage 注入（TokenCounter 注入 LoopHooks，per-turn 采集）
- [x] 1.4 Turn messages 采集（seed_len 切片，final_history 新增消息）
- [~] 1.5 Hook reminder 注入 —— deferred：daemon HookManager 不支持 prompt reminder，需 plugins/hooks 集成

## 2. TurnContext SSE 广播

- [x] 2.1 新增 SessionEventKind::TurnContext（layers + memories + messages + reminder + usage）
- [x] 2.2 在 final save 后广播一次 TurnContext

## 3. 前端 InspectorPanel

- [x] 3.1 sessionRunner 消费 turn_context 事件
- [x] 3.2 sessionStore 加 turnContext 字段 + TurnContextData 类型
- [x] 3.3 InspectorPanel 5 tab 视图（Layers/Memories/Messages/Hooks/Tokens）
- [x] 3.4 RightRail 加 inspector 入口

## 4. 验证

- [x] 4.1 cargo test 1493 passed + clippy 0 warnings
- [x] 4.2 npm build + lint 0 errors
