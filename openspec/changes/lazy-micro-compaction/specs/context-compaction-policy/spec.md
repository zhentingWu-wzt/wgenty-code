## ADDED Requirements

### Requirement: 请求视图默认逐字保留工具结果

水位线以下时，系统 SHALL 在 API 请求视图中逐字保留历史消息（含工具结果），不做每轮压实替换。

#### Scenario: 低负载轮次不产生占位符

- **WHEN** 校准后的 token 估算不超过水位线（上下文窗口 3/5 减 max_tokens）
- **THEN** 请求视图中的每条工具结果与原始 history 内容一致
- **AND** 不出现 `[Previous: used …]` 占位符

### Requirement: 水位触发的单调压实前沿

系统 SHALL 仅在视图 token 估算超过水位线时推进压实前沿，将前沿之前（最新 3 条工具结果之外）的工具结果替换为占位符；前沿 SHALL 单调前进，两次触发之间视图 SHALL 对历史 append-only。

#### Scenario: 超水位触发一次性压实

- **WHEN** 视图估算首次超过水位线且历史含 6 条工具结果
- **THEN** 除最新 3 条外的工具结果替换为占位符
- **AND** 最新 3 条保持逐字

#### Scenario: 触发后追加消息不翻转既有前缀

- **WHEN** 压实触发后新的一轮追加 1 条工具结果且未再超水位线
- **THEN** 上一轮视图是本轮视图的前缀（旧内容无翻转）
- **AND** 新结果逐字追加

#### Scenario: 全量压缩后前沿跟随边界

- **WHEN** 全量 LLM 摘要压缩推进了 compaction boundary
- **THEN** 压实前沿不低于新边界

### Requirement: 压实保留错误摘要

压实失败工具结果时，系统 SHALL 在占位符中保留截断的错误摘要；成功结果的占位符格式 SHALL 保持不变。

#### Scenario: executor 错误保留 message

- **WHEN** 被压实的结果为 `{"success":false,"error":{"message":"Patch context not found…","code":"context_not_found"}}`
- **THEN** 占位符含 `used <tool>; error:` 与该 message 的截断摘要

#### Scenario: 字符串形态错误同样保留

- **WHEN** 被压实的结果为 `{"success":false,"error":"task tool call arguments are invalid JSON…"}`
- **THEN** 占位符含该字符串的截断摘要

#### Scenario: hook 拦截文本保留首行

- **WHEN** 被压实的结果以 `Tool 'x' blocked by hook: …` 开头
- **THEN** 占位符含该首行的截断摘要

#### Scenario: 成功结果占位符不变

- **WHEN** 被压实的结果为 `{"success":true,…}`
- **THEN** 占位符为 `[Previous: used <tool>]`，不含错误后缀

### Requirement: file_read 结果豁免压实

系统 SHALL 在压实前沿之内继续逐字保留 `file_read` / `read_file` 工具结果。

#### Scenario: 前沿内读取结果不压缩

- **WHEN** 一条 file_read 结果位于压实前沿之前
- **THEN** 其内容在请求视图中逐字保留

### Requirement: 压实策略零回归

系统 SHALL 保持：全量压缩在实际发送视图上评估、413 应急压实与摘要器输入路径行为不变、会话持久化保存完整原始 history。

#### Scenario: 全量压缩评估输入不变

- **WHEN** 每轮评估是否需要全量 LLM 摘要压缩
- **THEN** 评估基于该轮实际发送的请求视图

#### Scenario: 会话文件不含占位符

- **WHEN** 任一压实触发后保存会话
- **THEN** 持久化的 history 中工具结果仍为原始完整内容
