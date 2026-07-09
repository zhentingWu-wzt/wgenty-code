# 根治:历史回放层 lenient_json 校验,杜绝截断 tool_call 毒化后续请求

## 背景
GLM-5.2(max_tokens 已调 65536)下,`file_write` 生成超大中文 content 时仍可能被输出上限截断 -> `arguments` 变非法 JSON -> 该 assistant 消息原样存入历史 -> 后续每轮请求都被方舟以 `InvalidParameter: Invalid request body` 拒绝。`src/utils/lenient_json.rs` 能检测截断,但当前只用于工具执行,未用于「请求体序列化前」。

## 关键设计决策:替换 arguments,而非丢弃 tool_call
用户原文提「丢弃该 tool_call 或要求模型重发」。**丢弃会孤儿化配对的 `tool` 响应消息**(它引用被丢弃的 `tool_call_id`),触发*另一种* `InvalidParameter`。

采用:**把非法 `arguments` 替换为合法 JSON 对象**(lenient 提取的部分字段,剥离 `_parse_error`/`_raw_arguments` 元键;无可提取字段时退化为 `{}`)。保留 `tool_call.id` -> 配对 tool 响应仍链接 -> payload 合法。模型看到「我调了 file_write({}) -> 报错」会自我纠正。仍满足「不原样回放」。

## Part A — 回放消毒(根治)— `src/api/mod.rs`
新增私有函数:
```rust
/// 发送前遍历 messages,把任何非法 JSON 的 assistant tool_call.arguments
/// 替换为合法 JSON(lenient 部分字段,退化到 `{}`)。保留 id 以免孤儿化
/// 配对 tool 响应。直接 serde 解析失败 = 被 max_tokens/流中断截断。
fn sanitize_tool_call_args_for_replay(messages: &mut [ChatMessage]) {
    for m in messages.iter_mut() {
        if m.role == "assistant" {
            if let Some(tcs) = m.tool_calls.as_mut() {
                for tc in tcs.iter_mut() {
                    if serde_json::from_str::<serde_json::Value>(&tc.function.arguments).is_err() {
                        let (partial, err) = crate::utils::lenient_json::parse_tool_args_lenient(
                            &tc.function.arguments, &tc.function.name);
                        let cleaned = match partial {
                            serde_json::Value::Object(mut map) => {
                                map.remove("_parse_error"); map.remove("_raw_arguments");
                                serde_json::Value::Object(map)
                            }
                            _ => serde_json::Value::Object(serde_json::Map::new()),
                        };
                        tracing::warn!(tool=%tc.function.name, error=%err.unwrap_or_default(),
                            "replay: replaced truncated/invalid tool_call arguments with valid JSON");
                        tc.function.arguments = cleaned.to_string();
                    }
                }
            }
        }
    }
}
```
调用点(2 处,`messages` 改 `mut`):
- `chat()`(mod.rs:105)dispatch 前调用
- `chat_stream()`(mod.rs:213)dispatch 前调用

覆盖全部 4 个内部方法(openai_compat/anthropic × stream/非stream),以及 anthropic 的 `convert_messages_to_anthropic(&messages)`(在消毒后读取)。

## Part B — 工具执行器暴露真因 — `src/tui/agent/core.rs`
当前截断时 lenient 提取不到 `path` -> file_write 返回误导的 `"path is required"`(真因是 args 非法)。`parse_err.is_some()` ⟺ args 非 JSON(经 preprocess 仍失败)。

**顺序路径**(core.rs:305-312 warn 块之后,314 ask_user 分支之前)插入短路:
```rust
if let Some(ref e) = parse_err {
    let msg = format!(
        r#"{{"success":false,"error":"tool call arguments are invalid JSON (likely truncated by max_tokens): {e}. Please re-issue the tool call."}}"#);
    let _ = self.event_tx.send(AppEvent::ToolResult {
        name: tc.function.name.clone(), args: args.clone(), content: msg.clone() });
    self.conversation_history.lock().await.push(ChatMessage::tool(&tc.id, msg));
    continue;
}
```
模式参照 timeout 短路(core.rs:411-430)。

**并行 task 路径**(core.rs:252 附近):同样的 `parse_err` 守卫,parse 失败时 push 错误结果而非用垃圾 args spawn task。

## Part C — 测试
`src/api/mod.rs` 测试模块:
1. `sanitize_replay_replaces_truncated_args`:assistant 带截断 arguments + 配对 tool 消息 -> 消毒后 assistant.args 为合法 JSON、id 不变、tool 消息仍链接。
2. `sanitize_replay_leaves_valid_args`:合法 arguments 原样不动。
3. `sanitize_replay_empty_when_unextractable`:中段截断无可提取字段 -> args == `{}`。

## 改动文件
- `src/api/mod.rs`:+1 函数、2 处调用点、+3 测试
- `src/tui/agent/core.rs`:2 处 parse_err 短路守卫

## 风险
低-中。消毒是附加性逻辑(仅对已非法的 args 生效);Part B 仅改变不可恢复 args 的行为(此前本就产生误导错误)。两者都改善失败模式。改后 `cargo check && cargo test`。

## Comet
无活跃 change(`fix-subagent-timeout-default` 已归档)。直接实现。如需正式追踪,事后可包装为新 comet change。
