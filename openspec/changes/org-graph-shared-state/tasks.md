# Tasks

## 1. WorkState schema 与模块骨架

- [ ] 1.1 新建 `src/org_graph/work_state.rs`，定义 `WorkState` struct，至少含字段：`requirement` / `generated_diff` / `compile_result`（含 `ok: bool` 与 `stderr`）/ `test_result`（含 `pass: bool` 与 `failed_cases`）/ `human_review`（enum `Approve`/`Reject`）/ `budget`（含 `max_iter` / `iter_used` / `token_used`）/ `step_log`；派生 `Serialize/Deserialize/Clone/Debug`
- [ ] 1.2 在 `src/org_graph/mod.rs` 导出 `pub mod work_state;` 及相关公开类型
- [ ] 1.3 为 schema 加单测：序列化往返（serialize → deserialize 字段等价）、默认值符合预期、各结构化子字段类型不丢失

## 2. 字段级访问权限（真强制）

- [ ] 2.1 设计并定义每个 `NodeType` 的「可读字段集 + 可写字段集」声明矩阵（至少覆盖 5 个内置节点类型的 pilot 相关字段）
- [ ] 2.2 实现 `WorkState` 受权限约束的读写 API：调用方提供 `NodeType`，越权写直接返回 `ContractViolation`（复用 `ContractDimension`，复用 `Permission` 还是新增 `State` 维度见 design 阶段定）
- [ ] 2.3 为权限强制加单测：节点正常读写授权字段成功、越权写字段被拒绝并触发 `ContractViolation`、授权读不写审计日志而授权写记入 `step_log`

## 3. turn 集成与检查点持久化

- [ ] 3.1 把 `WorkState` 锚定到 `exec_session` 的 turn：turn 开始时创建/继承 `WorkState` 实例（turn 间继承策略见 design 阶段定，默认只读字段继承、可写字段按 pilot 语义）
- [ ] 3.2 随 `CheckpointStore` 持久化 `WorkState`：turn 检查点时一并落盘，崩溃后从最近 turn 快照恢复结构化字段
- [ ] 3.3 为 turn 集成加单测：写入结构化字段后崩溃→从检查点恢复→字段值完整；既有文件级 capture/rewind 行为零回归

## 4. pilot 路由点（读结构化字段 + 验证强制）

- [ ] 4.1 查证现有代码是否存在真实的编译闭环或测试闭环路由（编译/测试结果是否真的回流驱动重试），按查证结果与 design D5 硬约束（必须含真实写字段场景）选定 pilot 路由点
- [ ] 4.2 实现 pilot 路由点：从「解析节点自然语言输出」改为「读取 `WorkState` 结构化字段」（如读 `compile_result.ok` 或 `test_result.pass`）做下一跳判定
- [ ] 4.3 确保 pilot 路由点涉及的写字段场景受 2.x 字段级权限强制（节点须声明可写该字段），并提供验证该强制的测试

## 5. 三层状态分层与零回归

- [ ] 5.1 验证 `WorkState` 与 `SessionState`（会话骨架）/ `AppState`（全局配置）三层职责分明：`SessionState` 字段语义不变、`AppState` 完全不动
- [ ] 5.2 `SubagentResultMailbox` 在非 pilot 路径维持原状（与 `WorkState` 并存，不强制同步、不强制淘汰）
- [ ] 5.3 与 `org-graph-dispatch-telemetry` 正交性验证：两者字段/schema 互不依赖、互不修改

## 6. 集成验证

- [ ] 6.1 `cargo build` 通过；`cargo test` 全绿（新增测试通过 + 既有节点契约 / 派发 / 契约强制 / transcript 测试零回归）
- [ ] 6.2 手动验证 pilot 路由点按结构化字段正确路由（成功路径 + 失败回流路径），且越权写字段被拦截
