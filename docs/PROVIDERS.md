# Provider 支持矩阵

Wgenty Code 通过 `ApiClient` + `Provider` trait 支持多家模型供应商，`resolve_provider()`
按 `base_url` / 显式 `provider` 字段自动路由。本文记录各 provider 的能力与已知差异，
供「按模型裁剪工具 / 提示」与质量回归参考。

## 已支持 provider

| Provider | `name()` | OpenAI-compat | 端点 | 备注 |
|----------|----------|---------------|------|------|
| Anthropic | `anthropic` | 否（原生 `/v1/messages`） | `api.anthropic.com` 等 | system prompt 单独字段；tool 用 Anthropic schema |
| DeepSeek | `deepseek` | 是 | `api.deepseek.com` | **必须回传 `reasoning_content`**（后续请求） |
| DashScope | `dashscope`（经 OpenAI-compat） | 是 | `dashscope.aliyuncs.com` | 阿里云通义系列 |
| OpenAI-compat（通用） | `openai` | 是 | 任意 relay | 显式 `provider=openai` 覆盖 |

## 路由规则（`resolve_provider`）

1. 显式 `provider` 字段优先（`anthropic` / `deepseek` / `openai`，大小写不敏感）
2. `base_url` 含 `anthropic` -> Anthropic
3. `base_url` 含 `deepseek` -> DeepSeek
4. 其余 -> OpenAI-compat

模型别名映射（`sonnet`/`haiku`/`opus` 等）由各 provider 的 `resolve_model_id` 完成。

## 能力矩阵（设计与实测目标）

> 标注为「✅ 已验证」「⚠️ 已知差异」「🔲 待测」。实测需在各 provider 真实 key 下跑
> `tests/integration` 与手动 `query` 场景；本表是回归基线，非一次性结论。

| 能力 | Anthropic | DeepSeek | DashScope | OpenAI-compat relay |
|------|-----------|----------|-----------|---------------------|
| 流式 SSE 解析 | ✅ | ✅ | ✅ | ✅ |
| tool_calls（function calling） | ✅ | ✅ | ⚠️ | ⚠️ relay 透传 |
| `reasoning_content` 回传 | n/a | ✅（必须） | ⚠️ | ⚠️ |
| 长上下文（>100k） | ✅ | ⚠️ 视模型 | ⚠️ | ⚠️ |
| 上下文压缩后继续 | ✅ | ⚠️ | ⚠️ | ⚠️ |
| Plan mode（无 tools 摘要） | ✅ | ✅ | ⚠️ | ⚠️ |
| 子代理（task 工具） | ✅ | ⚠️ tool 稳定性 | ⚠️ | ⚠️ |

## 已知差异与处理

- **DeepSeek `reasoning_content`**：`StreamProcessor` 累积 `reasoning_content`，
  压缩时纳入 transcript 文本（截断到 1000 字符/条），避免摘要丢失思考过程。
- **Anthropic 格式**：`convert_messages_to_anthropic` / `convert_tools_to_anthropic`
  负责转换；system prompt 提取为单独字段。
- **InvalidParameter 防护**：压缩后首条非 system 消息强制为 `user`（OpenAI-compat
  拒绝 assistant 开头）；`max_tokens` 预留避免 input+output 溢出窗口。
- **tool 参数 JSON 截断**：lenient 解析 + 连续 3 次不可恢复中止（共享 loop 统一策略）。

## 按模型裁剪工具

`ToolRegistry::with_settings()` 已有按 provider 移除不兼容工具的钩子。建议维护一张
「provider × 工具」表，对 `DashScope` 等关闭不稳定的元工具（如并行 `task`）。

## 回归建议

新增 provider 或改 `Provider` trait 时，至少跑：
- 流式 + 非流式各一轮 tool-use
- 压缩触发后继续对话
- 长输出（接近 `max_tokens`）截断行为
- `reasoning_content` 在 history 中正确回传
