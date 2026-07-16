# Subagent Permission Manual Test Kit

最小复现：验证 subagent 权限管线（`GuardingToolPort`），不是 `wgenty-code agent` CLI。

## 重要前提

| 入口 | 是否走 subagent 权限管线 |
|------|--------------------------|
| REPL / Query 里 root 调用 `task` 起子代理 | **是**（`TaskTool` → `GuardingToolPort`） |
| `wgenty-code agent --agent-type ...` | **否**（`AgentsService` 直跑，无 permission bridge） |

请用 **REPL** 粘贴下面的 prompt，不要用 `agent` 子命令测权限。

权限链路：

```
allowed_tools 过滤
  → ToolPermissionPolicy (Allow / Ask)
  → Ask 决策 (session_rules / escalate_to_user / deny)
  → guardian (仅 exec_command / execute_command)
  → 真正执行
```

相关配置（`~/.wgenty-code/settings.json`）：

| Key | 默认 | 作用 |
|-----|------|------|
| `agent.subagent.explore_readonly` | `true` | explore/plan 隐藏 `file_write`/`file_edit`/`apply_patch` |
| `agent.subagent.ask_strategy` | `escalate_to_user` | Ask → 升级用户 / 直接 deny |
| `agent.subagent.approval_timeout_secs` | `60` | 审批超时秒数 |
| `agent.subagent.timeout_decision` | `deny` | 超时后 fail closed |

## 快速开始

```bash
# 在仓库根目录
./scripts/subagent-permission/setup.sh defaults   # 恢复默认权限配置
./scripts/subagent-permission/setup.sh deny       # ask_strategy=deny
./scripts/subagent-permission/setup.sh escalate   # ask_strategy=escalate_to_user + 短超时
./scripts/subagent-permission/setup.sh writable-explore  # explore_readonly=false

# 开 REPL（需本机有 API key）
cargo run -- repl
# 或 release：
# cargo run --release -- repl
```

然后打开 `prompts.md`，按场景 **A→E** 逐条粘贴到 REPL。

## 观察点

- 工具结果里的 error code：
  - `tool_not_allowed`
  - `permission_denied`
  - `approval_unavailable`
  - `guardian_blocked`
- parent 摘要后缀：`[permissions: N denials; last: ...]`
- 审批观测文件：`.team/inbox/approval-obs-*.jsonl`
- 副作用是否发生（文件是否被创建）

## 单测（无 API，确定性）

```bash
./scripts/subagent-permission/run-unit.sh
```

## 恢复配置

```bash
./scripts/subagent-permission/setup.sh defaults
```
