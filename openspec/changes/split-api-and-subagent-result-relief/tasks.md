## 1. src/api 模块拆分(纯重构,代码已在工作区)

- [ ] 1.1 创建 `src/api/error.rs`,移动 `wrap_network_error` / `format_api_error` 及其单元测试
- [ ] 1.2 创建 `src/api/types.rs`,移动 wire-format 类型(`ToolDefinition`/`ChatMessage`/`ChatRequest`/`ChatResponse`/`StreamChunk` 等)及其单元测试
- [ ] 1.3 更新 `src/api/mod.rs`:`pub mod error/types` + `pub use types::*` + `pub(crate) use error::*`,保持 `crate::api::*` 旧路径
- [ ] 1.4 `cargo build && cargo test --lib && cargo clippy --lib` 验证行为零变化、旧 `crate::api::*` 引用仍编译

## 2. subagent 大结果交付重新设计(行为变更)

- [ ] 2.1 comet-design 阶段 brainstorming 选定缓解方案(按需加载 B1 / compaction 降级 B2 / 混合 B3)
- [ ] 2.2 查清 `to_compact()` 是否 dead code、是否应接入 compaction 作为降级入口
- [ ] 2.3 按 design 定案调整 `src/teams/subagent_mailbox.rs`:`SubagentResponse` 变体、`to_content()`、`to_compact()`、`offload_if_large()`
- [ ] 2.4 同步 `src/tools/meta/task.rs` 调用方(若 `to_content` 返回语义变化)
- [ ] 2.5 若选 B2:扩展 `src/tui/agent/compaction.rs` 识别上下文中的大 subagent 结果并替换为磁盘引用
- [ ] 2.6 更新/新增测试覆盖选定方案的 spec scenarios(不丢细节 + token 可控 + 磁盘恢复 + 持久化失败降级)
- [ ] 2.7 `cargo test` 验证 `subagent-result-delivery` spec 全部 scenarios 通过
