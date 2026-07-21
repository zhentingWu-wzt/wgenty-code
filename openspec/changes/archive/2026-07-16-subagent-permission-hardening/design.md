# Design: Subagent 权限闭环加固

## Overview

将 subagent 工具执行从「白名单 + registry 直执」改为与主 agent 共享的 **统一权限管线**，并补齐 Ask 升级、角色 enforcement、结构化审批与可观测性。

```
                    ┌──────────────────────────────────────┐
                    │           统一 Tool 管线              │
  root / child ───► │ 1. 可见性 (allowed_tools / role)      │
                    │ 2. ToolPermissionPolicy (Allow/Ask)   │
                    │ 3. Ask 解析 (rules / escalate / deny) │
                    │ 4. Guardian (exec 类)                 │
                    │ 5. execute + hooks + sandbox          │
                    └──────────────────────────────────────┘
                                      │
                    Ask 未命中 session_rules
                                      ▼
                    结构化 ApprovalRequest ──► root UI / user
                                      │
                              ApprovalResponse
                                      ▼
                              child 继续或 Deny
```

## Goals & Constraints

- **同一危险动作，同一套闸门**：禁止 child 旁路 `validate_tool_call` / guardian。
- **Ask 必须有收件人**：session_rules | escalate_to_user | escalate_to_parent；禁止悬空。
- **超时 fail closed**：默认 Deny。
- **角色用 enforcement 表达**：explore/plan 真只读。
- **最小破坏交付语义**：TaskGroup / finish_child / claim_ready 不改行为契约。
- **可配置**：默认跟随主 session mode；`escalate_to_user` + `explore_readonly=true`；超时 Deny。
- **性能**：不显著增加启动/每工具调用延迟（额外校验为内存规则 + 可选 mailbox 写）。

## Architecture Decisions

### D1: 统一执行入口 — GuardingToolPort（已确认）

**决策**：引入 **`GuardingToolPort`**（`ToolPort` 包装），替代「白名单 + registry 直通」：

1. `allowed_tools` / 角色可见性
2. 与 `ToolExecutor::validate_tool_call` **共享的** policy 校验（抽函数，防漂移）
3. Ask 解析（session_rules / escalate / deny）
4. 对 `execute_command`/`exec_command`：`guardian_check`
5. 通过后 `execute_with_context`（+ hooks 若已接线）

**不采用**：Shared 完整 `ToolExecutor` 硬耦合；不采用仅 `Tool::execute` 内散落检查；不采用 P0「Ask 一律 Deny」瘦身方案作为终态。

**session_rules 共享**：

- child 与 root 共享同一 session 的 `Arc` rules。
- **Allow once** 仅本调用；**Always** 写入 session_rules 供 root+children 复用。

**permission_mode（已确认）**：

- 默认 **跟随** 主 session 当前 mode（`agent.subagent.permission_mode = null`）。
- 非 null 时 **覆盖** 为 subagent 独立 mode。
- 目标：general-purpose 能力与 root 对齐，避免无故更死。

### D2: Ask 策略状态机

```
validate → Allow ──────────────────────────► execute
         → Ask
              ├─ session_rules hit ─────────► execute
              ├─ ask_strategy=deny ─────────► tool error (permission_denied)
              ├─ escalate_to_user ──────────► structured ApprovalRequest
              │                                 wait ApprovalResponse / timeout
              │                                 approve → execute
              │                                 reject/timeout → Deny
              └─ escalate_to_parent ────────► mailbox to parent agent id
                                              (parent 须回复；超时 Deny)
```

**默认**：`escalate_to_user`。
**无 UI / headless 且无法 escalate**：Deny + 明确 error code（`permission_denied` / `approval_unavailable`）。

### D3: 结构化 Approval payload

扩展 `TeamMessage::ApprovalRequest`（或并行字段，保持 serde 兼容）：

```rust
// 逻辑字段（wire 可用 JSON 嵌在 payload 或新增 optional 字段）
{
  "from": "child-id",
  "request_id": "uuid",
  "kind": "policy_ask" | "destructive_edit" | ...,
  "tool": "exec_command",
  "paths": ["..."],
  "command": "optional",
  "risk": "Low|Medium|High|Critical",
  "policy_reason": "workspace_escape|dangerous_command|...",
  "human_summary": "...",
  "session_rule": "path:...|command:...|tool:..."
}
```

兼容：旧仅 `kind`+自由文本的请求仍可解析；新路径始终写结构化字段。

### D4: Root 消费 ApprovalRequest — 审批桥（已确认）

**决策**：系统侧 **TUI/Daemon 审批桥**（非 LLM）：

- TUI：tick / turn 间隙消费待批结构化请求 → 复用 `PermissionRequired` UI → `ApprovalResponse` + 可选 `approve_rule`。
- Daemon：复用已有 permission 事件通道或等价 bridge，避免第二套 UI。
- Headless：Ask → Deny（`approval_unavailable`）；测试可预置 session_rules。

**不采用**：主 LLM 读 `<team-inbox>`；本阶段不强制「完全统一 PermissionChannel 单一类型」（可作为后续清理），但 bridge 应对用户表现为与主 agent Ask 同一套 UI。

### D5: 角色工具集（已确认）

| type | allowed_tools 规则 |
|------|-------------------|
| explore | 排除 `file_write` / `file_edit` / `apply_patch`；**保留 exec**，仍走 policy + guardian |
| plan | 同 explore |
| general-purpose | 全工具 − depth 限制的 task/delegate |

`explore_readonly: false` 时回退到「仅禁 spawn」的旧行为（兼容开关）。

### D6: 可观测（已确认）

- progress / action_log：`permission_denied` / `approval_requested` / `approval_resolved`
- finish summary **一行** denial 摘要（计数 + 最近 reasons）
- **不破坏** `ChildResult` 五字段 wire

### D7: Settings（已确认）

```json
{
  "agent": {
    "subagent": {
      "permission_mode": null,
      "ask_strategy": "escalate_to_user",
      "explore_readonly": true,
      "approval_timeout_secs": 60,
      "timeout_decision": "deny"
    }
  }
}
```

- `permission_mode: null` → **跟随** 主 session mode；非 null → 覆盖
- 详见 Design Doc：`docs/superpowers/specs/2026-07-16-subagent-permission-hardening-design.md`

## Data / Control Flow

### 工具调用（child）

1. 模型发出 tool_call
2. `allowed_tools` 检查 → 失败则 `tool_not_allowed` tool result，**继续 loop**
3. policy validate → Allow / Ask
4. Ask → rules / escalate / deny
5. guardian（exec）→ block 则 tool error
6. execute；失败则 tool error
7. 成功 tool result 回 history

### 审批（escalate_to_user）

1. child 阻塞在 oneshot（timeout 可配）
2. root bridge 展示 UI
3. 用户 Always → approve_rule + ApprovalResponse(true)
4. 用户 Deny / 超时 → ApprovalResponse(false) 或直接 channel false
5. child 收到后继续或返回 permission_denied

## Migration & Compatibility

- 默认更严：旧工作流中依赖「child 无确认写危险命令」的行为会变失败或需用户批一次。
- 可通过 `explore_readonly: false`、`ask_strategy: deny`（快速失败）、或放宽 mode 缓解。
- 不自动迁移旧 mailbox 消息格式；新字段 optional。

## Testing Strategy

- 单元：FilteredToolPort/Executor 对 Allow/Ask/Deny；explore 白名单；结构化 approval serde
- 集成：child Ask → root approve → 执行成功；超时 Deny
- 回归：TaskGroup 交付 10 项既有测试仍绿；主会话 permission UI 不回归

## Risks

| 风险 | 缓解 |
|------|------|
| 用户确认风暴 | session Always allow；合理 mode；非危险路径 Allow |
| headless CI 全挂 | 测试用 approve 规则预置 / ask_strategy=deny 明确错误 |
| 与主 agent 并发抢 session_rules | RwLock；规则 key 稳定 |
| explore 过严影响 RLM/任务 | 配置开关；general-purpose 不受影响 |

## Phased Implementation Order（单 change 内）

1. P0 **GuardingToolPort** + 共享 validate/session_rules + guardian
2. P0 Ask fail-closed + escalate_to_user waiter
3. P0/P1 结构化 approval + TUI/Daemon 审批桥
4. P1 explore/plan 只读白名单 + settings（mode null=跟随）
5. P1 可观测 + 文档 + 回归
