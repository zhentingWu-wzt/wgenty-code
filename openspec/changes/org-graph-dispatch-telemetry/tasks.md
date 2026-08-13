# Tasks

## 1. 数据模型与 schema 迁移

- [ ] 1.1 `SubagentTranscript`（`src/transcript/mod.rs:16`）新增 `node_type: Option<NodeType>` 字段，`#[serde(default)]`
- [ ] 1.2 `SubagentTranscriptHeader`（`src/transcript/mod.rs:76`）新增 `node_type`（供 `list` 视图）
- [ ] 1.3 `transcript/store.rs` 为 `subagent_transcripts` 增加幂等 `ALTER` 加 `node_type TEXT` 列（照搬 `failure_diagnostics` 迁移模式 `:134`）
- [ ] 1.4 `save` / 读取路径读写 `node_type`；未知字符串降级为 `None`（不 panic）
- [ ] 1.5 迁移单测：旧库（无 node_type 列）打开不崩 + legacy row 为 None；重复迁移幂等

## 2. node_type 透传管道

- [ ] 2.1 grep 确认 `node_type` 当前到达管道的最深位置（从 `coordinator.rs:507` 起）
- [ ] 2.2 在 `save_minimal_transcript`（`task.rs:337`）透传 `node_type` 到 transcript 构造
- [ ] 2.3 在 `store.save`（`task.rs:1009`）确保 `node_type` 写入 DB
- [ ] 2.4 透传单测：分别以 5 种 NodeType 分发并保存，transcript 持久化正确 node_type

## 3. Subagent CLI inspector 展示

- [ ] 3.1 `subagent list` 渲染增加 `node_type` 列；legacy（None）显示占位 `-`
- [ ] 3.2 `subagent trace <id>` 渲染增加该 run 的 `node_type`
- [ ] 3.3 CLI 展示单测：list 含 node_type 列；trace 含 node_type；legacy run 占位正确

## 4. 集成与回归验证

- [ ] 4.1 `cargo build` 通过
- [ ] 4.2 `cargo test` 全绿：新增测试通过 + 已有 subagent transcript / CLI 测试零回归
- [ ] 4.3 手动验证：分发一个 subagent → `subagent list` / `trace <id>` 正确显示其 node_type；旧库打开正常
