# web 聊天记录按 turn 分组展示 - 设计文档

- 日期：2026-08-06
- 状态：已批准（待实现）
- 范围：web 前端 ChatView 渲染层

## 1. 动机

web 端 ChatView 把 DisplayMessage[] 扁平平铺（均等 gap-5），没有 turn 级视觉分组：不同 turn（用户输入 -> 完整回复）之间、同一 turn 的多个 assistant round 之间都只有均等间距，回复和工具调用"挤在一起"。TUI（src/tui/components/chat.rs）通过 turn separator（实线）+ inline separator（虚线）实现轮次分组，web 应对齐这一体验。

## 2. 方案

分隔线方案（approach A：map 插分隔线），纯渲染层改动，不改数据模型。

### 2.1 turn 分隔线

ChatView 的 messages.map 里，当 m.role === "user" 且 index > 0 时，在消息前渲染一条淡色水平分隔线（border-t border-border + my-2），分隔不同 turn。

### 2.2 turn 内间距收紧

外层容器从 gap-5 改为 gap-2，turn 内消息更紧凑；turn 边界靠分隔线突出。

### 2.3 工具调用

保持现有 ToolCallCard（附在 assistant message 下方），toolExecs 间 gap-1.5 不变。多个 round 的工具调用靠各自 assistant message 容器自然分隔。

### 2.4 round 标记

m.round > 1 的 "· round N" 文字标注保留，标识 turn 内子轮次。

## 3. 不改动

- DisplayMessage 数据模型不变（不加 turnId）
- loop.ts / sessionRunner.ts / sessionLoad.ts 不变
- ToolCallCard / CodeBlock / Markdown 不变

## 4. 受影响文件

| 文件 | 改动 |
|------|------|
| web/src/features/chat/ChatView.tsx | import Fragment；外层 gap-5 -> gap-2；map 用 Fragment 包裹，user 消息前加分隔线 |

## 5. 验证

- tsc 类型检查 / vitest
- 手动：多轮对话后观察 turn 间分隔线、turn 内消息紧凑、工具调用清晰分组
