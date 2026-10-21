# Work-Graph E2E 测试手册

> 如何用真实 LLM 会话实测 Graph Engineering(begin_node / decompose_node / verify_node
> 全链路)。沉淀自 2026-08-23 实测(发现并修复 5 个运行时缺陷),配套缺陷报告见
> `2026-08-23-work-graph-e2e-findings.md`。

## 前置准备

```bash
# 1. 玩具项目(不要在主仓库测——verify_commands 会被真实执行,且边界检查
#    以工作区实际变更为准,主仓库的无关改动会污染 expected_files)
mkdir /tmp/graph-e2e && cd /tmp/graph-e2e
git init && echo seed > seed.txt && git add . && git commit -m seed

# 2. 关键配置(分解场景预算下限:floor 2/单元 + 父级保留 1)
wgenty-code config set agent.exec_session.auto_retry_max 5

# 3. 重启 daemon 并验证二进制新鲜度(见"陷阱"节)
cargo build && wgenty-code daemon stop && wgenty-code daemon &
wgenty-code daemon status   # 期待 "Binary: current"
```

**必须在提示词里点名工具**(`begin_node` / `decompose_node` / `verify_node`)。
模型看得到工具 schema,但默认倾向直接写文件干活;不点名就不会走图。

## 测试矩阵

### M1 — 基础节点生命周期(5 分钟)

```
用 begin_node 工具开启一个节点:goal 是"创建 hello.txt 内容为 hi",
verify_commands 是 ["test \"$(cat hello.txt)\" = hi"],
expected_files 是 ["hello.txt"]。
创建文件后用 verify_node 工具验证。把工具返回的 node_id 和 next_step 原样告诉我。
```

预期:`begin_node → {"node_id":"n1","status":"running"}`;实现产物后
`verify_node → {"next_step":"Complete"}`。

### M2 — 组合器封闭集(P1)

```
用 begin_node 开节点,risk 填 "high",has_test_infra 填 false,goal "写一行脚本",
verify_commands ["./run.sh"]。告诉我工具结果里的 template_id。
```

预期:`template_id: "impl+no-test+review+risk-high"`(组合签名;high 强制 review 门,
no-test 剥离 TestAnchor)。注意:**产物创建后工作区会有未声明的新文件**,verify 前把
它们列入 expected_files 或在同节点声明好,否则边界违规直接 Escalate。

### M3 — 封闭集拒绝(只跑单测,不要用提示词测)

模型无法"诚实"提交非法值——schema enum 让模型自动纠正(实测:让它填 `"extreme"`,
它填了合法的 `"high"`)。模型侧的 schema 是便利性约束,runtime 侧校验才是安全边界。
用单测覆盖:

```bash
cargo test begin_node_tool_rejects_invalid_closed_set_facts
cargo test decompose_node_rejects   # 拒绝矩阵:空/超限 units/非法枚举/超长 goal/深度/预算
```

### M4 — 递归分解(P4 全链)

```
用 begin_node 开节点:goal "实现两个独立小工具",
verify_commands ["ls tool_a.sh tool_b.sh"],
expected_files 列出 tool_a.sh、tool_b.sh(以及工作区已有的未提交改动,见 M2 注意)。
然后用 decompose_node 工具分解成两个单元:
unit 0 goal "创建 tool_a.sh 输出 a",verify_commands ["./tool_a.sh"];
unit 1 goal "创建 tool_b.sh 输出 b",verify_commands ["./tool_b.sh"]。
把每个 unit 的 template_id 和 allocated_max_iter 告诉我,再完成实现并用 verify_node 验证。
```

预期时序与断言:

| 步骤 | 工具返回 | 断言 |
|---|---|---|
| decompose_node | `units[].template_id` / `allocated_max_iter` | `impl+risk-low`;各 2;`child_graph_depth: 1` |
| (实现产物) | — | 记得 `chmod +x` |
| verify_node | `{"next_step":"Complete"}` | 单元锚点真实执行(见下) |

verify 后读审计确认全链证据:

```bash
python3 - <<'EOF'
import json, glob
ws = json.load(open(sorted(glob.glob('.wgenty-code/checkpoints/*/work_state.json'),
      key=__import__('os').path.getmtime)[-1]))
for u in ws.get('decomposed_units', []):
    o = u.get('outcome') or {}
    print(u['unit_id'], 'passed=', o.get('passed'), 'attempts=', o.get('attempts_used'))
for r in ws.get('unit_specialist_reports', []):
    print('report:', r['summary'][:60])
print('budget:', ws.get('budget'))
EOF
```

**全绿基线**(修复 `7bb5db21` 后):两单元 `passed=True attempts=1`;两条
`GeneralPurpose/implementation` 报告;父预算 `iter_used=1`(仅提案消耗)。
审计 `decomposed` 事件含命令级 exit code。

### M5 — 失败路径(反向验证)

- 单元必败:某单元 verify_commands 给 `["false"]` → 该单元 `passed=False attempts=2`
  (烧满分配),兄弟单元照常;全部失败时父级 iter_used +1。
- 节点 Escalate:`next_step: "Escalate"`,节点转 Failed;之后 `begin_node` 可直接
  开新节点(死锁已修,`ab97e9b6`),或 `rollback_node` 丢弃 Failed 节点工作。

## 边界检查语义(实测最大坑源)

`verify` 的成功 = 命令 exit 0 **且** `actual_changed_files ⊆ expected_files`。变更集是
**会话级**三源并集:checkpoint manifest ∪ `git diff --name-only <会话起点>` ∪ 新增
untracked。推论:

- 工作区里**任何**未提交改动(包括别的会话留下的)都会计入——开节点前
  `git status --porcelain` 全量列进 expected_files,或先清理。
- Rust 项目会被 `VerificationProfile` 自动扩充锚点(`cargo check` / `cargo test --all`
  / `cargo clippy -D warnings`),即使你只声明了一条 verify 命令——这些命令**真实执行**,
  主仓库测试前确认它们能过。

## 已知陷阱(全部实测踩过)

1. **daemon 二进制 inode 陷阱**:cargo 重建换 inode,运行中的 daemon 继续跑旧代码,
   `lsof` 显示的却是新路径。编译**之后**再重启;每次重启后跑 `daemon status` 看
   `Binary: current`(T3 修复后 STALE 会直接标出)。
2. **rollback 会吞节点期间的提交**:节点开始后有 >1 个提交(含基础设施提交)时,
   rollback 现在会拒绝并列出 subject(T2 修复);=1 个提交是 agent 标准工作流,会被
   回滚。测试循环"修 bug → commit → 重测 → rollback"注意顺序。
3. **预算下限**:分解 N 单元需要 `auto_retry_max >= 2N + 1`(floor 2/单元 + 父级
   保留 1 + 提案消耗 1)。默认 2 只够不分解的场景。
4. **配置路径**:`agent.exec_session.auto_retry_max`(嵌在 `agent` 下)。写错路径
   现在 `config set` 会直接报 `unknown setting key`(T4 修复)。

## 快速回归清单

```bash
cargo test --all                        # 全量(基线 1875 lib + 218 integration)
cargo test org_graph                    # 图逻辑:组合矩阵/适配/审计
cargo test decompose                    # 分解:拒绝矩阵/分账/注入
cargo test exec_session::node_runtime   # 运行时:单元执行/回滚/死锁恢复
```
