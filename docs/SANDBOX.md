# 沙箱执行模型

`src/sandbox/` 为命令执行提供跨平台 OS 级隔离。每个平台一个 backend，统一
实现 `SandboxBackend` trait；`SandboxManager::new()` 自动选择最优 backend，
不可用时降级到 `NoneBackend`（仅策略层）。

## 平台后端

| 平台 | 后端 | 机制 | 硬件强制 |
|------|------|------|----------|
| macOS | `seatbelt` | `sandbox-exec` + 生成 `.sb` profile（FS/网络/syscall） | 是 |
| Linux | `seccomp+ns` | `unshare`（mount/net/pid namespace）+ cgroups v2（内存/CPU/pids） | 是 |
| Windows | `job-object` | Win32 Job Object（进程数/内存/CPU 限制 + kill-on-close） | 是 |
| 无可用 | `none` | 仅 env 过滤 | 否 |

## 当前能力与边界

### Windows（`job-object`）
已实现：
- `JOB_OBJECT_LIMIT_ACTIVE_PROCESS`（`max_processes`）
- `JOB_OBJECT_LIMIT_JOB_MEMORY`（`max_memory_bytes`）
- `JOB_OBJECT_LIMIT_PROCESS_TIME`（`max_cpu_seconds`，FILETIME 单位）
- `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`（关闭 job handle 即终止进程树）
- 环境变量 allowlist
- `cmd.kill_on_drop(true)` 双保险
- cleanup 路径显式 `CloseHandle`（与 kill-on-close 协同）

**未实现**（计划中）：
- Restricted Token（deny-only SID / 去特权）：当前不做 FS/注册表最小权限隔离
- 创建挂起进程后 resume 的「先入队再跑」流程（现先 spawn 再 `AssignProcessToJobObject`）

降级行为：Job Object 创建或赋值失败 → 终止子进程并返回 `SandboxError`，绝不
让命令在「声称沙箱」下裸跑。

### Linux（`seccomp+ns`）
- `unshare --mount --net --pid --mount-proc` 隔离（namespace）
- cgroups v2 写 `memory.max` / `cpu.max` / `pids.max`
- **seccomp-bpf syscall 过滤未真正接入**：backend 名与 capabilities 仍标
  `syscall-filter`，但当前只靠 namespace+cgroup。真正白名单需 `libseccomp` C
  依赖 + syscall allowlist，列为后续（见路线图）。

### macOS（`seatbelt`）
- 运行时生成 `.sb` profile，按 `full_disk_read` / `writable_paths` / `NetworkPolicy`
  生成 allow/deny 规则
- **Codex 对齐读策略**：`full_disk_read=true`（Plan/Normal/AcceptEdits 默认）时发
  出无路径限制的 `(allow file-read*)`；写仍限 `writable_paths`（workspace + tmp）
- `full_disk_read=false`（Paranoid）时回退到 `readable_paths` + 系统路径白名单

## 如何查看当前后端

```bash
wgenty-code sandbox status
```

输出 `backend_name`、`is_hardware_enforced`、`capabilities`。

## 权限模式 ↔ 沙箱矩阵（Profile Matrix）

Shell 工具（`execute_command` / `exec_command` / `run_test`）按 **EffectiveMode**
解析 `SecurityLevel` + **FailMode**。模式只经 `ToolContext.effective_mode` 传递，
**不是**进程全局锁。

| EffectiveMode | 默认 SecurityLevel | Network（level 默认） | FS 读/写 | 默认 FailMode |
|---------------|-------------------|----------------------|----------|---------------|
| Plan | High | None | **全盘读** + workspace 写（Codex read-only 读策略） | HardFail |
| Normal | Standard | **Full**（cargo/npm/git） | **全盘读** + workspace 写（Codex workspace-write） | HardFail |
| AcceptEdits | Standard（仅 shell；写文件工具不走 OS 沙箱） | **Full** | 同上 | HardFail |
| Yolo | Minimal（metadata） | Full | **OS 沙箱关闭**（非 Minimal seatbelt） | DegradeWithMark |

- **HardFail**：沙箱 spawn/基础设施失败 → `ToolError` `sandbox_spawn_failed`，**绝不**裸跑。
- **DegradeWithMark**：允许直接 spawn，结果 metadata 必含 `sandbox_bypassed=true`；TUI 会话状态栏显示 `⚠ SANDBOX BYPASS`。
- `run_test.allow_network=true` 只保证 `NetworkPolicy::Full`（Standard 已是 Full 时无变化），**不**降低 SecurityLevel。
- `Paranoid` 仅能通过 settings 覆盖获得，不在默认矩阵中。
- **Enforcement fidelity**（metadata `sandbox_enforcement_fidelity`）：`full`（如 macOS seatbelt）/ `partial`（Linux ns、Windows job）/ `none`（NoneBackend 或 bypass）。Level 是 profile 意图，不是跨平台隔离强度保证。

### 设置 `integrations.sandbox`

```json
{
  "integrations": {
    "sandbox": {
      "enabled": true,
      "defaults_by_mode": {},
      "fail_mode_by_mode": {}
    }
  }
}
```

- `enabled: false`：所有模式强制 DegradeWithMark + bypass 标记（用户明确关闭 OS 沙箱）。
- `defaults_by_mode`：可选，按 `plan` / `normal` / `accept_edits` / `yolo` 覆盖 level
  （`minimal` | `standard` | `high` | `paranoid`）。
- `fail_mode_by_mode`：可选，覆盖 `hard_fail` | `degrade_with_mark`。

**Breaking（相对旧版默认 Minimal + 失败即裸跑）：** Normal/AcceptEdits shell 默认
**Standard + Full 网络 + HardFail**；Plan 默认 **High + 无网络 + HardFail**。需要更松
时用 Yolo 或 `defaults_by_mode` / `fail_mode_by_mode` 覆盖。

### CLI

```bash
wgenty-code sandbox status   # backend、fidelity、settings.enabled、各 mode 解析结果
wgenty-code sandbox enable   # 持久化 integrations.sandbox.enabled=true
wgenty-code sandbox disable  # 持久化 enabled=false → 全模式 DegradeWithMark + bypass 标记
```

## 路线图

| 优先级 | 项 |
|--------|----|
| P1 | Windows Restricted Token v1（去危险特权 + 可选 deny SID） |
| P1 | Windows CI：`sandbox::` + `agent::runtime::` 在 Windows runner 跑真机测试 |
| P2 | Linux seccomp-bpf syscall 白名单（libseccomp） |
| P2 | Windows「创建挂起 → 入队 → resume」流程 |

## 测试

- `sandbox::backends::windows::tests`：backend 元数据 +（仅 Windows）Job Object
  创建与 `echo` 执行
- `sandbox::tests`：`SandboxManager` 状态、cleanup 幂等
- `agent::runtime::loop_tests`：mock `LlmPort`/`ToolPort` 验证 loop 控制流
  （与沙箱无关，但覆盖共享 runtime 不回退）
