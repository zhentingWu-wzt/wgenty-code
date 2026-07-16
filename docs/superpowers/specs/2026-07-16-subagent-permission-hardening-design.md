---
archived-with: 2026-07-16-subagent-permission-hardening
status: final
status: final
---
# Subagent 权限闭环加固 — Design Doc

**Change**: `subagent-permission-hardening`  
**Date**: 2026-07-16  
**Status**: Confirmed  
**OpenSpec**: `openspec/changes/subagent-permission-hardening/`

## Problem

Subagent 与主 agent 工具执行路径分裂：

| 路径 | 入口 | Policy | Guardian | Ask |
|------|------|--------|----------|-----|
| Root | `ToolExecutor` | Yes | Yes (exec) | Permission UI |
| Child | `FilteredToolPort` → registry | **No** | **No** | 无 / mailbox 半成品 |

后果：workspace 外写与危险命令可在 child 旁路；explore/plan「只读」主要靠 prompt；`request_approval` 不自动问用户；denial 对 parent 不透明。

## Goals

1. 同一危险动作，同一套闸门（visibility → policy → Ask 解析 → guardian → execute/hooks/sandbox）。
2. Ask 必有收件人；超时 **fail closed (Deny)**。
3. explore/plan **enforcement** 真只读（可配置回退）。
4. Root 系统侧消费审批，不依赖主 LLM 读 inbox。
5. Denial 对 parent 可观测；不破坏 TaskGroup / ChildResult 五字段 wire。
6. 默认能力与 root **对齐**（跟随 session mode），可选覆盖，避免无故锁死 general-purpose。

## Non-goals

- 不改 TaskGroup / claim 交付主路径语义。
- 不把 mailbox 当最终结果通道。
- 不重做 guardian 规则引擎或 MCP/插件权限大一统。
- 不默认「所有写操作都问人」。

## Confirmed Decisions

| # | Topic | Decision |
|---|--------|----------|
| D1 | 统一接线 | **GuardingToolPort**（`ToolPort` 包装层） |
| D2 | Permission mode | **默认跟随** 主 session mode；`agent.subagent.permission_mode` **可选覆盖** |
| D3 | session_rules | **共享** 同一 session（Always 对 root+children 生效；Allow once 仅本调用） |
| D4 | Ask 策略 | 默认 **`escalate_to_user`**；timeout → Deny |
| D5 | Root 消费 | **TUI/Daemon 审批桥**，复用 PermissionRequired UI |
| D6 | explore/plan | **`explore_readonly=true`**：去掉 `file_write`/`file_edit`/`apply_patch`；**exec 仍可见**但走 policy |
| D7 | 可观测 | progress 事件 + finish **summary 一行** denial 摘要；不改 ChildResult wire |

## Architecture

### GuardingToolPort

```
child tool_call
    │
    ▼
┌─ GuardingToolPort ─────────────────────────────────────┐
│ 1. allowed_tools / role filter                         │
│    → miss: tool_not_allowed (continue loop)            │
│ 2. ToolPermissionPolicy::evaluate (mode: follow/override)│
│    → Allow → continue                                  │
│    → Ask → session_rules hit? → Allow                  │
│         → else escalate_to_user (structured request)   │
│         → wait response / timeout → Deny               │
│ 3. guardian_check (exec_command / execute_command)     │
│ 4. registry.execute_with_context (+ hooks if wired)    │
└────────────────────────────────────────────────────────┘
```

**共享状态（Arc）**：

- `ToolPermissionPolicy` 视图（mode 解析后）
- `session_rules: Arc<RwLock<HashSet/Map>>`（与 root executor 同一把）
- `PermissionBridge` / approval waiter registry（oneshot + timeout）
- 可选：hooks 与 root 对齐的执行包装

**不采用**：FilteredToolPort 内 registry 直通；不采用完整 ToolExecutor 硬耦合（A），但校验逻辑应与 `ToolExecutor::validate_tool_call` **抽共享函数**，避免行为漂移。

### Mode resolution

```
subagent_mode = settings.agent.subagent.permission_mode
if subagent_mode is None:
    mode = root_session.permission_mode   # 跟随
else:
    mode = subagent_mode                  # 覆盖
```

### Ask → 用户闭环

1. Policy `Ask` + 构造 `session_rule` key。
2. 命中共享 rules → Allow。
3. 否则写 **结构化** `ApprovalRequest`（见下），注册 oneshot waiter（timeout = `approval_timeout_secs`，默认 60）。
4. **审批桥**（TUI tick / daemon 事件）：drain 待批请求 → PermissionRequired UI（Allow once / Always / Deny）。
5. Always → `approve_rule(session_rule)` + `ApprovalResponse(true)`；Once → 仅本调用 true；Deny/timeout → false。
6. Child 收到 false → tool result `permission_denied`（明确 code），**不执行副作用**。
7. Headless / 无桥：Ask → Deny + `approval_unavailable`（可测环境预置 rules）。

**不采用**：主 LLM 读 `<team-inbox>` 完成审批。

### Structured Approval payload

```json
{
  "from": "child-id",
  "request_id": "uuid",
  "kind": "policy_ask",
  "tool": "exec_command",
  "paths": [],
  "command": "optional",
  "risk": "High",
  "policy_reason": "...",
  "human_summary": "...",
  "session_rule": "command:...|path:...|tool:..."
}
```

Wire：扩展现有 mailbox / TeamMessage 字段（optional），旧自由文本仍可解析；**新 escalate 路径必须写结构化字段**。

### Role tool visibility

| type | allowed_tools |
|------|----------------|
| explore | 全工具 − spawn − **mutating fs**（write/edit/apply_patch）；exec 保留 + policy |
| plan | 同 explore（+ 既有 plan 类工具若存在） |
| general-purpose | 全工具 − depth 限制的 task/delegate |

`explore_readonly: false` → 回退「仅禁 spawn」。

### Observability

- progress / action_log：`permission_denied` | `approval_requested` | `approval_resolved`
- `finish_child` summary 尾部示例：`[permissions: 2 denials; last: workspace_escape, tool_not_allowed]`
- 不扩展 ChildResult 稳定五字段 wire

### Settings

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

| Key | Default | Meaning |
|-----|---------|---------|
| `permission_mode` | `null`（跟随 root） | 非 null 时覆盖 |
| `ask_strategy` | `escalate_to_user` | 另可 `deny` / 未来 `escalate_to_parent` |
| `explore_readonly` | `true` | explore/plan 去写工具 |
| `approval_timeout_secs` | `60` | waiter 超时 |
| `timeout_decision` | `deny` | 仅 deny（fail closed） |

同步：`src/config/agent.rs`、settings template、WGENTY.md。

## Components to touch

| Area | Files (expected) |
|------|------------------|
| Port | `src/teams/subagent_loop.rs`（GuardingToolPort 替换/增强 FilteredToolPort） |
| Policy share | `src/tools/executor.rs`, `src/permissions/policy.rs` |
| Spawn | `src/tools/meta/task.rs`, `src/teams/mod.rs` / spawn 路径 |
| Approval | `src/teams/mailbox.rs`, `approval_registry.rs`, `request_approval.rs` |
| Bridge | `src/tui/agent/adapters.rs`, daemon handlers / permission channel |
| Config/docs | `src/config/agent.rs`, templates, WGENTY.md |
| Tests | unit + integration as in tasks.md |

## Error codes (tool results)

| Code | When |
|------|------|
| `tool_not_allowed` | 角色/白名单不可见 |
| `permission_denied` | Ask denied / timeout / strategy deny |
| `approval_unavailable` | 需要 escalate 但无 UI/桥 |
| guardian block | 保持现有 guardian 错误形态 |

## Testing

1. GuardingToolPort：workspace 外写 / 危险命令与 root 同决策族（非旁路 Allow）。
2. session_rules 命中不再弹；超时 Deny 且无副作用。
3. explore 调 file_write → not allowed；file_read → 经 pipeline 允许。
4. child Ask → bridge approve → 执行成功；deny → 失败。
5. 多次 denial 后 summary 含摘要。
6. TaskGroup 交付回归仍绿；fmt/clippy/tests。

## Risks & mitigations

| Risk | Mitigation |
|------|------------|
| 确认风暴 | Always → shared rules；跟随 mode 减少重复 Ask |
| Headless CI | 预置 rules 或 `ask_strategy=deny` |
| 与 executor 行为漂移 | 抽取共享 `validate` 函数 |
| explore 过严 | `explore_readonly=false`；exec 仍可用 |
| 审批桥漏接 | Headless fail closed；集成测覆盖 bridge |

## Implementation order

1. P0：GuardingToolPort + 共享 validate/session_rules + guardian  
2. P0：Ask fail-closed + escalate waiter  
3. P0/P1：结构化 approval + TUI/Daemon bridge  
4. P1：explore/plan 只读过滤 + settings  
5. P1：progress/summary 可观测 + 文档 + 回归  

## Spec alignment

Delta: `openspec/changes/subagent-permission-hardening/specs/subagent-tool-permissions/spec.md`  
本 Design Doc 与 delta 一致；mode **默认跟随** 应在 design.md / settings 叙述中明确 `null = follow`。
