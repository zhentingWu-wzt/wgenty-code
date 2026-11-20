# Tasks: Work-Graph 预算计费与诊断等待期修复

## P0：诊断等待期放开只读操作 + 明示等待语义

- [x] 1. 定位等待期拦截点（`waiting for its root-cause specialist report` 错误来源），改为按工具只读性分流：放行只读工具（exec_command/file_read/grep/glob/search/lsp/codegraph），继续拦截写入与图状态变更工具
- [x] 2. `verify_node` 派生 root-cause 专家时响应增加 `waiting_on` 说明字段
- [x] 3. 为等待期只读放行补单元测试

## P1：iter_used 只在重试 Implement 时计费

- [ ] 4. 移除 agent loop 每轮 `iter_used` 递增（node_runtime.rs:311），改在路由选择 Implement/RootCause 重试时计费
- [ ] 5. 更新依赖每轮递增语义的测试夹具与断言
- [ ] 6. 新增测试：导航轮次（读/查/验证工具调用）不消耗重试预算

## 收尾

- [ ] 7. `cargo test --lib` 全量回归 + `cargo clippy --lib -- -D warnings`
