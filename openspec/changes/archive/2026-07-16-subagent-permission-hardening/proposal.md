# Proposal: Subagent 权限闭环加固

## Why

当前 subagent 与主 agent 的工具权限路径不一致：

- **主 agent**：`ToolExecutor.validate_tool_call`（policy Allow/Ask）→ guardian → 执行 → sandbox
- **subagent**：`FilteredToolPort` 仅做白名单过滤后，经 `ToolRegistry.execute_with_context` **直执**，绕过 policy Ask 与统一 guardian 包装

这带来四个问题：

1. **安全旁路**：workspace 外写、危险命令、`requires_confirmation` 等在主会话会 Ask，在 child 上默认不经过同一闸门。
2. **角色假只读**：`explore`/`plan` 只去掉 `task`/`delegate`，写工具仍可能可见；「只读」主要靠 prompt。
3. **审批半成品**：`request_approval` 是 agent↔agent mailbox，不自动问用户；parent 不处理 inbox 则 60s 超时；payload 无结构化风险字段。
4. **失败不透明**：child 权限拒绝多半只进自身 history，parent 只看到 summary，无法汇总 denial。

目标是把 subagent 权限做成「与主 agent 同一危险动作、同一套闸门」，并补齐 Ask 收件人、角色 enforcement、可观测与配置。

## What Changes

1. **统一执行入口（P0）**
   所有 agent（root/child）经同一 `ToolPort`/`ToolExecutor` 路径：`validate_tool_call` → guardian（执行类）→ execute（hooks）→ sandbox。消除 subagent registry 直通捷径。

2. **Ask 升级策略（P0）**
   child 上 `PolicyDecision::Ask` 不得悬空：
   - 先匹配 parent/session 已批准 `session_rules` → Allow
   - 未命中 → escalate（默认 escalate_to_user，可配置）
   - 超时 → **Deny（fail closed）**
   无交互端口时 Ask → Deny，不静默放行。

3. **结构化审批与 Root 闭环（P0/P1）**
   审批请求自动附带 tool、paths、command、risk、policy_reason、child_id、request_id。
   Root/TUI/daemon 能消费 `ApprovalRequest`，桥到用户确认（或已有 permission UI），并回写 `ApprovalResponse`。

4. **角色真权限（P1）**
   - `explore` / `plan`：强制 read-only 工具集（去掉 write/edit/patch 及危险写类 exec，可配置）
   - `general-purpose`：全工具，但走统一 policy
   depth/并发/identity 约束保持。

5. **可观测（P1）**
   权限拒绝/升级写入 progress 与/或 `ChildResult` 侧车摘要（denial 列表），parent synthesis/claim 时可见。

6. **Settings（P1）**
   新增可配置项（命名以 design 为准），例如：
   - `agent.subagent.permission_mode`（默认 `workspace_write`）
   - `agent.subagent.ask_strategy`（默认 `escalate_to_user`）
   - `agent.subagent.explore_readonly`（默认 true）
   - `agent.subagent.approval_timeout_secs` / `timeout_decision: deny`

## Impact

- **Affected specs（新增/修改）**
  - 新增：`subagent-tool-permissions`（统一执行、Ask 升级、角色档位、可观测、settings）
  - 可能触及：`subagent-result-delivery`（denial 摘要进入交付语义时）
  - 可能触及：mailbox/team 相关隐式行为（ApprovalRequest 结构化与 root 消费）

- **Affected code（预期）**
  - `src/teams/subagent_loop.rs`：`FilteredToolPort` 改为走 ToolExecutor/统一 policy
  - `src/tools/executor.rs`、`src/permissions/policy.rs`：子 agent Ask 策略与 session_rules 共享/继承
  - `src/tools/meta/task.rs`：allowed_tools 按 type 真只读过滤
  - `src/tools/meta/request_approval.rs`、`src/teams/mailbox.rs` / approval_registry：结构化 payload
  - `src/daemon/handlers.rs`、`src/tui/agent/adapters.rs`（或等价）：root 消费 ApprovalRequest → 用户确认
  - `src/config/agent.rs`、settings template、WGENTY.md
  - 单测 + 集成测

- **Non-goals**
  - 不改 TaskGroup / claim 交付主路径语义
  - 不把 mailbox 改成最终结果通道
  - 不重做 guardian 规则引擎、MCP/插件权限大一统
  - 不默认「所有写操作都问人」

## Success Criteria

1. 主/子对同一危险 `exec_command` 均经 policy + guardian，无 registry 旁路。
2. explore 调用 `file_write` 被拒（白名单或只读策略）。
3. child 触发 Ask → 升级到用户；批准后可继续；超时 = Deny。
4. root 能处理 `ApprovalRequest` 并回 `ApprovalResponse`。
5. 多次 denial 在 child result/progress 中可见。
6. settings 可改 ask_strategy / explore_readonly。
7. `cargo fmt` / `clippy -D warnings` / 相关测试通过。
