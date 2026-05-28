# ratatui 前端替代 TypeScript+React 前端 — 设计文档

## 动机

当前 TypeScript+React (Ink) 前端增加了用户的安装门槛：需要 Node + npm + Rust 两套工具链。目标是将产品转为**单二进制分发**，用户 `brew install` 或直接下载 GitHub Releases 二进制即可运行。

## 架构

```
claude-code (单二进制)
  ├── tokio::spawn → axum daemon server (127.0.0.1:随机端口)
  ├── 等待 daemon ready (GET /api/v1/health)
  └── main thread → ratatui Terminal + App::run()
                      ├── ApiClient (reqwest, HTTP → daemon)
                      └── AgentLoop (SSE 流解析 + 工具调用循环)
```

- **单进程，双 tokio task**：daemon 后台运行，UI 主线程
- **随机端口**：避免端口冲突，通过 `DaemonHandle` 传递 base_url
- **优雅关闭**：UI 退出时发 shutdown signal 给 daemon task
- **daemon 代码零改动**：当前 900 行 daemon 完全保留

## ratatui 组件树

```
App (主布局)
├── HeaderBar        —— 会话名 + 当前模型 + 快捷键提示
├── ChatView         —— 消息列表（可滚动 viewport）
│   └── MessageRow   —— 单条消息气泡（user/assistant/tool/system）
├── StatusBar        —— 状态文字（thinking/streaming/idle）+ token 计数
├── InputBox         —— ▸ prompt 输入行，tui-textarea，CJK/IME 兼容
├── TaskPanel        —— 可折叠 todo 面板
├── WelcomeBanner    —— 首次启动 ASCII art
│
└── [Floating Popups]
    ├── PermissionPopup —— y/n/a 权限确认
    ├── QuestionPopup   —— ask_user_question 交互
    └── SessionPopup    —— 会话列表/搜索
```

## Agent Loop 移植

`agent-loop.ts`（766行）→ `src/tui/agent.rs`

核心逻辑完全保留：
1. `process_input()` → 构建 messages → `chat_stream()` → SSE 逐行解析
2. StreamProcessor 流式解析（已有 Rust 参考实现 `src/agent/`）
3. tool_calls 检测 → `execute_tool_with_permission()` → 权限弹窗 → 重试
4. `ask_user_question` → question 弹窗 → 结果注入
5. 微压缩（microCompact）+ 自动压缩（doAutoCompact）
6. 网络错误重试（最多 2 次，指数退避）
7. TodoWrite 追踪 + nag reminder（s03）
8. 后台任务结果注入

和 daemon 通信：HTTP `POST /api/v1/chat/stream` + `POST /api/v1/tools/execute`，与当前 TS → daemon 一致。

## 滚动稳定性

ratatui 的差分渲染天然避免全屏重刷问题：
- **消息列表区域**：仅在 `committedMessages` 变化时重绘
- **流式内容**：独立区域渲染，每帧只更新这一个段落
- **滚动**：`Paragraph::scroll` 显式控制 offset，用户手动上翻时不受新内容影响
- 不会出现 React + Ink 中的虚拟 DOM diff 导致的全 ANSI 重绘

## IME 兼容

两层策略：
1. `tui-textarea` 使用 crossterm `KeyEventKind` 区分 IME preedit 和已提交按键；支持 kitty keyboard protocol（WezTerm/Kitty/iTerm2）
2. 兜底：对不支持增强键盘协议的终端（如 macOS Terminal.app），通过 feature flag 切换为 cooked mode `read_line` + channel 方式

验收终端：iTerm2、WezTerm、macOS Terminal.app

## 数据流

### SSE 流式响应
```
用户输入 → AgentLoop::process_input()
  → ApiClient::chat_stream(messages)
    → POST /api/v1/chat/stream
    → reqwest bytes_stream
    → StreamProcessor::feed_bytes() → StreamEvent
      ├── Content(delta)     → 追加 streamingContent → UI tick
      ├── Reasoning(delta)   → 追加 reasoningContent
      ├── ToolCallDelta      → 累积 tool_calls
      ├── Done(finish_reason)→ flush → committedMessages
      └── StreamError(msg)   → 重试 or 错误
```

### 工具执行 + 权限
```
tool_calls 检测
  → POST /api/v1/tools/execute
  → 返回 { permission_required? }
    ├── Yes → PermissionPopup
    │   ├── deny  → 注入错误结果
    │   └── allow/always → POST /api/v1/tools/approve → 重试
    └── No → 直接注入 tool role message
```

### UI Tick 驱动
```rust
loop {
    tokio::select! {
        event = input_rx.recv() => { /* 用户输入 */ }
        token = stream_rx.recv() => { /* SSE token */ }
        _ = tick_interval.tick() => { /* 空闲刷新 */ }
    }
    terminal.draw(|f| { /* 渲染各组件 */ });
}
```

## 项目结构

```
Cargo.toml                      # +ratatui 0.29, +tui-textarea 0.7, +crossterm 0.28
src/
├── tui/                        # 新建
│   ├── mod.rs
│   ├── app.rs                  # 应用主循环 + 布局
│   ├── agent.rs                # AgentLoop
│   ├── sse.rs                  # SSE 解析
│   ├── client.rs               # HTTP client
│   ├── theme.rs                # 颜色/样式
│   └── components/
│       ├── mod.rs
│       ├── chat.rs             # ChatView + MessageRow
│       ├── input.rs            # InputBox (tui-textarea)
│       ├── status.rs           # StatusBar
│       ├── permission.rs       # PermissionPopup
│       ├── question.rs         # QuestionPopup
│       ├── session.rs          # SessionPopup
│       ├── task_panel.rs       # TaskPanel
│       └── welcome.rs          # WelcomeBanner
├── main.rs                     # 修改：默认启动 tui 模式
└── daemon/                     # 不变
```

## 迁移阶段

| 阶段 | 内容 | 验收标准 |
|------|------|----------|
| 1. 骨架 | ratatui 布局 + daemon 后台启动 + 健康检查 | `cargo run` 可见 TUI 框架 |
| 2. Agent | AgentLoop + SSE + ApiClient | 能发送消息、看到流式回复 |
| 3. 组件 | 权限/问题弹窗、会话管理、任务面板 | 完整功能交互 |
| 4. 打磨 | 滚动稳定性、CJK/IME、错误恢复、主题 | 与当前 TS 版 feature parity |

## 新增依赖

```toml
ratatui = "0.29"
tui-textarea = "0.7"
crossterm = "0.28"
```

`reqwest` 和 `tokio-stream` 已在 daemon feature 中存在，无需新增。

## 构建产物

```bash
cargo build --release  # → target/release/claude-code（单二进制）
```

零外部依赖，无需 Node / npm。

## 不做的

- 不修改 daemon 任何代码
- 不删除 TypeScript 前端（保留在 `packages/` 中，不再作为默认构建目标）
- 不实现前后端共存的双 UI 维护模式——ratatui 是唯一 CLI 前端
