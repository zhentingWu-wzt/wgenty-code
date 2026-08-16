# Brainstorm Summary: workspace-file-preview

- **日期**: 2026-08-16
- **状态**: 已确认（用户已选定方案并确认技术设计）

## 探索结论（事实基础）

- daemon 现有 `GET /fs/dirs` 仅服务目录选择器（全盘、仅目录），b 端点挂 protected router，bearer 认证由 `routes.rs` 统一施加
- `DaemonState` 持有 `projects: ProjectRegistry`（`main_root()` + `registered_roots()`），`state.rs:1678` 附近已有遍历注册根的现成模式；worktree 列表可经 worktrees 模块获得
- web 端已有完整 tab 体系：`uiStore` 的 `openTabs/activeTabId/openTab/closeTab/moveTab/pruneTabs`，subagent 详情以 `subagent:<nodeId>` 前缀 id 接入
- web 已集成 `react-markdown + remark-gfm`（会话消息）与 shiki（`chat/CodeBlock.tsx`，highlighter 单例 + `codeToHtml`）

## 已确认决策

| # | 决策 | 结论 |
|---|------|------|
| 1 | 预览 UI 组织 | **方案 A**：`preview:<absPath>` 接入现有 uiStore tab 体系，复用全部 tab 交互，按 path 去重 |
| 2 | Markdown 渲染 | 复用 `react-markdown + remark-gfm`，代码块复用 CodeBlock——**零新依赖** |
| 3 | daemon 模块 | 新建 `src/daemon/workspace_files.rs`，与 fs.rs 解耦 |
| 4 | 编辑能力 | **明确不做**（产品决策，详见下方"编辑能力决策记录"）：本地部署下浏览器编辑是降级体验且污染 per-turn checkpoint；替代路径是"引用到对话 + vscode:// 跳转"，留待独立 change 评估 |

## 编辑能力决策记录（产品层）

**结论**：文件编辑从 web 端路线图移除。后续候选改为「引用到对话」与「跳转本地编辑器」两个轻量能力。

**理由**：

1. 产品定位是终端优先的 agent 编排器（README："AI coding agent that lives in your terminal"），daemon 本地部署、本地编辑器就在手边——浏览器编辑在任何本地编辑器面前都是降级体验
2. 成本收益最差：乐观并发/冲突推送/checkpoint 边界污染（人会弄脏 per-turn 回滚点）加起来是编辑器成本的大头，换来的却是在浏览器里手写代码这一与产品核心价值（agent 替你改代码）相悖的动作

**替代承接**（合计成本 < 完整编辑器的 1/10）：

- **引用到对话**：预览中选中行 → 作为上下文插入 prompt。用户在会话中打开文件的真实意图多是"基于这段代码指挥 agent"，这才是该强化的动作
- **跳转本地编辑器**：`vscode://file/<abs path>` URI 一键打开，真要手改的人用真编辑器

**验证信号**：本 change（workspace-file-preview）上线后观察预览 tab 的实际使用数据，作为"监督闭环是否成立"的验证

**重新评估触发条件**：当远程/headless workspace 成为一等部署模式（浏览器成为唯一界面）时，编辑从"不合理"转为"必要"，届时开独立 change 重启评估

## 方案对比记录

- UI 组织：A 接入现有 tab（✅ 一致性最好/改动最小） vs B 独立面板（第二套 tab 语义，割裂） vs C 抽屉（宽度受限，不满足"复用 tab"契约）
- 依赖：复用 react-markdown（✅ 已存在） vs 新引 marked（无必要）

## 风险与缓解

- 文本探测误判（UTF-16/latin-1 文件）→ 降级为 octet-stream + `is_binary` 标记，前端 fallback 提示，不崩
- mtime version 本 change 不消费（只读预览），仅透传，语义留给后续 change
- 超大文本高亮卡顿 → >256KB 降级纯文本
