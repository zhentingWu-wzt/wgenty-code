# Tasks

## 1. 数据模型与 schema 迁移

- [x] 1.1 `SubagentTranscript`（`src/transcript/mod.rs:16`）新增 `node_type: Option<NodeType>` 字段，`#[serde(default)]`
- [x] 1.2 `SubagentTranscriptHeader`（`src/transcript/mod.rs:76`）新增 `node_type`（供 `list` 视图）
- [x] 1.3 `transcript/store.rs` 为 `subagent_transcripts` 增加幂等 `ALTER` 加 `node_type TEXT` 列（照搬 `failure_diagnostics` 迁移模式 `:134`）
- [x] 1.4 `save` / 读取路径读写 `node_type`；未知字符串降级为 `None`（不 panic）
- [x] 1.5 迁移单测：旧库（无 node_type 列）打开不崩 + legacy row 为 None；重复迁移幂等

<!-- tasks 1.1-1.5: DB 列存 serde 变体名字符串（node_type_to_db / parse_node_type_col，
     NULL/空/未知值降级 None + warn，不 panic）。迁移测试用真实 pre-migration schema
     建库：test_legacy_db_migration_adds_node_type / test_migration_is_idempotent
     （pragma_table_info 计数=1）/ test_unknown_node_type_string_degrades_to_none。
     get_by_id + list_by_session/list_by_project/search 四条读取路径均带 node_type。 -->

## 2. node_type 透传管道

- [x] 2.1 grep 确认 `node_type` 当前到达管道的最深位置（从 `coordinator.rs:507` 起）
- [x] 2.2 在 `save_minimal_transcript`（`task.rs:337`）透传 `node_type` 到 transcript 构造
- [x] 2.3 在 `store.save`（`task.rs:1009`）确保 `node_type` 写入 DB
- [x] 2.4 透传单测：分别以 5 种 NodeType 分发并保存，transcript 持久化正确 node_type

<!-- task 2.1 findings: node_type 最深到达 task.rs `execute_with_context`（解析于
     parse_node_type，经 SpawnChildRequest 传给 coordinator），transcript 构造点恰好
     同文件，无需穿层传参。透传点全览：
     - task.rs 主路径：node_type_bg 捕获进 spawn 闭包 → build_transcript(Some)
     - task.rs interception-1 fallback：execute_fallback_sync 新增 node_type 参数
     - run_script.rs 两处 + rlm/pipeline.rs 三处：Some(GeneralPurpose)（与其
       dispatch 的 with_node_type(GeneralPurpose) 一致）
     - subagent_loop.rs interception-2 模型回退：从失败 child 的已存 transcript
       恢复 node_type（同一逻辑 run）
     task 2.4: save_minimal_transcript_persists_each_node_type（Explore/Plan/
     GeneralPurpose/Verification/WgentyCodeGuide 5 种）+ None 保持 legacy。 -->

## 3. Subagent CLI inspector 展示

- [x] 3.1 `subagent list` 渲染增加 `node_type` 列；legacy（None）显示占位 `-`
- [x] 3.2 `subagent trace <id>` 渲染增加该 run 的 `node_type`
- [x] 3.3 CLI 展示单测：list 含 node_type 列；trace 含 node_type；legacy run 占位正确

<!-- tasks 3.1-3.3: list 列序 ID|LABEL|STATUS|NODE-TYPE|ROOT-CAUSE|…（宽 16，
     容纳 WgentyCodeGuide）；trace 文本格式（call-tree / error-timeline）前置
     "Node type: X" 一行，raw JSON 输出含 node_type 字段，chrome/html 机器格式
     payload 保持纯净（D4）。测试：list_renders_node_type_column（含 legacy "-"）、
     trace_renders_all_formats… 断言 "Node type: Explore"、trace_raw… 断言
     v["node_type"]=="Explore"。 -->

## 4. 集成与回归验证

- [x] 4.1 `cargo build` 通过
- [x] 4.2 `cargo test` 全绿：新增测试通过 + 已有 subagent transcript / CLI 测试零回归
- [x] 4.3 手动验证：分发一个 subagent → `subagent list` / `trace <id>` 正确显示其 node_type；旧库打开正常

<!-- tasks 4.1-4.3: cargo check --all-targets / clippy -D warnings / fmt 干净；
     cargo test 全绿（lib 1804 + integration 218，0 失败，含全部既有
     transcript/CLI/daemon/trace/health 套件零回归）。4.3 的"旧库打开正常"由
     test_legacy_db_migration_adds_node_type 用真实 legacy schema 建库覆盖；
     list/trace 展示由真实 store + 渲染函数的 CLI 测试覆盖；真实 LLM 分发的
     端到端冒烟无法在本沙箱执行（无 daemon/模型凭据），透传管道已由 2.4 的
     真保存-真读取测试等价证明。 -->
