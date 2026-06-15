---
comet_change: rlm-observability-and-robustness
role: technical-design
canonical_spec: openspec
archived-with: 2026-06-15-rlm-observability-and-robustness
status: final
---

# RLM 可观测性与鲁棒性增强 — 技术设计

## 1. CompletionEngine — TUI 输入框命令补全

### 1.1 架构

```
┌──────────────────────────────────────────────────┐
│                    TUI App                        │
│  ┌──────────────────────────────────────────────┐│
│  │         completion_panel.rs                   ││
│  │  ┌────────────────────────────────────────┐  ││
│  │  │ > @com                                  │  ││
│  │  │   ┌──────────────────────────────────┐  │  ││
│  │  │   │ comet          │ ← highlighted   │  │  ││
│  │  │   │ comet-open     │                  │  │  ││
│  │  │   │ comet-design   │                  │  │  ││
│  │  │   └──────────────────────────────────┘  │  ││
│  │  └────────────────────────────────────────┘  ││
│  └──────────────────────────────────────────────┘│
│                      ↑ AppEvent                   │
│  ┌──────────────────────────────────────────────┐│
│  │            input_reader.rs                     ││
│  │  @ prefix → CompletionTrigger                  ││
│  │  / prefix → CompletionTrigger                  ││
│  └──────────────────────────────────────────────┘│
│                      ↑                            │
│  ┌──────────────────────────────────────────────┐│
│  │         CompletionEngine (new)                 ││
│  │  - skills: Vec<SkillEntry>                     ││
│  │  - commands: Vec<PluginCommandEntry>            ││
│  │  - filter(prefix: &str) → Vec<Match>            ││
│  └──────────────────────────────────────────────┘│
└──────────────────────────────────────────────────┘
```

### 1.2 组件设计

**`CompletionEngine`**（`src/tui/components/completion_panel.rs` + 数据层在 `src/tui/completion.rs`）：

```rust
pub struct CompletionEngine {
    skills: Vec<SkillEntry>,       // 启动时从 ~/.claude/skills/ 扫描
    commands: Vec<CommandEntry>,   // 从 PluginRegistry.commands 加载
}

pub struct SkillEntry {
    pub name: String,              // e.g., "comet-design"
    pub description: String,       // 从 SKILL.md 提取首行
    pub path: PathBuf,             // 技能目录路径
}

pub struct CommandEntry {
    pub name: String,              // e.g., "code-review"
    pub description: String,       // 命令描述
    pub args_hint: Option<String>, // 参数提示，如 "<change-name>"
}
```

**数据来源**：
- Skills：遍历 `~/.claude/skills/` 下的子目录，读取每个目录名作为 skill name；description 从 `SKILL.md` 的 `description:` frontmatter 或第一段提取
- Plugin commands：通过 `PluginRegistry.commands` 获取 `HashMap<String, PluginCommand>`

**过滤逻辑**：
```rust
impl CompletionEngine {
    fn filter(&self, prefix: char, partial: &str) -> Vec<CompletionMatch> {
        match prefix {
            '@' => self.skills.iter()
                .filter(|s| s.name.to_lowercase().contains(&partial.to_lowercase()))
                .map(|s| CompletionMatch { text: s.name.clone(), description: s.description.clone(), .. })
                .sorted_by_key(|m| m.text.clone())
                .collect(),
            '/' => self.commands.iter()
                .filter(|c| c.name.to_lowercase().starts_with(&partial.to_lowercase()))
                .map(|c| CompletionMatch { text: c.name.clone(), description: c.description.clone(), args_hint: c.args_hint.clone() })
                .sorted_by_key(|m| m.text.clone())
                .collect(),
            _ => vec![],
        }
    }
}
```

### 1.3 AppEvent 扩展

在 `src/tui/app/event.rs` 新增三个事件变体：

```rust
pub enum AppEvent {
    // ... existing variants ...
    CompletionTrigger { prefix: char, partial: String },
    CompletionSelect { index: usize },
    CompletionDismiss,
}
```

### 1.4 输入框状态机

在 `src/tui/app/types.rs` 的 `App` 状态增加：

```rust
pub struct App {
    // ... existing fields ...
    pub completion_state: Option<CompletionState>,
}

pub struct CompletionState {
    pub prefix: char,               // '@' or '/'
    pub partial: String,            // 已输入的过滤文本
    pub matches: Vec<CompletionMatch>,
    pub selected_index: usize,
    pub visible: bool,
}
```

**快捷键映射**：
| 按键 | 行为 |
|------|------|
| `@`（非行首）| 触发 skill 补全 |
| `/`（行首或空格后）| 触发 plugin 命令补全 |
| `↑` / `↓` | 导航候选项 |
| `Tab` | 下一项（循环） |
| `Shift+Tab` | 上一项（循环） |
| `Enter` | 确认选中：替换 `@xxx` 为完整 skill/command 名 |
| `Esc` | 关闭补全面板 |

### 1.5 补全面板渲染

`CompletionPanel` 组件（`src/tui/components/completion_panel.rs`）复用 `PermissionState` 的 inline panel 渲染模式：

```rust
pub struct CompletionPanel;

impl CompletionPanel {
    pub fn render(f: &mut Frame, area: Rect, state: &CompletionState) {
        // 1. 计算面板尺寸（最多显示 8 项，超出可滚动）
        // 2. 边框使用橙色高亮（与 @ 前缀颜色一致）
        // 3. 高亮当前选中项
        // 4. 显示 description（dimmed）
        // 5. 底部显示快捷键提示
    }
}
```

**渲染位置**：输入框上方，消息区域上移。

### 1.6 Ink CLI 侧适配

在 `packages/cli/src/components/input-box.tsx` 中：

- `useInput` hook 中检测 `@` 和 `/` 前缀
- 通过 IPC/WebSocket 发送 `CompletionTrigger` 事件到 daemon
- 渲染补全下拉列表（Ink 的 `<Box>` 组件）
- 快捷键处理与 Rust TUI 侧一致

### 1.7 边界条件

- `~/.claude/skills/` 目录不存在时：`CompletionEngine.skills` 为空，`@` 触发的补全列表为空，显示 "No skills found" 提示
- PluginRegistry 未加载时：`CompletionEngine.commands` 为空，`/` 触发同上
- 输入中同时存在 `@` 和 `/` 时：以最后一个触发前缀为准
- 补全面板打开时收到其他 `AppEvent`（如 `ContentDelta`）：补全面板自动关闭

archived-with: 2026-06-15-rlm-observability-and-robustness
status: final
---

## 2. Subagent Transcript 持久化

### 2.1 模块结构

```
src/transcript/
├── mod.rs          # SubagentTranscriptStore 公开接口
└── store.rs        # SQLite 实现
```

### 2.2 数据库 Schema

数据库文件：`~/.wgenty-code/subagent_transcripts.db`

```sql
-- 执行记录头表
CREATE TABLE IF NOT EXISTS subagent_transcripts (
    id TEXT PRIMARY KEY,                    -- UUID v4
    session_id TEXT NOT NULL,               -- 所属会话 ID
    parent_id TEXT,                         -- 父节点 ID（NULL = root）
    label TEXT NOT NULL,                    -- 人类可读标签
    status TEXT NOT NULL,                   -- 'pending' | 'running' | 'completed' | 'failed' | 'cancelled'
    system_prompt TEXT,                     -- subagent 的 system prompt（可选）
    user_prompt TEXT NOT NULL,              -- 用户/父 agent 的输入
    started_at INTEGER NOT NULL,            -- Unix 毫秒
    finished_at INTEGER,                    -- Unix 毫秒
    total_tokens INTEGER DEFAULT 0,         -- 累计 token 消耗
    input_tokens INTEGER DEFAULT 0,         -- 输入 token 数
    output_tokens INTEGER DEFAULT 0,        -- 输出 token 数
    max_rounds INTEGER,                     -- 最大轮数
    actual_rounds INTEGER DEFAULT 0,        -- 实际执行轮数
    token_budget INTEGER,                   -- token 预算（千）
    error_message TEXT,                     -- 错误信息
    summary TEXT,                           -- 最终结果摘要
    created_at INTEGER DEFAULT (unixepoch('now'))
);

-- 事件明细表
CREATE TABLE IF NOT EXISTS subagent_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    transcript_id TEXT NOT NULL REFERENCES subagent_transcripts(id) ON DELETE CASCADE,
    round INTEGER NOT NULL,                 -- 第几轮
    event_type TEXT NOT NULL,               -- 'thought' | 'action' | 'tool_result' | 'error' | 'completion'
    tool_name TEXT,                         -- 工具名（action 类型时）
    tool_params TEXT,                       -- 工具参数 JSON（action 类型时）
    data TEXT NOT NULL,                     -- 事件详情 JSON
    elapsed_ms INTEGER NOT NULL,            -- 从 subagent 开始到本事件的毫秒偏移
    token_count INTEGER,                    -- 本事件的 token 消耗
    created_at INTEGER DEFAULT (unixepoch('now'))
);

-- 索引
CREATE INDEX IF NOT EXISTS idx_transcripts_session ON subagent_transcripts(session_id, started_at DESC);
CREATE INDEX IF NOT EXISTS idx_transcripts_status ON subagent_transcripts(status);
CREATE INDEX IF NOT EXISTS idx_events_transcript ON subagent_events(transcript_id, round);
CREATE INDEX IF NOT EXISTS idx_events_type ON subagent_events(event_type);
```

### 2.3 Store API

```rust
pub struct SubagentTranscriptStore {
    db: Connection,  // rusqlite::Connection
}

impl SubagentTranscriptStore {
    /// 打开或创建数据库，自动执行 migration
    pub fn open(path: &Path) -> Result<Self, TranscriptError>;

    /// 保存完整 transcript（header + all events），在单个事务中执行
    pub fn save(&self, transcript: &SubagentTranscript) -> Result<(), TranscriptError>;

    /// 更新 transcript header 的中间状态（用于 checkpoint）
    pub fn checkpoint(&self, id: &str, round: u32, tokens: u64) -> Result<(), TranscriptError>;

    /// 追加单条事件（用于 checkpoint）
    pub fn append_events(&self, transcript_id: &str, events: &[SubagentEvent]) -> Result<(), TranscriptError>;

    /// 按 session 列出所有 transcript（仅 header，不含 events）
    pub fn list_by_session(&self, session_id: &str) -> Result<Vec<SubagentTranscriptHeader>, TranscriptError>;

    /// 按 ID 获取完整 transcript（含所有 events）
    pub fn get_by_id(&self, id: &str) -> Result<Option<SubagentTranscript>, TranscriptError>;

    /// 搜索 transcript（按 label 模糊匹配）
    pub fn search(&self, query: &str) -> Result<Vec<SubagentTranscriptHeader>, TranscriptError>;

    /// 执行保留策略：删除超过 retention_days 的旧记录
    pub fn cleanup(&self, retention_days: u32) -> Result<usize, TranscriptError>;
}
```

### 2.4 写入策略

```
执行中                              完成/失败时
────────                            ──────────
Round 0  ──→ 内存缓存 events
Round 1  ──→ 内存缓存 events
...                                 批量写入 header + all events
Round 9  ──→ checkpoint: 写入前 10 轮  ──→ 完成时写入剩余 events
Round 10 ──→ 内存缓存 events
...
Round N  ──→ 批量写入 N-10..N events
```

- **Checkpoint 策略**：每 10 轮写入一次中间状态（仅 header 更新 + events 追加）
- **完成时**：在一个事务中写入/更新 header 和全部未写入的 events
- **Failed/Cancelled**：立即 flush 所有 events（不等待 checkpoint）
- **保留策略**：每次 save 后触发 cleanup，删除超过 `max_transcript_age_days`（默认 30）的记录

### 2.5 错误处理

- 数据库文件无法创建（权限/磁盘满）：降级为仅内存，打印 warning 日志
- 写入失败：不阻塞 subagent 执行，记录 error 日志
- 数据库损坏：自动删除旧文件并重建

archived-with: 2026-06-15-rlm-observability-and-robustness
status: final
---

## 3. Subagent 执行事件模型扩展

### 3.1 SubagentEvent 枚举

在 `src/agent/progress.rs` 扩展：

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SubagentEvent {
    Thought {
        text: String,            // 模型的完整 thinking 文本
        elapsed_ms: u64,
    },
    Action {
        tool_name: String,
        params: serde_json::Value,  // 完整参数，不再截断
        params_summary: String,     // TUI 展示的摘要（~80 chars）
        elapsed_ms: u64,
    },
    ToolResult {
        tool_name: String,
        success: bool,
        summary: String,            // 结果摘要（~200 chars）
        elapsed_ms: u64,
    },
    Error {
        message: String,
        error_type: ErrorType,      // Timeout | BudgetExceeded | Stuck | ToolError | ParseError | Unknown
        elapsed_ms: u64,
    },
    Completion {
        status: String,             // 'completed' | 'failed' | 'cancelled'
        summary: Option<String>,
        elapsed_ms: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ErrorType {
    Timeout,
    BudgetExceeded { limit_k: u64, used: u64 },
    Stuck { reason: String },
    ToolError { tool: String, message: String },
    ParseError { message: String },
    Unknown,
}
```

### 3.2 SubagentProgress 字段变更

```rust
pub struct SubagentProgress {
    // ... 现有字段保持不变 ...
    pub action_log: Vec<SubagentEvent>,   // 移除 50 条截断 → 完整保留
    pub text_snapshot: Option<String>,    // 移除 200 chars 截断 → 完整存储（TUI 层截断显示）

    // 新增字段
    pub progress_delta: Option<f32>,      // 新增：进展增量
    pub token_budget_k: Option<u64>,      // 新增：token 预算
    pub cumulative_tokens: u64,           // 新增：累计 token
    pub error_details: Option<ErrorInfo>, // 新增：错误详情
    pub events: Vec<SubagentEvent>,       // 新增：完整事件流（替代旧 action_log）
}

pub struct ErrorInfo {
    pub error_type: ErrorType,
    pub message: String,
    pub last_tool: Option<String>,
    pub last_params: Option<String>,
    pub round: u32,
    pub retryable: bool,
}
```

### 3.3 事件流与 TUI 渲染解耦

- 内存中的 `SubagentProgress.events` 保留完整列表直到 subagent 完成
- 完成时批量写入 SQLite，然后可清理内存中的 events（保留 header 摘要）
- TUI panel 渲染时从内存直接读取（Running/Completed 节点），detail view 从 SQLite 读取（历史节点）
- 对于 Running 节点：实时追加 events，TUI 渲染最近 N 条（窗口化显示）

archived-with: 2026-06-15-rlm-observability-and-robustness
status: final
---

## 4. Subagent 错误可视化与恢复

### 4.1 SubagentPanel 内联展开

**失败节点渲染流程**：

1. `SubagentPanel` 在渲染节点列表时，检查 `selected_index` 对应的节点是否 `status == Failed`
2. 若选中 Failed 节点 → 在节点下方渲染内联展开区域：
   ```
   ├─ ❌ sub: fix auth bug — Failed
   │  ┌────────────────────────────────────────────┐
   │  │ Error: Subagent timed out after 240s        │
   │  │ Last tool: file_read("src/auth/login.rs")   │
   │  │ Tokens: 12.5k/50k · Round: 8/20             │
   │  │ [r] retry  [d] details  [Esc] close         │
   │  └────────────────────────────────────────────┘
   ```

3. 如果选中 Completed 节点 → 显示摘要行（绿色），不展开

### 4.2 SubagentPanelState 状态扩展

```rust
pub struct SubagentPanelState {
    // ... 现有字段 ...
    pub detail_view: Option<DetailViewState>,
}

pub struct DetailViewState {
    pub transcript_id: String,
    pub scroll_offset: usize,      // 当前滚动位置
    pub events: Vec<SubagentEvent>, // 从 SQLite 加载的事件列表
    pub loading: bool,             // 是否正在加载
}
```

### 4.3 Detail View（时间线视图）

**布局**：
```
┌──────────────────────────────────────────────────────┐
│ 📋 sub: fix auth bug                    ✅ Completed  │
│                                                      │
│ ⏱ 66.2s  🔄 14 rounds  📊 307.0k tokens  🛠 28 calls│
│                                                      │
│ ─── Event Timeline ─────────────────────────────────│
│                                                      │
│ +0.2s  💭 Analyzing the skills directory...           │
│ +1.5s  🔧 grep("skill", path="src/")                 │
│ +3.2s     ✓ 12 matches in 4 files                     │
│ +4.1s  🔧 file_read("src/skills/registry.rs")        │
│ +5.8s     ✓ 245 lines read                            │
│ ...                                                   │
│ +66.2s 🏁 Completed: Found 42 skill definitions       │
│                                                      │
│ ↑↓ scroll · PgUp/PgDn page · g/G top/bottom · Esc back│
└──────────────────────────────────────────────────────┘
```

- 左侧 header 栏：状态/耗时/tokens/rounds/findings
- 右侧时间线滚动区域：事件图标 + 时间偏移 + 内容
- 事件图标：💭 (thought)、🔧 (action)、✓ (tool_result)、❌ (error)、🏁 (completion)
- 时间偏移：相对于 subagent 开始的 `+Xs` 格式

### 4.4 选择性回滚机制

**回滚范围**：只回滚出错步骤涉及的文件，保留之前成功步骤的修改。

**实现**：

```rust
// 在 task.rs 或 subagent_loop.rs 中
pub struct RollbackContext {
    stashed_ref: String,           // git stash 的 ref
    affected_files: Vec<PathBuf>,  // 本次 subagent 修改的文件列表
    parent_commit: String,         // 父节点的 git commit SHA
}

impl RollbackContext {
    /// subagent 开始修改文件前调用，创建 safety point
    pub fn create(label: &str) -> Result<Self, RollbackError>;

    /// 回滚到 safety point（仅恢复 affected_files）
    pub fn rollback(&self) -> Result<(), RollbackError>;

    /// 成功完成后释放 safety point
    pub fn release(&self) -> Result<(), RollbackError>;
}
```

**工作流**：

```
Subagent Start
  ↓
create_safety_point()  ← git stash push --include-untracked
  ↓
Round 1: read files only
  ↓
Round 2: modify file_a.rs  ← 记录 affected_files += "file_a.rs"
  ↓
Round 3: modify file_b.rs  ← 记录 affected_files += "file_b.rs"
  ↓
Round 4: ERROR (timeout)
  ↓
rollback() → git checkout affected_files from stash
  ↓
Retry: previous_attempt_error 注入 system prompt
```

**边界条件**：
- Subagent 仅执行了只读操作 → 无需回滚，直接重试
- 工作区有未提交改动 → 提示用户先提交/stash，禁止回滚
- stash 冲突（用户手动 stash）→ 报告冲突，建议手动处理

### 4.5 重试逻辑

```rust
async fn retry_subagent(
    failed_transcript: &SubagentTranscript,
    task_registry: &TaskRegistry,
) -> Result<SubagentResult, RetryError> {
    // 1. 从 SQLite 读取失败节点的完整 transcript
    // 2. 检查是否需要 rollback（如果有文件修改）
    // 3. 构建重试 prompt：
    //    - 原始 user_prompt
    //    - 注入 previous_attempt_error: { error_type, message, last_tool, round }
    //    - 注入 previous_partial_result（已完成的分析步骤）
    // 4. 重新 spawn subagent（复用原始 token_budget 和 max_rounds）
    // 5. 新的 subagent 使用新的 node_id，parent_id 指向原节点
}
```

archived-with: 2026-06-15-rlm-observability-and-robustness
status: final
---

## 5. RLM 结构化归约

### 5.1 格式定义

**模块**：`src/tools/meta/rlm/formats.rs`

```rust
// structured-claims/1
#[derive(Debug, Serialize, Deserialize)]
pub struct ClaimsOutput {
    pub format: String,  // "structured-claims/1"
    pub claims: Vec<Claim>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<ClaimsMetadata>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Claim {
    pub id: String,
    pub claim: String,
    pub evidence: String,
    pub confidence: f32,                // 0.0 - 1.0
    pub conflicts_with: Vec<String>,    // 冲突的 claim IDs
    pub actionable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recommendation: Option<String>,
}

// unified-diff/1
#[derive(Debug, Serialize, Deserialize)]
pub struct DiffOutput {
    pub format: String,  // "unified-diff/1"
    pub changes: Vec<FileChange>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FileChange {
    pub file: String,
    pub intent: String,                 // 变更意图
    pub diff: String,                   // unified diff 字符串
    pub confidence: f32,
    pub depends_on: Vec<String>,        // 依赖的其他文件路径
}
```

### 5.2 格式选择逻辑

在 RLM Planner 中根据任务类型注入输出格式指令：

```rust
fn inject_format_instruction(task_type: TaskType, prompt: &mut String) {
    match task_type {
        TaskType::Analysis => {
            prompt.push_str("\n\nOUTPUT FORMAT: structured-claims/1 JSON.\n");
            prompt.push_str("Your output MUST be valid JSON matching the structured-claims schema...");
        }
        TaskType::Modification => {
            prompt.push_str("\n\nOUTPUT FORMAT: unified-diff/1 JSON.\n");
            prompt.push_str("Your output MUST be valid JSON matching the unified-diff schema...");
        }
        TaskType::Mixed => {
            prompt.push_str("\n\nOUTPUT: Produce TWO sections:\n");
            prompt.push_str("1. ANALYSIS in structured-claims/1 JSON format...\n");
            prompt.push_str("2. CHANGES in unified-diff/1 JSON format...\n");
        }
    }
    // 追加 schema definition 到 prompt 中
    prompt.push_str(&get_schema_definition(task_type));
}
```

### 5.3 分层降级解析

```rust
impl ClaimsOutput {
    pub fn parse(text: &str) -> Result<Self, ParseError> {
        // Level 1: 尝试直接 JSON 解析
        if let Ok(claims) = serde_json::from_str::<Self>(text) {
            return Ok(claims);
        }
        
        // Level 2: Regex 提取 JSON 块（处理 LLM 在代码块中包裹 JSON 的情况）
        let re = Regex::new(r#"```(?:json)?\s*(\{[\s\S]*?"format"\s*:\s*"structured-claims/1"[\s\S]*?\})\s*```"#)?;
        if let Some(caps) = re.captures(text) {
            if let Ok(claims) = serde_json::from_str::<Self>(&caps[1]) {
                return Ok(claims);
            }
        }
        
        // Level 2b: 尝试提取任意 JSON 对象
        let re_loose = Regex::new(r#"\{[\s\S]*?"claims"\s*:\s*\[[\s\S]*?\][\s\S]*?\}"#)?;
        if let Some(caps) = re_loose.captures(text) {
            if let Ok(claims) = serde_json::from_str::<Self>(&caps[0]) {
                return Ok(claims);
            }
        }
        
        // Level 3: Fallback — 保留原文 + [unstructured] 标记
        Ok(ClaimsOutput {
            format: "structured-claims/1".into(),
            claims: vec![Claim {
                id: "unstructured-1".into(),
                claim: text.to_string(),
                evidence: String::new(),
                confidence: 0.5,
                conflicts_with: vec![],
                actionable: false,
                recommendation: None,
            }],
            metadata: Some(ClaimsMetadata {
                parse_method: "unstructured-fallback".into(),
                parse_warning: Some("Failed to parse structured output; preserving raw text".into()),
            }),
        })
    }
}
```

### 5.4 Aggregator 合并逻辑

```rust
pub struct Aggregator;

impl Aggregator {
    pub fn merge(results: Vec<SubtaskResult>) -> AggregatorOutput {
        let claims = Self::extract_all_claims(&results);
        let changes = Self::extract_all_changes(&results);
        
        // 1. Claims 去重：Jaccard 相似度 > 0.8 → 合并
        let deduped = Self::deduplicate_claims(claims, 0.8);
        
        // 2. 冲突检测：conflicts_with 引用解析
        let conflicts = Self::detect_conflicts(&deduped);
        
        // 3. 文件冲突检测：同文件多个 diff → 标记 write_conflict
        let file_changes = Self::merge_file_changes(changes);
        
        // 4. 仅对无法 resolve 的冲突 fallback LLM
        let unresolved = conflicts.iter()
            .filter(|c| c.status == ConflictStatus::Unresolved)
            .collect::<Vec<_>>();
        
        AggregatorOutput {
            claims: deduped,
            conflicts,
            file_changes,
            needs_llm_fallback: !unresolved.is_empty(),
            unresolved_items: unresolved,
        }
    }
    
    fn deduplicate_claims(mut claims: Vec<Claim>, threshold: f64) -> Vec<Claim> {
        let mut result = Vec::new();
        let mut merged_ids = HashSet::new();
        
        for i in 0..claims.len() {
            if merged_ids.contains(&claims[i].id) { continue; }
            for j in (i+1)..claims.len() {
                if merged_ids.contains(&claims[j].id) { continue; }
                if jaccard_similarity(&claims[i].claim, &claims[j].claim) > threshold {
                    // 合并：保留更高 confidence，拼接 evidence
                    if claims[j].confidence > claims[i].confidence {
                        claims[i].confidence = claims[j].confidence;
                    }
                    claims[i].evidence = format!("{}; {}", claims[i].evidence, claims[j].evidence);
                    merged_ids.insert(claims[j].id.clone());
                }
            }
            result.push(claims[i].clone());
        }
        result
    }
    
    fn detect_conflicts(claims: &[Claim]) -> Vec<ConflictEntry> {
        let mut conflicts = Vec::new();
        let claim_map: HashMap<&str, &Claim> = claims.iter()
            .map(|c| (c.id.as_str(), c)).collect();
        
        for claim in claims {
            for conflict_id in &claim.conflicts_with {
                if let Some(target) = claim_map.get(conflict_id.as_str()) {
                    conflicts.push(ConflictEntry {
                        claim_a_id: claim.id.clone(),
                        claim_b_id: target.id.clone(),
                        status: ConflictStatus::Unresolved,
                    });
                }
            }
        }
        conflicts
    }
    
    fn merge_file_changes(changes: Vec<FileChange>) -> Vec<FileChangeResult> {
        let mut by_file: HashMap<String, Vec<FileChange>> = HashMap::new();
        for change in changes {
            by_file.entry(change.file.clone()).or_default().push(change);
        }
        
        by_file.into_iter().map(|(file, file_changes)| {
            if file_changes.len() > 1 {
                FileChangeResult {
                    file,
                    status: ChangeStatus::PotentialWriteConflict,
                    changes: file_changes,
                }
            } else {
                FileChangeResult {
                    file,
                    status: ChangeStatus::Clean,
                    changes: file_changes,
                }
            }
        }).collect()
    }
}
```

### 5.5 Jaccard 相似度实现

```rust
fn jaccard_similarity(a: &str, b: &str) -> f64 {
    // Tokenize: 按 whitespace + punctuation 分词，转小写
    let tokenize = |s: &str| -> HashSet<String> {
        s.to_lowercase()
            .split(|c: char| c.is_whitespace() || c.is_ascii_punctuation())
            .filter(|t| t.len() > 1)
            .map(|t| t.to_string())
            .collect()
    };
    let set_a = tokenize(a);
    let set_b = tokenize(b);
    
    let intersection = set_a.intersection(&set_b).count();
    let union = set_a.union(&set_b).count();
    
    if union == 0 { return 0.0; }
    intersection as f64 / union as f64
}
```

archived-with: 2026-06-15-rlm-observability-and-robustness
status: final
---

## 6. RLM 预算控制与进展跟踪

### 6.1 Token Budget 实现

**task 工具 schema 扩展**（`src/tools/meta/task.rs`）：

```rust
// input_schema 中新增字段
"token_budget": {
    "type": "integer",
    "description": "Optional token budget in thousands of tokens (e.g., 10 = 10k tokens). 0 = unlimited.",
    "default": 0
}
```

**Subagent loop 中的预算检查**（`src/teams/subagent_loop.rs`）：

```rust
// 在每轮 API 调用完成后检查
let usage = response.usage.unwrap_or_default();
self.cumulative_tokens += usage.total_tokens as u64;

// 检查预算
if let Some(budget) = self.token_budget_k {
    if self.cumulative_tokens > budget * 1000 {
        return Err(SubagentError::BudgetExceeded {
            limit_k: budget,
            used: self.cumulative_tokens,
            rounds: self.round,
            last_tool: self.last_tool_name.clone(),
        });
    }
}

// 更新 progress
self.progress.cumulative_tokens = self.cumulative_tokens;
self.progress.token_budget_k = self.token_budget_k;
```

### 6.2 RLM Pipeline 预算分配

```rust
pub struct BudgetAllocation {
    pub total: u64,         // 总预算（千 tokens）
    pub planner: u64,       // 10%
    pub executor_pool: u64, // 80%
    pub aggregator: u64,    // 10%
}

impl BudgetAllocation {
    pub fn new(total_k: u64) -> Self {
        Self {
            total: total_k,
            planner: total_k / 10,
            executor_pool: total_k * 8 / 10,
            aggregator: total_k / 10,
        }
    }
    
    pub fn distribute_to_tasks(&self, task_count: usize) -> Vec<u64> {
        if task_count == 0 { return vec![]; }
        let per_task = self.executor_pool / task_count as u64;
        vec![per_task; task_count]
    }
    
    pub fn rollover_unused(&mut self, phase: Phase, unused: u64) {
        match phase {
            Phase::Planner => self.executor_pool += unused,
            Phase::Executor => self.aggregator += unused,
            _ => {}
        }
    }
}
```

### 6.3 Progress Delta 计算

```rust
pub struct ProgressTracker {
    tool_types_used: HashSet<String>,
    last_round_new_types: usize,
    stale_rounds: u32,
}

impl ProgressTracker {
    pub fn record_round(&mut self, events: &[SubagentEvent]) -> ProgressDelta {
        // 提取本轮使用的工具调用类型
        let this_round_types: HashSet<String> = events.iter()
            .filter_map(|e| match e {
                SubagentEvent::Action { tool_name, .. } => Some(tool_name.clone()),
                _ => None,
            })
            .collect();
        
        // 计算信息增益
        let new_types: HashSet<_> = this_round_types
            .difference(&self.tool_types_used)
            .collect();
        
        let delta = if self.tool_types_used.is_empty() {
            1.0  // 第一轮始终返回 1.0
        } else {
            new_types.len() as f32 / self.tool_types_used.len() as f32
        };
        
        // 更新状态
        self.tool_types_used.extend(this_round_types);
        
        // 检测停滞
        if delta < 0.05 {
            self.stale_rounds += 1;
        } else {
            self.stale_rounds = 0;
        }
        
        ProgressDelta {
            value: delta,
            stale_rounds: self.stale_rounds,
            is_stuck: self.stale_rounds >= 3,
        }
    }
}
```

### 6.4 Stuck Detection 集成

在 `StuckDetector` 基础上增加 `ProgressTracker`：

```rust
// subagent_loop.rs
let mut stuck_detector = StuckDetector::new(3);     // 现有：3 次重复 tool → abort
let mut progress_tracker = ProgressTracker::new();   // 新增：信息增益检查

loop {
    // ... 执行 API 调用和工具调用 ...
    
    let progress = progress_tracker.record_round(&round_events);
    if progress.is_stuck {
        return Err(SubagentError::NoProgress {
            rounds: round,
            delta: progress.value,
            stale_rounds: progress.stale_rounds,
        });
    }
    
    let stuck = stuck_detector.record_round(&tool_calls);
    if stuck.is_stuck() {
        return Err(SubagentError::Stuck(stuck));
    }
}
```

archived-with: 2026-06-15-rlm-observability-and-robustness
status: final
---

## 7. TUI 集成与渲染

### 7.1 渲染流程

```
render.rs
  ├─ completion_panel.rs   ← 新增：补全面板渲染（在输入框上方）
  ├─ subagent_panel.rs     ← 修改：错误内联展开 + token 显示
  │   └─ detail_view.rs    ← 新增：全屏时间线详情
  ├─ subagent_tree.rs      ← 修改：存储新字段
  ├─ status.rs             ← 修改：显示 subagent 失败计数 + token 预算
  └─ input_box             ← 无变化（补全由 completion_panel 渲染）
```

### 7.2 渲染优先级（Z-order）

```
1. Detail View（全屏，最高层）
2. Completion Panel（输入框上方）
3. Subagent Panel（侧边栏）
4. Status Bar（底栏）
5. Chat Area（主区域）
```

### 7.3 状态栏增强

`status.rs` 的 meta 行扩展：

```
Before: (3m5s · ↑ 17 tokens · ↓ 449 tokens · NORMAL)
After:  ⠧ 3 active · 3/8 done · 1 failed · 12.5k/50k (3m5s · ↑ 17 · ↓ 449 · NORMAL)
                                                      ↑ token 预算使用
                                        ↑ 失败计数（红色）
                             ↑ 完成/总数
                  ↑ 活跃数
```

### 7.4 Detail View 快捷键

| 按键 | 行为 |
|------|------|
| `Enter`（在 subagent panel 中选中节点）| 打开 detail view |
| `↑` / `↓` | 滚动事件时间线（一次一行）|
| `PgUp` / `PgDn` | 滚动一页 |
| `g` | 跳到顶部 |
| `G` | 跳到底部 |
| `f` | 跳转到第一个 Error 事件 |
| `Esc` | 关闭 detail view，返回 subagent panel |

archived-with: 2026-06-15-rlm-observability-and-robustness
status: final
---

## 8. 配置扩展

### 8.1 新增 Settings 字段

在 `src/config/settings.rs` 的 `Settings` struct 中新增：

```rust
pub struct Settings {
    // ... 现有字段 ...

    // Transcript 持久化
    #[serde(default = "default_max_transcript_age_days")]
    pub max_transcript_age_days: u32,  // 默认 30，0 = 不限

    // Token 预算
    #[serde(default)]
    pub default_subagent_token_budget_k: usize,  // 默认 0 = 不限

    // RLM
    #[serde(default = "default_jaccard_threshold")]
    pub rlm_jaccard_threshold: f64,  // 默认 0.8
}

fn default_max_transcript_age_days() -> u32 { 30 }
fn default_jaccard_threshold() -> f64 { 0.8 }
```

### 8.2 配置热加载

`ConfigChanged` 事件需传播到：
- `CompletionEngine`：重新扫描 skills 目录（如果 skills 路径变更）
- `SubagentTranscriptStore`：更新 `retention_days`
- `TaskTool`：更新 `default_budget`

### 8.3 Ink CLI 侧变更

`packages/cli/src/hooks/use-agent.ts` 中 `AgentStatus` 类型扩展：

```typescript
interface AgentStatus {
  // ... existing fields ...
  completionState?: {
    visible: boolean;
    prefix: '@' | '/';
    partial: string;
    matches: CompletionMatch[];
    selectedIndex: number;
  };
  detailView?: {
    transcriptId: string;
    events: SubagentEvent[];
    scrollOffset: number;
  };
}
```

archived-with: 2026-06-15-rlm-observability-and-robustness
status: final
---

## 9. 文件变更清单

| 文件 | 变更类型 | 内容 |
|------|----------|------|
| `src/tui/components/completion_panel.rs` | **新建** | 补全面板渲染组件 |
| `src/tui/completion.rs` | **新建** | CompletionEngine 数据层 |
| `src/tui/app/event.rs` | 修改 | 新增 CompletionTrigger/Select/Dismiss 事件 |
| `src/tui/app/types.rs` | 修改 | CompletionState + DetailViewState |
| `src/tui/input_reader.rs` | 修改 | 检测 @/前缀，发送 CompletionTrigger |
| `src/tui/components/subagent_panel.rs` | 修改 | 错误内联展开 + token 显示 |
| `src/tui/components/detail_view.rs` | **新建** | 全屏时间线 detail view |
| `src/tui/components/subagent_tree.rs` | 修改 | 新字段存储（progress_delta, budget, etc.）|
| `src/tui/components/status.rs` | 修改 | 失败计数 + token 预算显示 |
| `src/tui/app/render.rs` | 修改 | 集成补全面板 + detail view 全屏模式 |
| `src/transcript/mod.rs` | **新建** | Transcript 模块公开接口 |
| `src/transcript/store.rs` | **新建** | SQLite 实现 |
| `src/agent/progress.rs` | 修改 | SubagentEvent 枚举扩展 + ErrorInfo + progress_delta |
| `src/teams/subagent_loop.rs` | 修改 | 完整事件记录 + 预算检查 + progress_tracker + checkpoint |
| `src/tools/meta/task.rs` | 修改 | token_budget 参数 + task_type 分类 + 重试逻辑 |
| `src/tools/meta/rlm/formats.rs` | **新建** | ClaimsOutput/DiffOutput struct + parse |
| `src/tools/meta/rlm/pipeline.rs` | 修改 | 预算分配 + 格式注入 + Aggregator 合并 |
| `src/tools/meta/rlm/mod.rs` | 修改 | 导出 formats 模块 |
| `src/plugins/commands.rs` | 修改 | 暴露命令列表供 CompletionEngine 读取 |
| `src/config/settings.rs` | 修改 | 新增 max_transcript_age_days, default_subagent_token_budget_k, rlm_jaccard_threshold |
| `packages/cli/src/components/input-box.tsx` | 修改 | Ink CLI 补全触发 |
| `packages/cli/src/hooks/use-agent.ts` | 修改 | AgentStatus 类型扩展 |

## 10. 测试策略

### 单元测试

| 测试目标 | 文件 | 测试点 |
|---------|------|--------|
| CompletionEngine.filter() | `src/tui/completion.rs` | @ 触发返回 skills、/ 触发返回 commands、空输入、大小写不敏感 |
| ClaimsOutput::parse() | `src/tools/meta/rlm/formats.rs` | 有效 JSON、代码块包裹、无 JSON、格式错误各场景 |
| DiffOutput::parse() | `src/tools/meta/rlm/formats.rs` | 同上 |
| jaccard_similarity() | `src/tools/meta/rlm/pipeline.rs` | 完全相同、完全不同、部分重叠、空字符串 |
| Aggregator::deduplicate_claims() | `src/tools/meta/rlm/pipeline.rs` | 重复 claims、唯一 claims、边界阈值 |
| Aggregator::merge_file_changes() | `src/tools/meta/rlm/pipeline.rs` | 单文件单 diff、单文件多 diff、多文件 |
| ProgressTracker::record_round() | `src/teams/subagent_loop.rs` | 新类型→高 delta、重复类型→低 delta、连续 3 轮 stuck |
| SubagentTranscriptStore CRUD | `src/transcript/store.rs` | save、list_by_session、get_by_id、search、cleanup |
| Token budget enforcement | `src/teams/subagent_loop.rs` | budget=0 不限、budget=10 超限 kill、budget 未超继续 |
| BudgetAllocation | `src/tools/meta/rlm/pipeline.rs` | 100k 分配比例、rollover、0 task 边界 |

### 集成测试

| 测试场景 | 验证点 |
|---------|--------|
| @ 触发 → 选择 skill → 提交 | 完整补全链路 |
| Subagent 正常完成 → transcript 持久化 | SQLite 写入验证 |
| Subagent timeout → 错误展示 + retry | 错误内联展开 + 重试链 |
| RLM pipeline 两个 subagent 产出冲突 | Aggregator 标记冲突 |
| Token budget 耗尽 → subagent kill | 错误信息完整 |
| Checkpoint 写入 → subagent 崩溃 → 恢复 | 中间状态完整性 |

### 手动验证

- TUI 输入框 `@`/`/` 补全交互
- Subagent panel 实时时间线滚动
- Failed 节点内联展开 + 重试
- Detail view 导航（↑↓/PgUp/PgDn/g/G/Esc）
- 选择性回滚（修改文件 + 重试）
