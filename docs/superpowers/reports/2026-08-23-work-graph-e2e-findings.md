# Work-Graph E2E 实测发现的缺陷与待办

> 2026-08-23 实测记录:P4 递归工作图全链路 E2E(driver: 真实 daemon + 真实命令 + 真实 checkpoint)。
> 修复的 5 个缺陷已入 dev;2 个死锁类缺陷与 3 个工程改进为待办。

## 已修复(dev)

| # | 缺陷 | 提交 |
|---|---|---|
| 1 | verify 前分解:预算未初始化直接拒(种子只在 verify 路径) | 86a4736c |
| 2 | 分账公式无父级保留 cap:单单元 `max(2, remaining)` 吃光预算,永久拒绝 | 86a4736c |
| 3 | daemon 硬编码 `auto_retry_max=2`,settings 完全无效(两条装配路径之一漏接线) | bd3b8178 |
| 4 | `run_decomposed_units` 无 agent 入口:分解后单元永远 pending,verify 只跑父级锚点 | 25bab223 |
| 5 | 单元判定误用会话级边界:`exit 0` 被判失败,烧光分配;父级边界是唯一裁决 | 7bb5db21 |

## 待办(按风险排序)

### T1 — Failed 节点死锁(高)

节点 Escalate 后无 Verified 锚点时:`begin_node` 拒(前置校验)、`rollback_node` 拒(no
verified node to roll back to)——会话永久卡死,只能手工改 session.json。
**建议**:允许"丢弃当前 Failed 节点并清其 turn"的显式操作(如 `rollback_node` 的
`--discard-failed` 语义,或 begin_node 对 Failed 前置放行并自动截断)。

### T2 — rollback 静默吞节点期间的外部提交(高)

`rollback_to` 的 Stage 1 对"HEAD ≠ 节点起点"一律 `git reset --hard`,包括与节点无关的
基础设施提交(实测:两个修复提交被吞,靠 reflog 才找回;连带 `.wgenty-code/checkpoints/`
untracked 目录被清,导致 rollback 自身死锁)。
**建议**:(a) 回滚前检测 HEAD 领先节点起点 >1 个提交时拒绝/警告;(b) untracked 清理排除
`.wgenty-code/`(runtime 状态不应被自身回滚删除)。

### T3 — daemon 重启与二进制 inode 陷阱(中,工程)

cargo 重建替换二进制 inode;运行中的 daemon 继续映射旧 inode,`lsof` 显示的路径却是新的
——"重启了但修复没生效"极易误判(实测连续踩坑 3 次)。
**建议**:(a) daemon status 输出进程二进制 inode + 启动时间;(b) `daemon stop` 前比对
磁盘 inode,不一致时提示"二进制已更新,需重启";(c) 文档记录"编译后再重启"的顺序要求。

### T4 — `config set` 不校验路径(低)

`config set exec_session.auto_retry_max 5` 静默写入无效顶层键(真实路径是
`agent.exec_session.auto_retry_max`),无任何报错。
**建议**:校验 key 路径存在于 Settings schema,未知路径报错。

### T5 — 测试手册沉淀(低)

模型侧无法"诚实"测试封闭集拒绝(schema enum 使模型自动纠正非法值)——该场景只跑单测。
提示词驱动必须点名工具名(`begin_node`/`decompose_node`),否则模型直接干活不走图。
使用分解时建议 `auto_retry_max >= 5`(floor 2/单元 + 父级保留 1)。

## 复现基线

干净链路(见 7bb5db21 后实测):begin_node(声明全部 expected)→ decompose_node(2 单元)
→ 写产物 → verify_node → 单元各 1 次通过 → 父级 Rust profile 锚点(cargo check/
test --all/clippy)全绿 → Complete;审计含单元命令级 exit code 与终态路由。
