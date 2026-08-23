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

### T1 — Failed 节点死锁(高)✅ 已修(ab97e9b6)

`begin_node` 对 Failed(终态)放行,新节点以当前工作区为新基线;`rollback_node` 无
Verified 锚点时回退到最后一个 Failed 节点作为丢弃锚点(回滚其 start_turn 并连同
移除)。Running(非终态)仍拒绝两条路径。

### T2 — rollback 静默吞节点期间的外部提交(高)✅ 已修(1a6b351d)

HEAD 与 turn 起点 SHA 之间 >1 个提交时 rollback 拒绝并列出 subject(=1 为 agent
标准工作流,放行);untracked 清理无条件跳过 `.wgenty-code/`,runtime 状态不再被
自身回滚删除。

### T3 — daemon 重启与二进制 inode 陷阱(中,工程)✅ 已修

`daemon status` 现输出 `Binary: STALE/current`:比对磁盘二进制 mtime 与 daemon 启动
时间,启动后重建过的二进制不可能在跑(实测当天即捕获一次真实 STALE)。

### T4 — `config set` 不校验路径(低)✅ 已修

serde 默认丢弃未知字段,`config set exec_session.auto_retry_max 5` 会"成功"写入
无效顶层键。现在 set 后做 round-trip 导航验证:路径不在 schema 中 →
`unknown setting key` 报错;真实嵌套路径不受影响。

### T5 — 测试手册沉淀(低)✅ 已完成

见 `2026-08-23-work-graph-e2e-testplaybook.md`:测试矩阵 M1–M5、边界检查语义、
四大陷阱(二进制 inode / rollback 吞提交 / 预算下限 / 配置路径)、审计断言脚本。

模型侧无法"诚实"测试封闭集拒绝(schema enum 使模型自动纠正非法值)——该场景只跑单测。
提示词驱动必须点名工具名(`begin_node`/`decompose_node`),否则模型直接干活不走图。
使用分解时建议 `auto_retry_max >= 5`(floor 2/单元 + 父级保留 1)。

## 复现基线

干净链路(见 7bb5db21 后实测):begin_node(声明全部 expected)→ decompose_node(2 单元)
→ 写产物 → verify_node → 单元各 1 次通过 → 父级 Rust profile 锚点(cargo check/
test --all/clippy)全绿 → Complete;审计含单元命令级 exit code 与终态路由。
