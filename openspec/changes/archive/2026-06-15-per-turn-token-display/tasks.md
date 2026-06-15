## 1. TokenCounter 扩展

- [x] 1.1 为 `TokenCounter` 新增 `turn_input: Arc<AtomicUsize>` 和 `turn_output: Arc<AtomicUsize>` 字段
- [x] 1.2 实现 `add_input(tokens: usize)` 方法，原子累加 turn_input
- [x] 1.3 实现 `add_output(tokens: usize)` 方法，原子累加 turn_output
- [x] 1.4 实现 `reset_turn()` 方法，将 turn_input 和 turn_output 归零
- [x] 1.5 实现 `turn_input_tokens(&self) -> usize` 和 `turn_output_tokens(&self) -> usize` 读取方法

## 2. AgentLoop 集成

- [x] 2.1 在 `AgentLoop::process_input` 入口调用 `token_counter.reset_turn()` 重置当前 turn 计数
- [x] 2.2 在 `process_input_inner` 中，用户消息推入历史前，估算 `input.len() / 4` 并调用 `token_counter.add_input()`
- [x] 2.3 在 `run_agent_loop` 中，将 `token_counter.add(usage.total_tokens)` 改为 `token_counter.add_output(usage.completion_tokens)`（预算控制仍用 `used` 字段，单独调用 `token_counter.add(usage.total_tokens)` 或用 `completion_tokens` 近似）

## 3. 状态栏渲染更新

- [x] 3.1 修改 `components::status::render` 签名，接收 `(input_tokens: usize, output_tokens: usize)` 替代原有的 `tokens_used: usize`
- [x] 3.2 修改 `format_tokens` 显示逻辑：`↑ N · ↓ Mk` 格式，token=0 时隐藏对应部分；k 单位阈值 1000

## 4. App 渲染适配

- [x] 4.1 在 `App::render_status` 中读取 `token_counter.turn_input_tokens()` 和 `token_counter.turn_output_tokens()` 传入 status render
