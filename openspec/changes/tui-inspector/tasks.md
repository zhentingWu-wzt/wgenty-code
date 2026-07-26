## 0. 删除遗留 task_panel

- [ ] 0.1 删除文件 `src/tui/components/task_panel.rs`
- [ ] 0.2 在 `src/tui/components/mod.rs` 移除 `pub mod task_panel;`
- [ ] 0.3 在 `src/tui/app/mod.rs` 移除 `task_panel: TaskPanel` 字段及初始化
- [ ] 0.4 在 `src/tui/app/event.rs` 移除 `Ctrl+T` toggle 处理
- [ ] 0.5 在 `src/tui/app/render.rs` 移除 task_panel 右侧 Split Pane 布局 + 渲染逻辑

## 1. 数据模型层

- [ ] 1.1 在 `src/prompts/mod.rs` 新增 `LayerMeta`、`LayerSource`、`MemoryMeta`、`PartialTurnContext`、`TurnContext` 数据结构，`AssembledInstructions` 增加 `layers: Vec<LayerMeta>` 字段
- [ ] 1.2 在 `assemble_instructions()` 中为每层 system prompt 填充 `LayerMeta`（label + source + content + char_count），保持 `system_messages` 向后兼容
- [ ] 1.3 在 `MemoryMeta` 中捕获 `MemoryContextInjector` 召回的 project/global 记忆元数据（scope、importance、score）
- [ ] 1.4 在 `process_input_inner()` 中捕获 `build_user_turn_reminder()` 返回的 `ReminderOutput`，存入 App 临时字段供 Inspector 读取

## 2. 轮次快照

- [ ] 2.1 在 `src/tui/app/mod.rs` 的 `App` struct 增加 `turn_contexts: Vec<TurnContext>` + `pending_context: Option<PartialTurnContext>` 字段
- [ ] 2.2 在 `process_input_inner()` 调用后 stash `PartialTurnContext`（assembled_layers + memories + reminder）
- [ ] 2.3 在 `run_agent_loop()` 返回后 finalize `TurnContext`（补充 full_messages）并 push 到 ring buffer（max 50）
- [ ] 2.4 实现 `TurnContext` ring buffer 溢出时移除最旧条目

## 3. Inspector 组件

- [ ] 3.1 创建 `src/tui/components/inspector.rs`：`InspectorComponent` struct + `Component` trait 实现
- [ ] 3.2 实现 Tab 导航头渲染（L | M | Msg | H | T），Tab/Shift+Tab 切换，高亮当前 tab
- [ ] 3.3 实现 **Layers Tab**：树形列表，每层显示 label+source badge，Enter 展开/折叠，L 展开/折叠全部
- [ ] 3.4 实现 **Memories Tab**：记忆列表，scope badge + importance 进度条 + content preview
- [ ] 3.5 实现 **Messages Tab**：等宽字体渲染完整 messages 数组，role 标签着色，viewport clipping
- [ ] 3.6 实现 **Hooks Tab**：分区展示 `ReminderOutput` 的 to_model / to_transcript
- [ ] 3.7 实现 **Tokens Tab**：字符数 + 估算 token + measured_tokens（如有）表格
- [ ] 3.8 实现历史轮次导航（↑↓），footer 显示 "Turn N/M"，新轮完成不自动跳转
- [ ] 3.9 实现滚动（J/K or ↑↓ in list mode，PgUp/PgDn），viewport 裁剪

## 4. 布局集成

- [ ] 4.1 在 `src/tui/app/render.rs` 中实现 Inspector 右侧 Split Pane：可见时 chat 65% + Inspector 35%
- [ ] 4.2 Inspector 渲染接入 `render()` 主循环
- [ ] 4.3 F2 切换 Inspector visible 状态，关闭时主面板恢复 100%
- [ ] 4.4 窗口宽度 < 80 列时自动折叠 Inspector（不展示，保留状态）
- [ ] 4.5 `subagent_focus_view` 弹出时 Inspector 自动折叠，关闭后恢复

## 5. 组件注册与主题

- [ ] 5.1 在 `src/tui/components/mod.rs` 注册 `InspectorComponent`
- [ ] 5.2 在 `src/tui/theme.rs` 增加 Inspector 专用颜色常量（tab_active、tab_dim、source_badge_builtin、source_badge_file、source_badge_memory、importance_high...）

## 6. 测试与验真

- [ ] 6.1 `inspector.rs` 组件级单元测试：tab 切换、层展开/折叠、历史轮次边界
- [ ] 6.2 `prompts/mod.rs` 测试：`assemble_instructions` 产出的 `layers` 每层 source 标注正确
- [ ] 6.3 `tui/app/turn.rs` 测试：`TurnContext` 快照数据完整性
- [ ] 6.4 确认 task_panel 删除后编译通过，无残留引用
- [ ] 6.5 `cargo fmt --check` + `cargo clippy --all-targets -- -D warnings` 通过
- [ ] 6.6 `cargo test` 相关测试全部通过
