# ExecutionSession 内层设计:SessionCoordinator + verify-gate

**日期**: 2026-07-19
**状态**: 设计稿 v2(回溯修订:复用 CheckpointStore,不新做 SnapshotStore)
**关联**: C 方案分层(长程自主性闭环)、`2026-07-18-per-turn-file-checkpoint-design.md`

## 1. 背景与目标

wgenty-code 已有 CheckpointStore(turn 级 file capture + rewind + undo 工具),但长程任务缺乏"可中断 / 可恢复 / 可回滚 / 可验证"的执行运行时。本设计定义 ExecutionSession 的**内层**--SessionCoordinator + verify-gate--作为长程自主性闭环(C 方案)的地基。

### 1.1 关键决策:复用 CheckpointStore,不新做 SnapshotStore

现有 CheckpointStore 已实现 turn 级 file_edit pre-edit capture + rewind + Tombstone(新建文件)+ prune,且不依赖 git(非 git 项目也能 file 回退)。内层**不重做快照机制**,而是:

- **file 回退**:复用 CheckpointStore(完全不动,继承非 git 能力)
- **git refs 保护**:SessionCoordinator 记 head + git reset(新,现有不管)
- **exec 覆盖**:靠 git reset(tracked)+ untracked 列表(新建)(新,现有不管 exec)
- **session 串联**:SessionCoordinator 维护 turn 链 + current_turn(新,现有离散 turn)
- **verify-gate**:verify_and_complete 工具(新,核心增量)

**内层的真正增量是 verify-gate + session 串联 + git refs 保护,不是快照本身。** 快照部分复用现有 CheckpointStore,不新做 SnapshotStore / FileBlobStore / declared_side_effects。

### 1.2 C 方案分层

```
comet (流程编排层, skill, 可选) - WHAT
  change 生命周期 / 阶段 / 决策点 / artifacts
    ↓ build 阶段委托
ExecutionSession (执行运行时层, 核心) - HOW
  外层: node 状态机 + 跨会话持久化 + comet-adapter  (后续设计)
  内层: SessionCoordinator + verify-gate  ← 本设计
    ↓
CheckpointStore / sandbox / guardian (机制层, 现有) - 现有零件
```

### 1.3 内层交付

- **可中断**:任意时刻 Ctrl+C,session 状态一致(session.json 原子写)
- **可回滚**:回退到任意 turn(git reset + CheckpointStore::rewind + 删新增 untracked)
- **可验证**:agent 声称完成前,runtime 亲自执行验证命令 + 越界检测,否则拒绝标记 completed

## 2. 解耦边界(与 comet)

### 2.1 核心原则

ExecutionSession 不探测 comet 是否安装。用不用 comet 是**调用方**的决策,ExecutionSession 只接收 `source` 参数和 `hooks` 实例。

### 2.2 三种调用方路径

| 调用方 | 触发 | source | hooks | adapter |
|---|---|---|---|---|
| comet skill | 用户输入 `/comet` | `comet` | `CometHooks` | 激活 |
| agent loop | 正常对话(不输 /comet) | `agent-self` | `DefaultHooks` | 不激活 |
| agent loop | 用户显式"不用 comet" | `agent-self` | `DefaultHooks` | 不激活 |

"装了 comet 但本次不用" = `source=agent-self` + `DefaultHooks`,comet-adapter 不注入。comet 入口是显式 `/comet` 命令,不输入就不触发。

### 2.3 source 流转

- **创建**:调用方决定(`comet` / `agent-self` / `user-direct`)
- **resume**:允许 `comet -> agent-self` 降级(换 DefaultHooks);禁止 `agent-self -> comet`(缺 artifacts)
- comet-adapter 非自动劫持:adapter 只在 comet skill 显式创建 session 时注入

### 2.4 不变式

> ExecutionSession 代码里搜不到 "comet" 字符串(除注释/文档举例)。ExecutionSession 不持有"comet 是否安装"的任何信息,只知道 `source` 参数和 `hooks` trait 实例。

## 3. 内层设计

### 3.1 SessionCoordinator 数据结构与存储

#### 模块定位

新模块 `src/exec_session/`。SessionCoordinator 是 CheckpointStore 的上层协调,不重做 capture。

```
CheckpointStore (现有, src/tools/checkpoint_store.rs, 不动)
  - turn 级 file_edit pre-edit capture + rewind + Tombstone + prune
    ↑ 复用
SessionCoordinator (新, src/exec_session/coordinator.rs)
  - session.json: 串联 turn(parent 链)+ status + source
  - git_refs: 每 turn 记 head
  - untracked 列表: turn 开始前记
  - verify_and_complete: 越界检测 + 跑 commands + session 状态
```

模块依赖:`exec_session` 依赖 `tools::checkpoint_store`(tools 是底层,允许被依赖;未来可抽到 storage 层)。CheckpointStore 完全不动,现有 undo 工具行为不变。

#### session.json 数据结构

```json
{
  "session_id": "es-a3f2c1",
  "source": "agent-self",
  "status": "in_progress",
  "created_at": "...",
  "updated_at": "...",
  "turns": [
    {
      "turn_id": "turn-0",
      "parent": null,
      "checkpoint_turn_id": "ct-abc",
      "git_refs": { "head": "abc1234" },
      "untracked_files": ["temp.tmp"],
      "created_at": "..."
    }
  ],
  "current_turn": "turn-0"
}
```

字段:
- `status`:`in_progress`(默认)/ `completed`(verify 通过)/ `unverified`(兜底)/ `failed`(verify 超 max)
- `turns`:session 的 turn 链,每个 turn 记 CheckpointStore 的 `checkpoint_turn_id` 关联 + `git_refs.head` + `untracked_files`
- `current_turn`:当前游标(供外层跨进程 resume 读)
- `parent`:turn 链(回退时遍历)

存储位置:`<project>/.wgenty-code/snapshots/<session_id>/session.json`(与 CheckpointStore 的 `.wgenty-code/checkpoints/` 分开,session 元数据独立)

#### 关键决策

- CheckpointStore 完全不动(file capture / rewind / prune 复用)
- SessionCoordinator 维护 session 元数据(turn 链 + git refs + untracked),不存 file blob(blob 在 CheckpointStore)
- session.json 原子写(tmp+rename)
- turn 的 `untracked_files`:turn 开始前 `git ls-files --others --exclude-standard`(轻量,非 git 项目为空)
- turn 的 `git_refs.head`:turn 开始前 `git rev-parse HEAD`(非 git 项目为 null)

### 3.2 触发时机(turn 边界,非工具前)

内层在 **turn 边界**操作,不拦截每个工具:

```
turn N 开始前(SessionCoordinator::begin_turn):
  1. 记 git_refs.head(git rev-parse HEAD;非 git 项目跳过)
  2. 记 untracked_files(git ls-files --others --exclude-standard;非 git 跳过)
  3. 关联 CheckpointStore 的当前 turn_id(后续 file_edit 由 CheckpointStore capture)

turn N 内:
  - file_edit/write/apply_patch: CheckpointStore 照常 capture(现有机制,不动)
  - exec_command/background: 不单独 capture(回退靠 git reset + untracked)
  - git_operations: 不单独 capture(回退靠 git reset 到 turn 开始 head)

turn N 结束(SessionCoordinator::end_turn):
  - 封存 turn 条目,更新 current_turn
```

**为什么不在工具前 capture**(回溯修订的关键):
- file 回退:CheckpointStore 已做(file_edit 前 capture),不需重做
- git refs 回退:turn 开始记 head 就够(git reset 撤销整 turn commit)
- exec 改文件回退:git reset(tracked)+ 删 untracked(新建),不需 capture
- **不需要 `declared_side_effects`**(不需知道每个工具改什么,用 git + CheckpointStore)

### 3.3 verify-gate(A 方案)

#### verify_and_complete 工具

```rust
// 内层工具,操作内层轻量 session
verify_and_complete({
  "commands": ["cargo test", "cargo clippy --all-targets -- -D warnings"],
  "expected_changed_files": ["src/cli.rs", "src/memory/list.rs", "tests/memory.rs"]
})
```

工具归属:内层(操作 session.json 状态)。内层 session = turn 链 + verify 状态(轻量);外层 ExecutionSession 在其上加 node 状态机 + 跨会话持久化(重量)。

#### 防编造(机制上杜绝)

工具**不接收 agent 贴的"声称结果"**,只接收 commands + expected_changed_files。runtime 亲自执行 commands 得真实结果,存 verify_log。agent 没机会贴假结果。

第一版只支持命令式验证。非命令式("我手动检查了")无法 runtime 校验,靠 agent loop 兜底标 unverified。

#### 越界检测

- `actual_changed_files` = CheckpointStore 的 session 范围 turn manifest 文件路径并集 + git diff(若 git 项目,tracked 改动)+ untracked 新增(对比 turn 链 untracked_files)
- `expected_changed_files` = agent 声明
- 校验:`actual ⊆ expected`
- 越界 -> gate 失败,返回越界清单

#### 安全

commands 经 guardian 审查 + sandbox 执行,和 exec_command 同等对待。runtime 不裸跑 agent 提供的命令。

#### 失败处理(gate 失败语义:不回退)

**核心原则:gate 失败 ≠ 自动回退。** gate 失败是信号(告诉 agent 哪里没过),不是惩罚(抹掉 agent 的工作);回退是 agent 的显式工具(`rollback_to`),不是 gate 失败的副作用。

失败原因:(1) 命令 exit 非 0;(2) 越界。

**gate 失败后 runtime 只做两件事**

- turn 标记 `failed`,**工作区改动保留**,不抹掉
- 触发 `verify_fail` hook(默认 `AutoRetry{max:2}`),把失败原因(哪个 command exit 非 0、越界清单)回传给 agent

**gate 失败后 agent 的三条路(agent 自主决策,runtime 不强制)**

- (a) **自修正**:看失败原因,调工具修,重新调 `verify_and_complete`--长程自主的主路径
- (b) **主动回退**:agent 判断这个 turn 方向错了,显式调 `rollback_to(turn-N)`,换方向--回退**只在 agent 主动调时发生**
- (c) **升级**:连续失败超 `AutoRetry.max` -> session.status = `failed`,升级给上层编排(comet/plan)或人工介入(现场保留,不回退)

**为什么不自动回退(长程自主性的要求)**

- 长程自主的核心是 agent 能**试错、修正、往前走**;自动回退 = 抹掉试错 = 退化成"每次必须一次做对"的短程循环,违背长程自主
- **保留错误状态比抹掉更有信息量**:agent 看着自己改了什么、verify 为什么失败,在失败状态上往前修;抹掉则丢失这些信息,得从空白重新推理,反而更难修
- AutoRetry 语义:允许 agent 再调 N 次 `verify_and_complete`,**不是** runtime 自动重跑、也**不是** runtime 自动回退

**分层职责(为什么 runtime 不判断方向)**

- runtime(`exec_session` 内层)只管**单 turn 完成是否可证明**--防编造、防越界,是"验收层"
- 方向性跑偏(连续几个 turn 往错方向走)是**上层编排**(comet/plan)判断的,runtime 不介入
- 这正是 `exec_session` 与 comet 解耦的原因--方向判断是编排层职责,核心层不掺合

#### 兜底(unverified)

agent 完成但没调 verify_and_complete。agent loop 检测 session 结束信号(最终回复 / 用户结束 / 超时),session 仍 `in_progress` -> 标记 `unverified`。用户可见"未验证"标记。

#### verify_log

存 `<session_id>/verify_log.json`,记录每次 attempt(commands_run / exit_code / actual / expected / result)+ final_status。

### 3.4 resume L1 协议

#### 回退算法(git reset + CheckpointStore::rewind + 删 untracked)

回退到 turn N 之前:

```
1. if turn_N.git_refs.head != 当前 HEAD:
     git reset --hard <turn_N.git_refs.head>   # 撤销 turn N 及之后的 commit,恢复 tracked
2. CheckpointStore::rewind(turn_N.checkpoint_turn_id)   # 复用现有 rewind:file_edit pre-edit 恢复 + Tombstone 删
3. 删除 untracked: 当前 untracked - turn_N.untracked_files   # 删 turn N 期间新建的 untracked 文件
```

顺序:git reset --hard(tracked 恢复)-> CheckpointStore::rewind(file pre-edit 恢复 + Tombstone 删)-> 删新增 untracked。

边界:
- agent 没 commit(head 没变):跳过 step 1,只 rewind + 删 untracked。常见,零 git 开销
- 非 git 项目:跳过 step 1 和 step 3(git_refs/untracked 为空),只 CheckpointStore::rewind(file 回退仍工作)
- CheckpointStore::rewind 是局部回退(turn N 涉及文件);turn N+1 改的其他 tracked 文件靠 git reset 恢复,非 tracked 不撤销(和现有 undo 一致)

#### 崩溃一致性(L1:单文件原子写)

session.json 写入:tmp + rename 原子。中断时:要么完整 session.json 要么旧版本,不产生半个。resume 时读 session.json,无效则降级。

CheckpointStore 的崩溃一致性已有(现有机制,manifest 最后写 + tmp/rename)。

#### current_turn 游标(内层写,外层读)

session.json 的 `current_turn` 字段。内层每次 end_turn 更新(原子)。内层自己**不读**做 resume(L1:重启游标丢)。外层 ExecutionSession 跨进程 resume 时读,告诉内层从哪个 turn 继续。单向数据流,无循环依赖。

#### 快照失败策略

- git refs 记录失败(git 不可用):降级(跳过 git refs,记 null),不阻断 turn
- session.json 写失败:fail fast,返回错误给 agent
- CheckpointStore capture 失败:沿用现有策略(现有 fail fast)

#### L1 边界

| 能力 | 内层 L1 | 外层 ExecutionSession |
|---|---|---|
| session.json 持久化 | ✓ | - |
| 内存游标 | ✓ | - |
| 跨进程 resume(读游标重建) | ✗(L1 丢) | ✓(读 current_turn) |
| node 状态机 | ✗ | ✓ |
| verify_and_complete | ✓(轻量 session) | 在其上加 node 契约 |
| file capture/rewind | 复用 CheckpointStore | - |

## 4. 不做的事(YAGNI 边界)

- 新做 SnapshotStore / FileBlobStore -- 复用现有 CheckpointStore
- `declared_side_effects` / Tool trait 改动 -- 不需要(用 git + CheckpointStore)
- agent loop 拦截每个工具 -- 只在 turn 边界操作
- 多文件事务一致性(L3,WAL/事务)-- L1 单文件原子足够
- 完整逆向回放(逐个 rewind 后续 turn)-- CheckpointStore::rewind 局部回退 + git reset 够用
- git index 级保护 -- 只记 HEAD
- 非命令式 verify -- 第一版只支持命令式
- 外层 ExecutionSession(node 状态机、跨会话持久化、comet-adapter)-- 后续设计
- exec_command 全量 blob capture -- 靠 git reset + untracked 列表,不 capture

## 5. 向前兼容与扩展点

- session.json 的 `turns` 可扩展 `node_states` 字段(外层 node 状态机)
- `SessionHooks` trait 预留 `pre_node` / `post_node`(外层 node 状态机用)
- `git_refs` 可扩展 `index` 字段(第二版)
- `verify_and_complete` 可扩展 `manual_check` 字段(标"不可校验",第二版)
- SessionCoordinator 与 CheckpointStore 松耦合(通过 checkpoint_turn_id 关联),未来可替换 CheckpointStore 实现或抽到 storage 层

## 6. 风险与权衡

| 风险 | 缓解 |
|---|---|
| 复用 CheckpointStore,耦合 tools 模块 | `exec_session` 依赖 `tools::checkpoint_store`(tools 是底层,允许);未来可抽到 storage 层 |
| git reset --hard 丢弃用户手动改动 | 回退是用户主动选择;guardian 提示"将丢弃工作区改动" |
| 删 untracked 误删用户文件 | 只删"turn 开始后新增的"untracked(对比 `untracked_files` 列表),**不**用 `git clean -fd` |
| 非 git 项目无 git refs/untracked 保护 | 降级到纯 CheckpointStore file 回退(仍工作);符合主场景优先 |
| verify_and_complete agent 忘记调用 | agent loop 兜底标 unverified + prompt 引导 |
| 越界检测 actual 计算不全 | actual = CheckpointStore manifest + git diff(tracked)+ untracked 新增,三源全覆盖 |
| CheckpointStore 未来改动影响内层 | 通过 `checkpoint_turn_id` 松耦合;内层只依赖 rewind/capture 接口,不依赖内部实现 |

## 7. 验收标准(内层完成定义)

- [ ] SessionCoordinator 实现 `begin_turn` / `end_turn` / `rollback_to`,复用 CheckpointStore
- [ ] session.json 含 turns 链(parent)+ git_refs.head + untracked_files + status,原子写(tmp+rename)
- [ ] turn 边界记 git refs + untracked(非 git 降级,记 null/空)
- [ ] verify_and_complete 工具:亲自跑 commands + 越界检测(读 CheckpointStore manifest + git diff + untracked)+ guardian/sandbox
- [ ] verify_fail hook(AutoRetry,不回退)+ agent loop 兜底 unverified
- [ ] 回退算法:git reset --hard(若 head 变)+ CheckpointStore::rewind + 删新增 untracked
- [ ] 崩溃一致性:session.json tmp+rename,中断不半个;resume 读 session.json,无效降级
- [ ] current_turn 游标(内层写外层读)
- [ ] 快照失败策略:git 不可用降级 / session.json 写失败 fail fast
- [ ] 解耦不变式:`exec_session/` 代码无 "comet" 字符串
- [ ] 现有 CheckpointStore / undo 工具测试不受影响(CheckpointStore 不动)

