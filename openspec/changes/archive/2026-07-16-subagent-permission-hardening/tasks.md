# Tasks: Subagent 权限闭环加固

## 1. P0 — 统一执行入口

- [x] 1.1 梳理并文档化当前 subagent 与 daemon 主路径的工具执行差异（FilteredToolPort vs ToolExecutor）
- [x] 1.2 实现 `GuardingToolPort`：注入共享 policy 校验、session_rules、guardian、approval bridge；禁止 registry 直通危险路径
- [x] 1.3 从 `ToolExecutor::validate_tool_call` 抽取共享校验，供 root 与 GuardingToolPort 复用（防行为漂移）
- [x] 1.4 保证 child 与 root 共享同一 session 的 `session_rules`（Arc）；mode 默认跟随 root，settings 可覆盖
- [x] 1.5 单测：subagent 路径对 workspace 外写 / 危险命令触发与主路径一致的 PolicyDecision（非旁路 Allow）
- [x] 1.6 单测：白名单外工具仍返回 `tool_not_allowed`，loop 可继续

## 2. P0 — Ask 策略与 fail-closed

- [x] 2.1 实现 Ask 解析：session_rules 命中 → Allow；否则按 `ask_strategy` 处理
- [x] 2.2 默认 `ask_strategy = escalate_to_user`；`timeout_decision = deny`
- [x] 2.3 无交互端口 / escalate 失败 → 明确 `permission_denied` / `approval_unavailable` tool error（不静默执行）
- [x] 2.4 审批超时 → Deny（oneshot false 或等价），清理 waiter
- [x] 2.5 单测：Ask + 已批准 rule → 直接执行；Ask + 超时 → Deny；Ask + 无 UI → Deny

## 3. P0/P1 — 结构化审批与 Root 闭环

- [x] 3.1 定义结构化 approval payload（tool、paths、command、risk、policy_reason、session_rule、child_id、request_id）
- [x] 3.2 扩展 `ApprovalRequest` 序列化（向后兼容旧自由文本）
- [x] 3.3 policy Ask escalate 时自动填充结构化字段（不依赖模型手写 payload）
- [x] 3.4 Root/TUI：pending-permissions API + TUI poller 复用 PermissionRequired UI，resolve + 可选 always session_rule
- [x] 3.5 Daemon/headless：定义无 TUI 时的行为（默认 Deny；测试可预置 rules）
- [x] 3.6 集成测：child Ask → root approve → 工具成功；child Ask → deny/timeout → 工具失败且未执行副作用

## 4. P1 — 角色真权限

- [x] 4.1 `explore`/`plan`：在 `task` 的 allowed_tools 过滤中强制只读工具集（`explore_readonly` 默认 true）
- [x] 4.2 `general-purpose`：保留全工具 + depth 限制 spawn
- [x] 4.3 配置 `explore_readonly: false` 可回退旧「仅禁 spawn」行为
- [x] 4.4 单测：explore 调用 file_write/file_edit/apply_patch → not allowed；explore 调用 file_read/grep → 允许（再经 policy）

## 5. P1 — 可观测

- [x] 5.1 progress/action_log 记录 permission_denied / approval_requested / approval_resolved 事件
- [x] 5.2 child 终态 summary 附带 denial 摘要（计数 + 最近 reasons），不破坏 ChildResult 五字段 wire
- [x] 5.3 单测或集成：多次 denial 后 parent 可见摘要

## 6. P1 — Settings 与文档

- [x] 6.1 `config` 增加 subagent permission 相关字段与默认值；settings.json.template 同步
- [x] 6.2 WGENTY.md / 必要用户文档说明默认行为与开关
- [x] 6.3 delta spec：`specs/subagent-tool-permissions/spec.md`（及若需要的 result-delivery 补充）

## 7. 验证与收尾

- [x] 7.1 `cargo fmt --check`
- [x] 7.2 `cargo clippy --all-targets -- -D warnings`（本轮以 `cargo clippy --lib -- -D warnings` 通过；all-targets 与 host CI 对齐时再全量跑）
- [x] 7.3 相关单测 + TaskGroup/subagent 交付回归测试通过（`teams::` 51 + guarding/permission_bridge）
- [x] 7.4 脚本/单测覆盖：explore 写拒绝（filter 单测）；outside write deny；Ask approve/deny/timeout（bridge + GuardingToolPort）
