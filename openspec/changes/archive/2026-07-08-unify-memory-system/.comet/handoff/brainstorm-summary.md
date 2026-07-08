# Brainstorm Summary

- Change: unify-memory-system
- Date: 2026-07-08

## 确认的技术方案

以 compaction 为锚点，将四个孤立部件统一为 Agent Loop 记忆循环：

```
会话启动 → consolidate (AutoDream gate) → recall (keyword search) → 注入 prompt
  → 正常对话 (流式 SSE) → compaction → extract (非流式 chat, JSON {summary, memories})
  → memories 持久化, summary 替换 history
```

六个关键设计决策：
1. **压缩时提取**：不额外调 LLM，复用 compaction 调用
2. **关键词召回**：零延迟，embedding 字段预留未来升级
3. **一次 LLM 双输出**：JSON `{summary: string, memories: [{type, content, importance}]}`
4. **compaction 改用非流式 `chat()`**：保证 JSON 完整性，正常对话保持流式
5. **AutoDream 简化**：只做门控触发，去重委托给 MemoryManager
6. **移除 ContextWindow**：与 conversation_history 完全重复

## 关键取舍与风险

| 风险 | 缓解 |
|------|------|
| JSON parse 失败 | fallback: 全文作 summary，跳过提取 |
| 记忆膨胀 | ConsolidationEngine max_memories + importance 阈值 |
| 关键词召回质量 | 当前够用；embedding 字段已预留 |
| 启动延迟 | per-file JSON <100ms；consolidation 仅 gate 通过时 |

## 测试策略

- 单元测试：JSON parse 成功/失败、空/非空 memories 注入、AutoDream gate 通过/失败
- 集成测试：compaction → memory 文件、session restart → prompt 中出现 memories
- 回归：`cargo test` + `cargo clippy` 通过

## Spec Patch

无 — 所有需求已在 specs/agent-memory/spec.md 中完整定义
