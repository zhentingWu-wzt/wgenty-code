# Tasks: 子 agent 结果投递时机改进

## L0: root finalize join 接线

- [x] 1. `TaskGroupStore` 新增 `peek_ready`（只读就绪计数，不消费）；单元测试覆盖就绪/未就绪/generation 不匹配
- [x] 2. daemon 新增 `POST /api/v1/agents/task-groups/await`（快路径 claim + 订阅节流重试，`timeout_ms` 上界 120s）；handler 测试覆盖：命中、超时 204、恰好一次 claim
- [x] 3. TUI client 新增 `await_task_group`；新 `RootSynthesisPort` 实现 `SynthesisPort` 并在 `tui/agent/core.rs` 接线（替换 `synthesis: None`）；错误降级为 `None` 不影响 finalize
- [x] 4. 修正 `subagent_loop.rs` root join 注释与实际一致

## L1: 回合内就绪提示

- [x] 5. daemon `execute_tool` 在响应 content 末尾捎带就绪 `<reminder>`（`peek_ready > 0` 时）；handler 测试覆盖捎带与不捎带两路

## 收尾

- [x] 6. `cargo test --lib` 全量 + `cargo clippy --lib -- -D warnings`
