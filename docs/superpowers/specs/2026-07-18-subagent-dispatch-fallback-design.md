---
comet_change: subagent-dispatch-fallback
role: technical-design
canonical_spec: openspec
archived-with: 2026-07-18-subagent-dispatch-fallback
status: final
---

# Design: Subagent Dispatch Fallback

## Context

当前 subagent 派发失败的处理是"把结构化错误码交给父 agent 模型自行决定",无自动兜底。两类失败路径不同:

- **派发前失败**(`task.rs:897` `map_coordinator_error`):深度超限/并发关闭/任务组添加失败 -> 返回 `ToolError` 给父模型。不经 `ChildTerminal`,不经 `collect_children_for_synthesis`。
- **运行时失败**(`subagent_loop.rs:823-865`):超时/卡住/panic/模型错误 -> `ChildTerminal::Failed`,父 agent 通过 `collect_children_for_synthesis`(`coordinator.rs:1155`,`JoinPolicy::BestEffort`)收到失败 `ChildResult`。

模型不可用目前被压成 `ErrorType::Unknown`(`subagent_loop.rs:858`),因为 `RuntimeError::Stream("API error ...")` 不含 "stuck"/"timeout"。这是 deepseek-reasoner 不可用问题的根因。

核心约束:Comet build 阶段隔离规则禁止主会话/根协调者直接执行 task。fallback 执行者必须是派发该子 agent 的父 agent(本身是 subagent)。

## Goals / Non-Goals

**Goals**
- 模型不可用 + 派发前结构性失败时,由父 agent 接管执行,使派发失败不再等于任务失败。
- 模型类失败自动切换备用模型;结构性失败不换模型。
- 单次 fallback、不可递归。
- 细化失败分类,让"模型不可用"可被决策器识别。
- 与 Comet 隔离规则兼容。

**Non-Goals**
- 不对超时/卡住/max轮/panic 做 fallback(保留现状交父模型决定)。
- 不改父作用域取消语义。
- 不做多级 fallback 链 / 递归重试。
- 不改 `subagent-driven-development` 双审查流程。
- 不实现根协调者/主会话执行路径。

## Architecture: Dual Interception Points

混合方案 -- 两个拦截点分别对应两种执行形态,因为两类失败的代码路径天然不同。

```
拦截点 1(派发前失败)-> 候选 B:TaskTool 内兜底同步执行
  位置:TaskTool::execute_with_context,reserve_child_in_group 返回结构失败后
  触发:CoordinatorError = DepthLimitReached | ConcurrencyClosed | TaskGroup
  执行:用 self.api_client + self.tool_registry 同步跑 full_prompt(task.rs:491 拼好,作用域内)
  prompt 来源:full_prompt,派发点直接可得(子 agent 从未运行,无 transcript)
  工具集:父 agent(TaskTool self)当前工具集
  结果:作为 task 工具调用的返回值,父 agent 模型无感

拦截点 2(运行时模型失败)-> 候选 A:再派发降级子 agent
  位置:collect_children_for_synthesis 返回后,SubagentSynthesis::on_candidate_final 之前
  触发:ChildResult.status=Failed 且 error_code=subagent_model_unavailable
  执行:换 models.main.name(复用原 endpoint),再派发降级子 agent
  prompt 来源:transcript_store.get_by_id(child_id).user_prompt
  工具集:filter_allowed_tools(同原派发的子 agent 类型/深度)
  结果:新 ChildTerminal,进入父 agent synthesis
```

两个拦截点共用 `fallback_eligible` 判定,但 `FallbackKind` 不同。

## Components

### 1. ErrorType 扩展(`src/agent/progress.rs`)

`ErrorType` 新增 `ModelUnavailable` 变体。`SubagentError::code()`(`subagent_loop.rs:113`)新增映射 `ModelUnavailable -> "subagent_model_unavailable"`。

### 2. 失败分类细化(`src/teams/subagent_loop.rs:845-858`)

`RuntimeError::Stream(msg)` 分类 match 扩展:
- `msg.contains("API error") || 含 HTTP 状态码模式 || msg.contains("connection")` -> `ErrorType::ModelUnavailable`
- 其余 `Stream` -> `ErrorType::Unknown`(维持)
- `StreamTimeout` -> `Timeout`(维持)
- `MaxRoundsExceeded` -> `Stuck`(维持)

### 3. fallback_eligible 判定(新模块 `src/agent/fallback.rs`)

```rust
pub enum FallbackKind { ModelUnavailable, Structural }

// 拦截点 1 来源
pub fn fallback_eligible_from_coordinator_error(e: &CoordinatorError) -> Option<FallbackKind>
// 拦截点 2 来源
pub fn fallback_eligible_from_child_result(r: &ChildResult) -> Option<FallbackKind>
```

- `DepthLimitReached` / `ConcurrencyClosed` / `TaskGroup` -> `Some(Structural)`
- `ChildResult.error_code = "subagent_model_unavailable"` -> `Some(ModelUnavailable)`
- `subagent_timeout` / `subagent_stuck` / `subagent_cancelled` / `subagent_error`(非模型)/ `subagent_tool_error` / `subagent_parse_error` -> `None`

### 4. fallback_used 标记(`src/agent/task_group.rs` GroupRecord)

`GroupRecord` 新增 `fallback_used: HashMap<AgentId, bool>`(per-child,不进 `ChildResult`,不被序列化传递)。方法:`mark_fallback_used(child_id)` / `fallback_already_used(child_id) -> bool`。

派发前失败(child 可能尚未 reserve)的标记键策略留 build 阶段定(候选:label+session 临时键,或在 reserve 失败前先用 pending 键)。

### 5. 备用模型 settings(`src/config/mod.rs`)

新增 `fallback_model_settings(&self, model_name: &str) -> Self`,类比 `small_model_settings`(`config/mod.rs:140`)但只 override `models.main.name`,保留 `base_url`/`api_key`/`appkey`/`provider`。复用原 endpoint;若 endpoint 不通则 fallback 失败(由单次约束终止)。

### 6. 配置键

`agent.subagent.fallback_models: Vec<String>` -- 模型名有序列表。模型类失败时按序取第一个 != 失败子 agent 原模型的项;无配置或空 -> 降级交根模型(等同现状)。

## Data Flow

### 拦截点 1(候选 B)

```
TaskTool::execute_with_context
  -> 构造 full_prompt(task.rs:491)
  -> coordinator.reserve_child_in_group(...)
  -> 若 Err(e):
       fallback_eligible_from_coordinator_error(&e)?
         | None -> map_coordinator_error -> 返回 ToolError(现状)
         | Some(Structural):
             guard: !is_root(caller) && !fallback_used(child)
               | root 或已 used -> 返回 ToolError(交根模型)
               | 通过:
                   同步执行 full_prompt(self.api_client, self.tool_registry, 父工具集)
                   mark_fallback_used(child)
                   成功 -> 结果作为 task 工具返回
                   失败 -> 返回 ToolError(交根模型,不递归)
```

### 拦截点 2(候选 A)

```
collect_children_for_synthesis -> Vec<ChildResult>
  -> for each Failed child:
       fallback_eligible_from_child_result(&r)?
         | None -> 交父模型(现状)
         | Some(ModelUnavailable):
             guard: !is_root(caller) && !fallback_used(child)
               | root 或已 used -> 交根模型
               | 通过:
                   选备用模型(fallback_models 首个 != 原模型)
                     | 无配置 -> 降级交根模型
                     | 有:
                         settings = fallback_model_settings(name)
                         api_client = ApiClient::new(settings)
                         prompt = transcript_store.get_by_id(child_id)?.user_prompt
                           | 无 transcript -> 降级交根模型,记日志
                         allowed_tools = filter_allowed_tools(...)
                         run_subagent_loop_with_permissions(... 备用 api_client, prompt ...)
                         mark_fallback_used(child)
                         成功 -> 新 ChildTerminal::Completed
                         失败 -> 交根模型(不递归)
```

## Error Handling

- 备用模型也失败(endpoint 不通)-> 单次约束,交根模型,不递归。
- transcript 读不到 prompt -> 降级交根模型,记日志。
- 拦截点 1 同步执行失败 -> 返回 ToolError 交根模型,不递归。
- 派发前失败 child 未 reserve 的标记键 -> build 阶段定策略。
- root 派发的 child -> is_root 守卫拦截,直接交根模型(隔离规则)。

## Testing

**单元**:
- `ModelUnavailable` 分类:`Stream("API error (503): ...")` -> ModelUnavailable;`Stream("connection refused")` -> ModelUnavailable;`Stream("other")` -> Unknown;`StreamTimeout` -> Timeout。
- `fallback_eligible`:各 `CoordinatorError` 变体、各 `error_code` 字符串。
- `fallback_model_settings`:只换 name,base_url/api_key 不变。
- `fallback_used`:per-child 独立,置位后第二次 already_used。
- `is_root` 守卫:root caller 不触发。

**集成**:
- 模型失败换备用成功(mock 首模型 503,备用成功)。
- 深度超限 TaskTool 兜底成功。
- fallback 失败不递归(备用也 503)。
- root 派发不 fallback。
- 无备用配置降级交根模型。
- 派发前失败(ConcurrencyClosed)TaskTool 兜底。

**观测**:
- tracing:fallback 触发(失败类型/候选 A or B/备用模型名)/成功/失败。
- `subagent_health.rs` `FailureMode::classify` + `ModelUnavailable` 桶。

## Risks / Trade-offs

- **[Risk] 字符串匹配 `RuntimeError::Stream` 易碎** -> Mitigation:匹配多特征(`"API error"` + HTTP 状态码 + `"connection"`)。Open question:是否给 `RuntimeError` 加结构化 `ModelUnavailable` variant。
- **[Risk] 备用模型用原 endpoint,endpoint 不通则 fallback 失败** -> Mitigation:单次约束终止,交根模型(预期行为,用户已确认)。
- **[Risk] 候选 B 用父工具集可能缺专用工具** -> Mitigation:用户已接受;B 是兜底,优于完全不完成。
- **[Risk] transcript 无 prompt 记录(边缘)** -> Mitigation:降级交根模型,记日志。
- **[Trade-off] 双拦截点比单一复杂** -> 接受,因两类失败代码路径天然不同,双路径语义最干净。
- **[Trade-off] 候选 B 落 TaskTool 内而非侵入父 loop** -> 接受,实现最简,父模型无感。

## Migration Plan

1. 纯增量:新增 `ErrorType::ModelUnavailable`、`fallback_eligible`、双拦截点、配置项。未配置 `fallback_models` 时,模型类失败仍降级为现状(交父模型),行为不变。
2. 配置灰度:先放一个备用模型(如 claude),验证 deepseek 失败场景;稳定后再加链。
3. 回滚:删除配置项即恢复现状;fallback 路径是新增分支,移除不影响原有失败交付。

## Open Questions

1. 派发前失败 child 未 reserve 的 `fallback_used` 标记键策略(build 阶段定)。
2. 是否给 `RuntimeError` 加结构化 `ModelUnavailable` variant 取代字符串匹配(build 阶段评估侵入度)。
3. 候选 B 同步执行的 token budget 归属(计入父 agent 预算 vs 独立)。
4. 候选 A 再派发的降级子 agent 是否计入父的并发上限(`agent.subagent.max_concurrent`)。
