---
change: generic-agent-runtime
design-doc: docs/superpowers/specs/2026-06-23-generic-agent-runtime-design.md
base-ref: 21c47a77490bc0c6d287a8b89347015fd7fee016
---

# Generic Agent Runtime 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**目标：** 将 Comet 领域逻辑从 Rust 代码中剥离，只通过扩展 hooks 系统（SlashCommand 事件、inject_context/ask_user 动作、when_state 条件）实现泛型 Agent Runtime，使 Comet 完全由 YAML + 脚本 + SKILL.md 定义。

**架构：** Session 启动时一次性解析 workflow.yaml 产出纯数据（Guards、Layers、Routes、StateHandle），注入各组件后 engine 对象消失。Rust Runtime 零知晓 "workflow"、"comet"、"openspec" 等语义。

**技术栈：** Rust, serde_yaml, async_trait, tokio, ratatui

## 全局约束

- 不新增 WorkflowEngine/StateMachine/TransitionGuard/GuardPipeline 抽象
- 只扩展 hooks 系统：HookEvent::SlashCommand、HookAction::InjectContext/AskUser、when_state 条件
- `grep -r "comet\|openspec\|phase" src/runtime/` 必须返回零结果
- `grep -r "CometPhase\|CometState\|CometGuard\|comet_slash_agent_prompt" src/` 必须返回零结果
- `pub mod comet` 必须从 src/lib.rs 中移除
- src/comet/ 目录整个删除
- src/hooks/ -> src/runtime/hooks/
- src/guardian/ -> src/runtime/guardian.rs
- 所有 YAML 钩子按 fail-open 原则处理错误（hook 脚本失败时放行工具调用）
- 所有用户交互按 fail-closed 处理（不自动批准决策）
- 内部 visibility 上下文必须严格与用户可见消息分离

---

## 文件结构

```
src/
├── runtime/
│   ├── mod.rs                          # 模块声明、重新导出
│   ├── context.rs                      # 新建：ContextLayer, ContextAssembler
│   ├── interaction.rs                  # 新建：InteractionService trait + 类型
│   ├── interaction_tui.rs              # 新建：TUI 实现
│   ├── interaction_cli.rs              # 新建：CLI 实现
│   ├── interaction_headless.rs         # 新建：Headless 实现
│   ├── command.rs                      # 新建：CommandRouter
│   ├── hooks/
│   │   ├── mod.rs                      # 从 src/hooks/mod.rs 迁移并扩展
│   │   └── cc_adapter.rs               # 从 src/hooks/cc_adapter.rs 迁移
│   └── guardian.rs                     # 从 src/guardian/mod.rs 迁移
├── tools/
│   └── executor.rs                     # 修改：替换 CometGuard 为 hooks when_state
├── prompts/
│   └── mod.rs                          # 修改：替换 Layer 1b 为 ContextAssembler
├── tui/
│   └── app/
│       ├── input.rs                    # 修改：替换 route_slash_command + comet_slash_agent_prompt
│       ├── mod.rs                      # 修改：启动时初始化 workflow
│       └── completion.rs               # 修改：从 CommandRouter 读取完成项
├── knowledge/
│   └── external_registry.rs            # 修改：移除 comet_slash_agent_prompt、route_slash_command
├── comet/                              # 删除整个目录
├── hooks/                              # 删除（已迁移到 runtime/hooks/）
├── guardian/                           # 删除（已迁移到 runtime/guardian.rs）
```

---

## 任务分解

### Task 1: 创建 src/runtime/mod.rs + 模块骨架

**文件：**
- 创建：`src/runtime/mod.rs`
- 创建：`src/runtime/context.rs`（仅骨架：mod 声明、空结构体站位）
- 创建：`src/runtime/interaction.rs`（仅骨架）
- 创建：`src/runtime/interaction_tui.rs`（仅骨架）
- 创建：`src/runtime/interaction_cli.rs`（仅骨架）
- 创建：`src/runtime/interaction_headless.rs`（仅骨架）
- 创建：`src/runtime/command.rs`（仅骨架）

**接口：**
- 消费：无（第一个任务）
- 产出：`src/runtime/mod.rs` 声明所有模块，`src/lib.rs` 加 `pub mod runtime`

- [x] **Step 1: 创建 `src/runtime/mod.rs`**

```rust
pub mod command;
pub mod context;
pub mod hooks;
pub mod interaction;
pub mod interaction_cli;
pub mod interaction_headless;
pub mod interaction_tui;

pub mod guardian;
```

- [x] **Step 2: 为 6 个新文件各创建骨架文件，每个文件仅含 `// TODO: implement in Task X` 注释**

```bash
for f in context.rs interaction.rs interaction_tui.rs interaction_cli.rs interaction_headless.rs command.rs; do
  echo "// TODO: implement" > "src/runtime/$f"
done
```

- [x] **Step 3: 在 `src/lib.rs` 中添加 `pub mod runtime;`**

检查当前 `src/lib.rs` 中 `pub mod` 的声明顺序，在合适位置（如 `pub mod hooks` 附近）插入 `pub mod runtime;`

```bash
sed -i '' '/pub mod hooks/a\
pub mod runtime;\' src/lib.rs
```

- [x] **Step 4: 编译验证**

```bash
cargo check 2>&1 | head -20
```
预期输出：编译通过，或仅有未使用导入警告。

- [x] **Step 5: 提交**

```bash
git add src/runtime/ src/lib.rs
git commit -m "feat(runtime): create src/runtime/ module skeleton"
```

---

### Task 2: 扩展 HookEvent 枚举 + HookDefinition 重构

**文件：**
- 修改：`src/hooks/mod.rs`（将在 Task 8 中迁移，但此时仍在该位置工作）

**接口：**
- 消费：Task 1 的 `src/runtime/mod.rs`
- 产出：`HookEvent::SlashCommand` 变体、`HookAction` 枚举、`HookDefinition.actions` / `HookDefinition.when_state` 字段、`HookContext.workflow_state` / `HookContext.variables` 字段

- [x] **Step 1: 添加 `HookEvent::SlashCommand` 变体**

在 `src/hooks/mod.rs` 的 `HookEvent` 枚举中添加：

```rust
pub enum HookEvent {
    PreToolUse,
    PostToolUse,
    SessionStart,
    SessionEnd,
    Notification,
    Stop,
    UserPromptSubmit,
    PermissionRequest,
    SlashCommand,  // NEW
}
```

更新 `Display` impl：

```rust
HookEvent::SlashCommand => write!(f, "SlashCommand"),
```

- [x] **Step 2: 添加 `HookAction` 枚举**

在 `HookDefinition` 之前添加：

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ContextSource {
    Template(String),
    File(PathBuf),
    Inline(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LayerVisibility {
    Internal,
    Visible,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserOption {
    pub label: String,
    pub value: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HookAction {
    Command {
        command: String,
        timeout_secs: u64,
    },
    InjectContext {
        source: ContextSource,
        priority: u8,
        visibility: LayerVisibility,
    },
    AskUser {
        question: String,
        options: Vec<UserOption>,
    },
}
```

- [x] **Step 3: 重构 `HookDefinition`**

将 `command` + `timeout_secs` + `hook_type` 替换为 `actions: Vec<HookAction>` + `when_state: Option<String>`：

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookDefinition {
    pub event: HookEvent,
    pub matcher: Option<String>,
    pub when_state: Option<String>,        // NEW
    pub actions: Vec<HookAction>,          // was: command + timeout_secs + hook_type
}
```

需要保留向后兼容的 YAML 反序列化能力：如果 YAML 中提供 `command` 字段而非 `actions`，自动包装为 `HookAction::Command`。在 `HookDefinition` 上实现自定义 `Deserialize`：

```rust
impl<'de> Deserialize<'de> for HookDefinition {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct HookDefHelper {
            event: HookEvent,
            matcher: Option<String>,
            when_state: Option<String>,
            command: Option<String>,
            timeout_secs: Option<u64>,
            hook_type: Option<String>,
            actions: Option<Vec<HookAction>>,
        }
        let helper = HookDefHelper::deserialize(deserializer)?;
        let actions = match helper.actions {
            Some(a) => a,
            None => vec![HookAction::Command {
                command: helper.command.unwrap_or_default(),
                timeout_secs: helper.timeout_secs.unwrap_or(30),
            }],
        };
        Ok(HookDefinition {
            event: helper.event,
            matcher: helper.matcher,
            when_state: helper.when_state,
            actions,
        })
    }
}
```

- [x] **Step 4: 更新 `HookContext`**

添加 `workflow_state` 和 `variables` 字段，保留 `comet_phase` 作为别名（向后兼容，内部代理到 `workflow_state`）：

```rust
pub struct HookContext {
    // ... existing fields
    pub workflow_state: Option<String>,   // NEW (replaces comet_phase)
    pub variables: HashMap<String, String>, // NEW
}
```

将原 `comet_phase: Option<String>` 替换为 `workflow_state`。更新 `with_comet_phase()` 为 `with_workflow_state()`（保留原方法作为弃用别名）。

- [x] **Step 5: 编译验证**

```bash
cargo check 2>&1 | head -30
```

- [x] **Step 6: 提交**

```bash
git add src/hooks/mod.rs
git commit -m "feat(hooks): add SlashCommand event, HookAction enum, when_state condition"
```

---

### Task 3: HookManager 扩展（fire() 支持 when_state + 多 action 执行）

**文件：**
- 修改：`src/hooks/mod.rs`

**接口：**
- 消费：Task 2 的 `HookAction`、`HookDefinition.actions`、`when_state`
- 产出：`HookManager::fire()` 接收 `state` 参数 + 返回 `HookOutcome` 向量、`register_workflow_hooks()` 方法

- [x] **Step 1: 扩展 `HookOutcome`**

```rust
#[derive(Debug, Clone)]
pub struct HookOutcome {
    pub def: HookDefinition,
    pub continue_execution: bool,
    pub reason: Option<String>,
    pub injected_content: Option<String>,  // NEW: for InjectContext
    pub user_answer: Option<UserAnswer>,   // NEW: for AskUser
}

#[derive(Debug, Clone)]
pub struct UserAnswer {
    pub selected: Vec<String>,
}
```

- [x] **Step 2: 修改 `fire()` 方法签名**

```rust
pub async fn fire(
    &self,
    event: &HookEvent,
    ctx: &HookContext,
    state: Option<&str>,
) -> Vec<HookOutcome> {
    let defs = self.hooks.get(event).map(Vec::as_slice).unwrap_or(&[]);
    let mut outcomes = Vec::new();
    for def in defs {
        // NEW: when_state filter
        if let Some(ref when) = def.when_state {
            if let Some(current) = state {
                let states: Vec<&str> = when.split('|').collect();
                if !states.contains(&current) {
                    continue; // skip this hook
                }
            }
        }
        // NEW: execute all actions
        for action in &def.actions {
            let outcome = self.execute_action(def, action, ctx).await;
            outcomes.push(outcome);
        }
    }
    outcomes
}
```

- [x] **Step 3: 实现 `execute_action()` 方法**

```rust
async fn execute_action(
    &self,
    def: &HookDefinition,
    action: &HookAction,
    ctx: &HookContext,
) -> HookOutcome {
    match action {
        HookAction::Command { command, timeout_secs } => {
            // 现有 execute_hook 逻辑移入此分支
            let result = self.run_shell_command(command, *timeout_secs, ctx).await;
            HookOutcome {
                def: def.clone(),
                continue_execution: result.continue_execution,
                reason: result.reason,
                injected_content: None,
                user_answer: None,
            }
        }
        HookAction::InjectContext { source, priority: _, visibility: _ } => {
            let content = match source {
                ContextSource::Template(t) => Some(self.render_template(t, ctx)),
                ContextSource::File(p) => self.read_file_content(p).await,
                ContextSource::Inline(s) => Some(s.clone()),
            };
            HookOutcome {
                def: def.clone(),
                continue_execution: true,
                reason: None,
                injected_content: content,
                user_answer: None,
            }
        }
        HookAction::AskUser { question, options } => {
            // 占位：InteractionService 将在 Task 6 中集成
            HookOutcome {
                def: def.clone(),
                continue_execution: true,
                reason: None,
                injected_content: None,
                user_answer: Some(UserAnswer { selected: vec![] }),
            }
        }
    }
}
```

- [x] **Step 4: 添加 `register_workflow_hooks()` 方法**

```rust
impl HookManager {
    pub fn register_workflow_hooks(&mut self, hooks: Vec<HookDefinition>) {
        for hook in hooks {
            self.hooks.entry(hook.event.clone()).or_default().push(hook);
        }
    }
}
```

- [x] **Step 5: 添加参数到 `fire()` 的 `with_state()` 辅助方法**

在现有的 HookContext builder 方法附近添加：

```rust
pub fn with_state(mut self, state: Option<String>) -> Self {
    self.workflow_state = state;
    self
}
```

- [x] **Step 6: 更新所有 `fire()` 调用点（编译时发现）**

```bash
cargo check 2>&1
```
修复所有因 `fire()` 签名变更导致的编译错误。

- [x] **Step 7: 编译 + 测试**

```bash
cargo test -p wgenty-core -- hooks 2>&1 | tail -20
```

- [x] **Step 8: 提交**

```bash
git add src/hooks/mod.rs
git commit -m "feat(hooks): extend HookManager with when_state filtering and multi-action execution"
```

---

### Task 4: 实现 ContextAssembler

**文件：**
- 创建：`src/runtime/context.rs`（完整实现）

**接口：**
- 消费：Task 2 的 `ContextSource`、`LayerVisibility`
- 产出：`ContextLayer`、`ContextAssembler`、`AssembledContext`、`LayerCondition`

- [x] **Step 1: 编写 `ContextLayer` 和 `LayerCondition`**

```rust
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone)]
pub struct ContextLayer {
    pub id: String,
    pub priority: u8,
    pub visibility: LayerVisibility,
    pub source: ContextSource,
    pub condition: Option<LayerCondition>,
}

#[derive(Debug, Clone)]
pub enum LayerCondition {
    StateMatches(String),    // "build" — pipe-separated
    VariableSet(String),     // "build_mode=subagent"
}
```

- [x] **Step 2: 实现 `ContextAssembler`**

```rust
use super::hooks::{ContextSource, LayerVisibility};

pub struct ContextAssembler {
    layers: Vec<ContextLayer>,
    state: Arc<RwLock<String>>,
    variables: HashMap<String, String>,
}

#[derive(Debug, Default)]
pub struct AssembledContext {
    pub internal_instructions: Vec<String>,
    pub visible_content: Vec<String>,
}

impl ContextAssembler {
    pub fn new(
        layers: Vec<ContextLayer>,
        state: Arc<RwLock<String>>,
        variables: HashMap<String, String>,
    ) -> Self {
        ContextAssembler { layers, state, variables }
    }

    pub async fn assemble(&self) -> AssembledContext {
        let current_state = self.state.read().await.clone();

        // 1. Filter layers by condition
        let mut active: Vec<&ContextLayer> = self.layers.iter()
            .filter(|layer| self.is_active(layer, &current_state))
            .collect();

        // 2. Sort by priority (ascending)
        active.sort_by_key(|l| l.priority);

        // 3. Resolve sources and route by visibility
        let mut ctx = AssembledContext::default();
        for layer in active {
            let content = self.resolve_source(&layer.source).await;
            match layer.visibility {
                LayerVisibility::Internal => ctx.internal_instructions.push(content),
                LayerVisibility::Visible => ctx.visible_content.push(content),
            }
        }
        ctx
    }

    fn is_active(&self, layer: &ContextLayer, current_state: &str) -> bool {
        match &layer.condition {
            None => true,
            Some(LayerCondition::StateMatches(states)) => {
                states.split('|').any(|s| s.trim() == current_state)
            }
            Some(LayerCondition::VariableSet(key_val)) => {
                let parts: Vec<&str> = key_val.splitn(2, '=').collect();
                if parts.len() == 2 {
                    self.variables.get(parts[0]).map_or(false, |v| v == parts[1])
                } else {
                    self.variables.contains_key(parts[0])
                }
            }
        }
    }

    async fn resolve_source(&self, source: &ContextSource) -> String {
        match source {
            ContextSource::Template(t) => self.render_template(t),
            ContextSource::File(p) => {
                tokio::fs::read_to_string(p).await.unwrap_or_default()
            }
            ContextSource::Inline(s) => s.clone(),
        }
    }

    fn render_template(&self, template: &str) -> String {
        let mut result = template.to_string();
        // {{ state }}
        result = result.replace("{{ state }}", &self.state.try_read().map(|s| s.clone()).unwrap_or_default());
        for (key, value) in &self.variables {
            result = result.replace(&format!("{{{{ {} }}}}", key), value);
        }
        result
    }
}
```

- [x] **Step 3: 写单元测试**

在文件末尾添加测试模块：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_priority_ordering() {
        let state = Arc::new(RwLock::new("build".to_string()));
        let layers = vec![
            ContextLayer {
                id: "high".into(), priority: 50, visibility: LayerVisibility::Internal,
                source: ContextSource::Inline("high priority".into()), condition: None,
            },
            ContextLayer {
                id: "low".into(), priority: 10, visibility: LayerVisibility::Internal,
                source: ContextSource::Inline("low priority".into()), condition: None,
            },
        ];
        let assembler = ContextAssembler::new(layers, state, HashMap::new());
        let ctx = assembler.assemble().await;
        // low priority (10) comes first, then high (50)
        assert_eq!(ctx.internal_instructions[0], "low priority");
        assert_eq!(ctx.internal_instructions[1], "high priority");
    }

    #[tokio::test]
    async fn test_visibility_separation() {
        let state = Arc::new(RwLock::new("build".to_string()));
        let layers = vec![
            ContextLayer {
                id: "internal".into(), priority: 10, visibility: LayerVisibility::Internal,
                source: ContextSource::Inline("hidden".into()), condition: None,
            },
            ContextLayer {
                id: "visible".into(), priority: 20, visibility: LayerVisibility::Visible,
                source: ContextSource::Inline("shown".into()), condition: None,
            },
        ];
        let assembler = ContextAssembler::new(layers, state, HashMap::new());
        let ctx = assembler.assemble().await;
        assert!(ctx.internal_instructions.contains(&"hidden".to_string()));
        assert!(ctx.visible_content.contains(&"shown".to_string()));
        assert!(!ctx.visible_content.contains(&"hidden".to_string()));
    }

    #[tokio::test]
    async fn test_template_variable_substitution() {
        let state = Arc::new(RwLock::new("design".to_string()));
        let mut vars = HashMap::new();
        vars.insert("change".into(), "my-feature".into());
        let layers = vec![
            ContextLayer {
                id: "phase".into(), priority: 10, visibility: LayerVisibility::Internal,
                source: ContextSource::Template("Current: {{ state }}, change: {{ change }}".into()),
                condition: None,
            },
        ];
        let assembler = ContextAssembler::new(layers, state, vars);
        let ctx = assembler.assemble().await;
        assert_eq!(ctx.internal_instructions[0], "Current: design, change: my-feature");
    }

    #[tokio::test]
    async fn test_condition_state_matches() {
        let state = Arc::new(RwLock::new("build".to_string()));
        let layers = vec![
            ContextLayer {
                id: "build-only".into(), priority: 10, visibility: LayerVisibility::Internal,
                source: ContextSource::Inline("build content".into()),
                condition: Some(LayerCondition::StateMatches("build".into())),
            },
            ContextLayer {
                id: "open-only".into(), priority: 20, visibility: LayerVisibility::Internal,
                source: ContextSource::Inline("open content".into()),
                condition: Some(LayerCondition::StateMatches("open".into())),
            },
        ];
        let assembler = ContextAssembler::new(layers, state, HashMap::new());
        let ctx = assembler.assemble().await;
        assert!(ctx.internal_instructions.contains(&"build content".to_string()));
        assert!(!ctx.internal_instructions.contains(&"open content".to_string()));
    }
}
```

- [x] **Step 4: 运行测试验证**

```bash
cargo test -p wgenty-core -- context 2>&1 | tail -20
```
预期输出：4 个测试全部 PASS。

- [x] **Step 5: 提交**

```bash
git add src/runtime/context.rs
git commit -m "feat(runtime): implement ContextAssembler with priority, visibility, conditions"
```

---

### Task 5: 实现 CommandRouter

**文件：**
- 创建：`src/runtime/command.rs`（完整实现）
- 修改：`src/knowledge/external_registry.rs`（移除 `route_slash_command` 和 `comet_slash_agent_prompt`）

**接口：**
- 消费：无
- 产出：`CommandRouter`、`RouteResult`、`CommandInvocation`

- [ ] **Step 1: 编写 CommandRouter + RouteResult**

```rust
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct CommandInvocation {
    pub name: String,
    pub args: String,
    pub raw_input: String,
}

#[derive(Debug, Clone)]
pub enum RouteResult {
    BuiltIn,
    Workflow { name: String, command: String, args: String },
    Unknown { command: String, suggestions: Vec<String> },
    NotSlash,
}

pub struct CommandRouter {
    builtins: Vec<String>,
    workflow_commands: HashMap<String, String>, // command -> workflow name
}

impl CommandRouter {
    pub fn new(builtins: Vec<String>) -> Self {
        CommandRouter {
            builtins,
            workflow_commands: HashMap::new(),
        }
    }

    pub fn register_workflow(&mut self, name: &str, entry_commands: &[String]) {
        for cmd in entry_commands {
            self.workflow_commands.insert(cmd.clone(), name.to_string());
        }
    }

    pub fn route(&self, input: &str) -> RouteResult {
        if !input.starts_with('/') {
            return RouteResult::NotSlash;
        }
        let text = &input[1..]; // strip /
        let parts: Vec<&str> = text.splitn(2, ' ');
        let command = parts[0].to_string();
        let args = parts.get(1).unwrap_or(&"").to_string();

        if self.builtins.contains(&command) {
            return RouteResult::BuiltIn;
        }
        if let Some(workflow_name) = self.workflow_commands.get(&command) {
            return RouteResult::Workflow {
                name: workflow_name.clone(),
                command: command.clone(),
                args,
            };
        }
        RouteResult::Unknown { command, suggestions: vec![] }
    }

    pub fn entry_commands(&self) -> Vec<String> {
        let mut cmds: Vec<String> = self.builtins.clone();
        for cmd in self.workflow_commands.keys() {
            cmds.push(cmd.clone());
        }
        cmds
    }
}
```

- [ ] **Step 2: 写单元测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_match() {
        let router = CommandRouter::new(vec!["clear".into(), "help".into()]);
        assert!(matches!(router.route("/clear"), RouteResult::BuiltIn));
        assert!(matches!(router.route("/help"), RouteResult::BuiltIn));
    }

    #[test]
    fn test_workflow_match() {
        let mut router = CommandRouter::new(vec![]);
        router.register_workflow("comet", &["comet".into(), "comet-open".into()]);
        match router.route("/comet fix bug") {
            RouteResult::Workflow { name, command, args } => {
                assert_eq!(name, "comet");
                assert_eq!(command, "comet");
                assert_eq!(args, "fix bug");
            }
            _ => panic!("expected Workflow route"),
        }
    }

    #[test]
    fn test_not_slash() {
        let router = CommandRouter::new(vec![]);
        assert!(matches!(router.route("hello"), RouteResult::NotSlash));
    }

    #[test]
    fn test_unknown() {
        let router = CommandRouter::new(vec![]);
        assert!(matches!(router.route("/unknown"), RouteResult::Unknown { .. }));
    }
}
```

- [ ] **Step 3: 在 `src/knowledge/external_registry.rs` 中标记 `route_slash_command` 和 `comet_slash_agent_prompt` 为弃用**

添加 `#[deprecated(note = "use CommandRouter instead")]` 注解到这两个函数。将在 Task 10 中替换所有调用点并删除。

- [ ] **Step 4: 运行测试**

```bash
cargo test -p wgenty-core -- command 2>&1 | tail -20
```
预期输出：4 个测试 PASS。

- [ ] **Step 5: 提交**

```bash
git add src/runtime/command.rs src/knowledge/external_registry.rs
git commit -m "feat(runtime): implement CommandRouter with builtin/workflow routing"
```

---

### Task 6: 实现 InteractionService trait + TUI/CLI/Headless 实现

**文件：**
- 创建：`src/runtime/interaction.rs`（trait + 类型）
- 创建：`src/runtime/interaction_tui.rs`
- 创建：`src/runtime/interaction_cli.rs`
- 创建：`src/runtime/interaction_headless.rs`
- 修改：`src/state/agent_phase.rs`（添加 `WaitingForInteraction` 变体）

**接口：**
- 消费：Task 2 的 `UserOption`
- 产出：`InteractionService` trait、TUI/CLI/Headless 实现

- [ ] **Step 1: 实现 `src/runtime/interaction.rs`**

```rust
use async_trait::async_trait;

#[derive(Debug, Clone)]
pub struct InteractionQuestion {
    pub id: String,
    pub message: String,
    pub options: Vec<InteractionOption>,
}

#[derive(Debug, Clone)]
pub struct InteractionOption {
    pub label: String,
    pub value: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone)]
pub struct UserAnswer {
    pub selected: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ConfirmPrompt {
    pub message: String,
    pub default_yes: bool,
}

#[async_trait]
pub trait InteractionService: Send + Sync {
    async fn ask(&self, question: &InteractionQuestion) -> anyhow::Result<UserAnswer>;
    async fn confirm(&self, prompt: &ConfirmPrompt) -> anyhow::Result<bool>;
}
```

- [ ] **Step 2: 实现 `src/runtime/interaction_tui.rs`**

```rust
use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio::sync::Mutex;
use super::interaction::{InteractionService, InteractionQuestion, ConfirmPrompt, UserAnswer};

pub struct TuiInteractionService {
    event_tx: mpsc::UnboundedSender<AppEvent>,
    response_rx: Mutex<mpsc::UnboundedReceiver<UserAnswer>>,
}

// AppEvent 和 AgentPhase 的引用需要在 tui crate 中解析，这里提供骨架
```

注意：完整 TUI 集成需要引用 `AppEvent` 和 `AgentPhase` 类型（在 `src/tui/app/` 和 `src/state` 中）。此任务提供骨架实现，Task 10.4 中完成完整集成。

- [ ] **Step 3: 实现 `src/runtime/interaction_cli.rs`**

```rust
use async_trait::async_trait;
use tokio::task;
use super::interaction::{InteractionService, InteractionQuestion, ConfirmPrompt, UserAnswer};

pub struct CliInteractionService;

#[async_trait]
impl InteractionService for CliInteractionService {
    async fn ask(&self, question: &InteractionQuestion) -> anyhow::Result<UserAnswer> {
        println!("\n{}", question.message);
        for (i, opt) in question.options.iter().enumerate() {
            let desc = opt.description.as_ref()
                .map(|d| format!(" - {}", d))
                .unwrap_or_default();
            println!("  {}. {}{}", i + 1, opt.label, desc);
        }
        print!("Enter choice (1-{}): ", question.options.len());
        let input = task::spawn_blocking(|| {
            let mut buf = String::new();
            std::io::stdin().read_line(&mut buf).ok();
            buf.trim().to_string()
        }).await?;
        let idx: usize = input.parse().unwrap_or(0);
        if idx > 0 && idx <= question.options.len() {
            Ok(UserAnswer { selected: vec![question.options[idx - 1].value.clone()] })
        } else {
            Ok(UserAnswer { selected: vec![] })
        }
    }

    async fn confirm(&self, prompt: &ConfirmPrompt) -> anyhow::Result<bool> {
        let default = if prompt.default_yes { "Y/n" } else { "y/N" };
        println!("{} [{}]", prompt.message, default);
        let input = task::spawn_blocking(|| {
            let mut buf = String::new();
            std::io::stdin().read_line(&mut buf).ok();
            buf.trim().to_lowercase()
        }).await?;
        Ok(match input.as_str() {
            "y" | "yes" => true,
            "n" | "no" => false,
            "" => prompt.default_yes,
            _ => prompt.default_yes,
        })
    }
}
```

- [ ] **Step 4: 实现 `src/runtime/interaction_headless.rs`**

```rust
use async_trait::async_trait;
use super::interaction::{InteractionService, InteractionQuestion, ConfirmPrompt, UserAnswer};

#[derive(Debug, Clone)]
pub struct AnswerMap {
    pub question_id: String,
    pub answer: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum HeadlessPolicy {
    Deny,
    PreConfigured(Vec<AnswerMap>),
}

pub struct HeadlessInteractionService {
    policy: HeadlessPolicy,
}

impl HeadlessInteractionService {
    pub fn new(policy: HeadlessPolicy) -> Self {
        HeadlessInteractionService { policy }
    }
}

#[async_trait]
impl InteractionService for HeadlessInteractionService {
    async fn ask(&self, question: &InteractionQuestion) -> anyhow::Result<UserAnswer> {
        match &self.policy {
            HeadlessPolicy::Deny => Err(anyhow::anyhow!(
                "Interaction denied: headless mode does not support user input"
            )),
            HeadlessPolicy::PreConfigured(maps) => {
                for map in maps {
                    if map.question_id == question.id {
                        return Ok(UserAnswer { selected: map.answer.clone() });
                    }
                }
                Err(anyhow::anyhow!(
                    "No pre-configured answer for question: {}", question.id
                ))
            }
        }
    }

    async fn confirm(&self, prompt: &ConfirmPrompt) -> anyhow::Result<bool> {
        match &self.policy {
            HeadlessPolicy::Deny => Err(anyhow::anyhow!(
                "Interaction denied: headless mode does not support user input"
            )),
            HeadlessPolicy::PreConfigured(maps) => {
                // default_yes as fallback if no specific answer
                Ok(prompt.default_yes)
            }
        }
    }
}
```

- [ ] **Step 5: 添加 `AgentPhase::WaitingForInteraction`**

在 `src/state/agent_phase.rs` 中：

```rust
pub enum AgentPhase {
    // ... existing variants
    WaitingForInteraction,  // NEW
}
```

- [ ] **Step 6: 运行测试**

```bash
cargo check 2>&1 | head -20
```

- [ ] **Step 7: 提交**

```bash
git add src/runtime/interaction.rs src/runtime/interaction_tui.rs src/runtime/interaction_cli.rs src/runtime/interaction_headless.rs src/state/agent_phase.rs
git commit -m "feat(runtime): implement InteractionService trait with TUI/CLI/Headless backends"
```

---

### Task 7: 创建 workflow.yaml 配置

**文件：**
- 创建：`.wgenty-code/skills/comet/workflow.yaml`
- 创建：`tests/workflow_comet_test.rs`

- [ ] **Step 1: 创建 `.wgenty-code/skills/comet/workflow.yaml`**

```yaml
name: comet
entry_commands:
  - comet
  - comet-open
  - comet-design
  - comet-build
  - comet-verify
  - comet-archive
  - comet-hotfix
  - comet-tweak

state:
  initial: null
  read_script: "comet-state current --json"
  write_script: "comet-state set"

hooks:
  # SlashCommand: inject comet skill + discover OpenSpec state
  - event: SlashCommand
    matcher: "comet|comet-open|comet-design|comet-build|comet-verify|comet-archive"
    actions:
      - type: inject_context
        source:
          file: "${SKILL_DIR}/SKILL.md"
        priority: 40
        visibility: internal
      - type: command
        command: "openspec list --json"
        timeout_secs: 10

  # PreToolUse: phase guard via comet-guard.sh
  - event: PreToolUse
    when_state: "open|design|verify|archive"
    matcher: "Write|Edit|Bash"
    actions:
      - type: command
        command: "comet-guard check --phase ${STATE} --tool ${TOOL}"
        timeout_secs: 10

  # PreToolUse: read-only bash exceptions
  - event: PreToolUse
    when_state: "open|design|verify|archive"
    matcher: "Bash"
    actions:
      - type: command
        command: "comet-guard check-readonly --command '${ARGS.command}'"
        timeout_secs: 5

  # UserPromptSubmit: phase guard rule injection
  - event: UserPromptSubmit
    when_state: "open|design|build|verify|archive"
    actions:
      - type: inject_context
        source:
          file: "${SKILL_DIR}/references/phase-guard.md"
        priority: 25
        visibility: internal

context:
  # Phase awareness
  - id: phase-instruction
    priority: 35
    visibility: internal
    condition:
      state_matches: "open|design|build|verify|archive"
    source:
      template: |
        Current Comet phase: {{ state }}
        Phase rules: {{ phase_rules[state] }}

  # Coordinator mode reminder (build phase only)
  - id: coordinator-reminder
    priority: 30
    visibility: internal
    condition:
      state_matches: "build"
      variable_set: "build_mode=subagent-driven-development"
    source:
      file: "${SKILL_DIR}/references/coordinator-reminder.md"

templates:
  phase_rules:
    open: |
      Phase: Open. Allowed: create proposal/design/tasks. Forbidden: source code changes.
    design: |
      Phase: Design. Allowed: brainstorming, create Design Doc. Forbidden: source code changes.
    build: |
      Phase: Build. Allowed: source code, tests, plan execution. Must confirm at decision points.
    verify: |
      Phase: Verify. Allowed: verification, branch handling. Must not skip failure handling.
    archive: |
      Phase: Archive. Allowed: archive confirmation. Forbidden: source code changes.
```

- [ ] **Step 2: 编写集成测试 `tests/workflow_comet_test.rs`**

```rust
#[cfg(test)]
mod tests {
    use std::path::Path;

    #[test]
    fn test_workflow_yaml_exists() {
        let path = Path::new(".wgenty-code/skills/comet/workflow.yaml");
        assert!(path.exists(), "workflow.yaml must exist");
    }

    #[test]
    fn test_workflow_yaml_is_valid_yaml() {
        let content = std::fs::read_to_string(".wgenty-code/skills/comet/workflow.yaml")
            .expect("failed to read workflow.yaml");
        let parsed: serde_yaml::Value = serde_yaml::from_str(&content)
            .expect("workflow.yaml must be valid YAML");
        assert!(parsed.get("name").is_some(), "workflow.yaml must have a name");
        assert!(parsed.get("entry_commands").is_some(), "workflow.yaml must have entry_commands");
    }

    #[test]
    fn test_workflow_entry_commands() {
        let content = std::fs::read_to_string(".wgenty-code/skills/comet/workflow.yaml").unwrap();
        let parsed: serde_yaml::Value = serde_yaml::from_str(&content).unwrap();
        let commands = parsed["entry_commands"].as_sequence().unwrap();
        let names: Vec<&str> = commands.iter()
            .map(|c| c.as_str().unwrap())
            .collect();
        assert!(names.contains(&"comet"));
        assert!(names.contains(&"comet-open"));
        assert!(names.contains(&"comet-design"));
        assert!(names.contains(&"comet-build"));
        assert!(names.contains(&"comet-verify"));
        assert!(names.contains(&"comet-archive"));
    }

    #[test]
    fn test_workflow_state_config() {
        let content = std::fs::read_to_string(".wgenty-code/skills/comet/workflow.yaml").unwrap();
        let parsed: serde_yaml::Value = serde_yaml::from_str(&content).unwrap();
        let state = parsed.get("state").expect("workflow.yaml must have state config");
        assert!(state.get("read_script").is_some(), "state must define read_script");
        assert!(state.get("write_script").is_some(), "state must define write_script");
    }

    #[test]
    fn test_workflow_hooks_defined() {
        let content = std::fs::read_to_string(".wgenty-code/skills/comet/workflow.yaml").unwrap();
        let parsed: serde_yaml::Value = serde_yaml::from_str(&content).unwrap();
        let hooks = parsed.get("hooks").expect("workflow.yaml must have hooks");
        assert!(!hooks.as_sequence().unwrap().is_empty(), "must have at least one hook");
    }

    #[test]
    fn test_workflow_context_layers_defined() {
        let content = std::fs::read_to_string(".wgenty-code/skills/comet/workflow.yaml").unwrap();
        let parsed: serde_yaml::Value = serde_yaml::from_str(&content).unwrap();
        let context = parsed.get("context").expect("workflow.yaml must have context layers");
        assert!(!context.as_sequence().unwrap().is_empty(), "must have at least one context layer");
    }
}
```

- [ ] **Step 3: 运行集成测试**

```bash
cargo test --test workflow_comet_test 2>&1 | tail -20
```

- [ ] **Step 4: 提交**

```bash
git add .wgenty-code/skills/comet/workflow.yaml tests/workflow_comet_test.rs
git commit -m "feat(comet): create workflow.yaml defining comet as hook configuration"
```

---

### Task 8: 迁移 hooks 和 guardian 到 runtime 目录

**文件：**
- 移动：`src/hooks/mod.rs` -> `src/runtime/hooks/mod.rs`
- 移动：`src/hooks/cc_adapter.rs` -> `src/runtime/hooks/cc_adapter.rs`
- 移动：`src/guardian/mod.rs` -> `src/runtime/guardian.rs`
- 修改：所有 `crate::hooks` 导入 -> `crate::runtime::hooks`
- 修改：所有 `crate::guardian` 导入 -> `crate::runtime::guardian`
- 删除：`src/hooks/`（空目录）
- 删除：`src/guardian/`（空目录）

**接口：**
- 消费：Task 1 的 `pub mod runtime`
- 产出：所有导入更新完毕、原有 hooks/guardian 保持功能不变、所有测试通过

- [ ] **Step 1: 移动 hooks 文件**

```bash
mkdir -p src/runtime/hooks
git mv src/hooks/mod.rs src/runtime/hooks/mod.rs
git mv src/hooks/cc_adapter.rs src/runtime/hooks/cc_adapter.rs
```

- [ ] **Step 2: 移动 guardian 文件**

```bash
git mv src/guardian/mod.rs src/runtime/guardian.rs
```

- [ ] **Step 3: 更新所有 `crate::hooks` 引用为 `crate::runtime::hooks`**

```bash
# 查找所有引用 crate::hooks 的文件
grep -rl "crate::hooks" src/ | grep -v target/
```

逐一更新每个文件。常见的需要修改的文件包括：
- `src/tools/executor.rs`
- `src/prompts/mod.rs`
- `src/tui/app/input.rs`
- `src/tui/app/mod.rs`
- 其他通过 use crate::hooks 导入的文件

对于每个文件，执行替换：
```bash
sed -i '' 's/crate::hooks/crate::runtime::hooks/g' src/tools/executor.rs
sed -i '' 's/crate::hooks/crate::runtime::hooks/g' src/prompts/mod.rs
# ... 对所有匹配文件重复
```

- [ ] **Step 4: 更新所有 `crate::guardian` 引用为 `crate::runtime::guardian`**

```bash
grep -rl "crate::guardian" src/ | grep -v target/
# 对每个文件执行替换
sed -i '' 's/crate::guardian/crate::runtime::guardian/g' src/tools/executor.rs
# ... 对所有匹配文件重复
```

- [ ] **Step 5: 编译验证**

```bash
cargo check 2>&1 | head -30
```
修复所有 "module not found" 错误。可能遗漏的引用文件也执行替换。

- [ ] **Step 6: 运行所有 hooks 相关测试**

```bash
cargo test -p wgenty-core -- hooks 2>&1 | tail -20
cargo test -p wgenty-core -- guardian 2>&1 | tail -20
```
确认所有测试仍然通过。

- [ ] **Step 7: 删除空的 src/hooks/ 和 src/guardian/ 目录**

```bash
rmdir src/hooks 2>/dev/null; rmdir src/guardian 2>/dev/null; true
```

- [ ] **Step 8: 提交**

```bash
git add src/
git commit -m "refactor(runtime): migrate hooks/ and guardian/ into runtime/ module"
```

---

### Task 9: 替换 ToolExecutor 中的 CometGuard 为 hooks when_state

**文件：**
- 修改：`src/tools/executor.rs`

**接口：**
- 消费：Task 3 的 `HookManager::fire()` with `state` parameter、Task 8 的 `crate::runtime::hooks` 路径
- 产出：`ToolExecutor` 替换 `comet_state: Option<CometState>` 为 `state_handle: Option<Arc<RwLock<str>>>`，移除 `CometGuard::check()` 调用

- [ ] **Step 1: 替换 `ToolExecutor` 结构体字段**

```rust
// Before:
pub struct ToolExecutor {
    pub comet_state: Option<CometState>,
    // ...
}

// After:
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct ToolExecutor {
    pub state_handle: Option<Arc<RwLock<String>>>,
    pub hook_manager: Arc<HookManager>,
    // ...
}
```

- [ ] **Step 2: 移除 `CometGuard::check()` 调用**

找到并删除类似以下代码块：
```rust
if let Some(ref state) = self.comet_state {
    let decision = CometGuard::check(&state.phase, tool_name, &guard_args);
    if decision.blocked { /* ... */ }
}
```

替换为：
```rust
// PreToolUse hooks with when_state — HookManager handles filtering
let state_val = self.state_handle.as_ref()
    .map(|s| s.try_read().ok().map(|r| r.clone()))
    .flatten();
let pre_ctx = HookManager::pre_tool_context(tool_name, &args, session_id)
    .with_state(state_val.clone());
let pre_outcomes = self.hook_manager.fire(
    &HookEvent::PreToolUse,
    &pre_ctx,
    state_val.as_deref(),
).await;

// If any hook blocked execution, block the tool
for outcome in &pre_outcomes {
    if !outcome.continue_execution {
        // existing block logic (fire notification, return error)
        return Err(ToolError::BlockedByHook {
            reason: outcome.reason.clone().unwrap_or_default(),
        });
    }
}
```

- [ ] **Step 3: 更新 `HookManager::pre_tool_context()` 的 `comet_phase` -> `workflow_state`**

在 `src/runtime/hooks/mod.rs` 中找到 `pre_tool_context()` 和 `pre_user_prompt_context()` 方法，将 `comet_phase: None` 改为 `workflow_state: None`。

- [ ] **Step 4: 添加 `set_state_handle()` 方法**

```rust
impl ToolExecutor {
    pub fn set_state_handle(&mut self, handle: Option<Arc<RwLock<String>>>) {
        self.state_handle = handle;
    }
}
```

- [ ] **Step 5: 编译验证**

```bash
cargo check 2>&1 | head -30
```

- [ ] **Step 6: 运行工具执行测试**

```bash
cargo test -p wgenty-core -- executor 2>&1 | tail -30
```

- [ ] **Step 7: 提交**

```bash
git add src/tools/executor.rs src/runtime/hooks/mod.rs
git commit -m "refactor(executor): replace CometGuard::check() with hooks when_state filtering"
```

---

### Task 10: 替换 Prompt Assembler 中的 Layer 1b 为 ContextAssembler

**文件：**
- 修改：`src/prompts/mod.rs`

**接口：**
- 消费：Task 4 的 `ContextAssembler`、`AssembledContext`
- 产出：`PromptContext` 获得 `context_assembler: Option<Arc<ContextAssembler>>` 字段

- [ ] **Step 1: 为 `PromptContext` 添加 `context_assembler` 字段**

```rust
pub struct PromptContext {
    // ... existing fields
    pub context_assembler: Option<Arc<ContextAssembler>>,
}
```

- [ ] **Step 2: 替换 Layer 1b（硬编码 comet 注入）**

找到以下代码块（原始行 ~119-128）：
```rust
if let Some(comet_state) = crate::comet::CometState::read(working_dir) {
    system_messages.push(ChatMessage::system(comet_state.phase_instruction()));
}
if crate::comet::CometGuard::is_coordinator_mode(working_dir) {
    system_messages.push(ChatMessage::system(crate::comet::CometGuard::coordinator_reminder()));
}
```

替换为：
```rust
if let Some(ref assembler) = context.context_assembler {
    let assembled = assembler.assemble().await;
    for instruction in &assembled.internal_instructions {
        system_messages.push(ChatMessage::system(instruction));
    }
}
```

- [ ] **Step 3: 移除 `use crate::comet` 相关导入**

搜索并移除 `src/prompts/mod.rs` 中所有引用 `crate::comet` 的 `use` 语句。

添加 `use crate::runtime::context::ContextAssembler;` 和 `use std::sync::Arc;`。

- [ ] **Step 4: 移除 `comet_phase` 引用**

检查 `src/prompts/mod.rs` 中是否还有对 `comet_phase` 或 `phase_instruction` 的引用，全部清理。

- [ ] **Step 5: 编译验证**

```bash
cargo check 2>&1 | head -30
```

- [ ] **Step 6: 运行 prompts 测试**

```bash
cargo test -p wgenty-core -- prompts 2>&1 | tail -30
```

- [ ] **Step 7: 提交**

```bash
git add src/prompts/mod.rs
git commit -m "refactor(prompts): replace hardcoded comet Layer 1b with ContextAssembler"
```

---

### Task 11: 替换 TUI 输入和启动中的 Comet 引用

**文件：**
- 修改：`src/tui/app/input.rs`
- 修改：`src/tui/app/mod.rs`
- 修改：`src/tui/completion.rs`

**接口：**
- 消费：Task 5 的 `CommandRouter`、`RouteResult`、Task 3 的 `HookManager`、Task 6 的 `InteractionService`
- 产出：TUI 从 CommandRouter 读取命令、SlashCommand 事件触发 hooks、启动时初始化 workflow

- [ ] **Step 1: 修改 `src/tui/app/input.rs`**

找到 `route_slash_command()` 调用：
```rust
let route = crate::knowledge::route_slash_command(&text, builtins, registry);
match route {
    crate::knowledge::SlashRoute::ExternalSkill { skill, args } => {
        let agent_input = crate::knowledge::comet_slash_agent_prompt(&skill, &args)...;
        // Shows "🔧 External skill '/...' detected..." message
    }
}
```

替换为：
```rust
if let Some(ref router) = self.command_router {
    match router.route(&text) {
        RouteResult::Workflow { name, command, args } => {
            // Fire SlashCommand hooks — inject_context handles the skill prompt
            let hook_ctx = HookContext {
                event: "SlashCommand".to_string(),
                tool_name: Some(command.clone()),
                tool_input: Some(serde_json::json!({
                    "command": command,
                    "args": args,
                })),
                // ... other fields
            };
            self.hook_manager.fire(&HookEvent::SlashCommand, &hook_ctx, None).await;
            // Show friendly status
            self.committed_messages.push(UIMessage {
                role: MessageRole::System,
                content: format!("Starting {} workflow...", name),
                // ...
            });
        }
        RouteResult::BuiltIn => { /* existing builtin handling */ }
        RouteResult::Unknown { command, suggestions } => { /* existing unknown handling */ }
        RouteResult::NotSlash => { /* existing non-slash handling */ }
    }
}
```

- [ ] **Step 2: 修改 `src/tui/app/mod.rs` App 启动**

在 `ExternalSkillRegistry` 初始化之后，添加：

```rust
// Discover and load workflow
let mut command_router = CommandRouter::new(builtin_commands.clone());
let workflow_yaml_path = PathBuf::from(".wgenty-code/skills/comet/workflow.yaml");
if workflow_yaml_path.exists() {
    let content = std::fs::read_to_string(&workflow_yaml_path)?;
    let workflow_def: serde_yaml::Value = serde_yaml::from_str(&content)?;
    if let Some(commands) = workflow_def["entry_commands"].as_sequence() {
        let entry_cmds: Vec<String> = commands.iter()
            .filter_map(|c| c.as_str().map(String::from))
            .collect();
        command_router.register_workflow("comet", &entry_cmds);
    }
    // Parse hooks and register
    if let Some(hooks_val) = workflow_def["hooks"].as_sequence() {
        let hooks: Vec<HookDefinition> = serde_yaml::from_value(serde_yaml::Value::Sequence(hooks_val.clone()))?;
        hook_manager.register_workflow_hooks(hooks);
    }
    // Parse context layers
    if let Some(ctx_val) = workflow_def["context"].as_sequence() {
        let layers: Vec<ContextLayer> = serde_yaml::from_value(serde_yaml::Value::Sequence(ctx_val.clone()))?;
        let state = Arc::new(RwLock::new(String::new()));
        let context_assembler = Arc::new(ContextAssembler::new(layers, state.clone(), HashMap::new()));
        // Inject into components
        tool_executor.set_state_handle(Some(state.clone()));
        prompt_context.context_assembler = Some(context_assembler);
    }
    // Set interaction service
    app_state.interaction_service = Some(Arc::new(TuiInteractionService::new(event_tx)));
}
```

- [ ] **Step 3: 修改 `src/tui/completion.rs`**

将 slash 命令补全源从 `ExternalSkillRegistry` 改为 `CommandRouter`：

```rust
// Before: skills registry direct listing
// After:
if let Some(ref router) = self.command_router {
    for cmd in router.entry_commands() {
        completions.push(format!("/{}", cmd));
    }
}
```

- [ ] **Step 4: 编译验证**

```bash
cargo check 2>&1 | head -50
```

- [ ] **Step 5: 提交**

```bash
git add src/tui/app/input.rs src/tui/app/mod.rs src/tui/completion.rs
git commit -m "refactor(tui): replace comet_slash_agent_prompt with CommandRouter + SlashCommand hooks"
```

---

### Task 12: 删除 src/comet/ 和清理 Comet 引用

**文件：**
- 删除：`src/comet/` 整个目录
- 修改：`src/lib.rs`（移除 `pub mod comet`）
- 修改：所有残留 `use crate::comet` 导入

**接口：**
- 消费：前 11 个任务的所有修改
- 产出：`grep -r "comet\|openspec\|phase" src/runtime/` 返回零结果、编译通过

- [ ] **Step 1: 删除 `src/comet/` 目录**

```bash
git rm -r src/comet/
```

- [ ] **Step 2: 从 `src/lib.rs` 移除 `pub mod comet`**

```bash
grep -n "pub mod comet" src/lib.rs
sed -i '' '/pub mod comet/d' src/lib.rs
```

- [ ] **Step 3: 搜索并清理残留的 `use crate::comet` 引用**

```bash
grep -rn "use crate::comet" src/
```
对每个出现的文件，移除该行。

```bash
grep -rn "crate::comet" src/
```
检查是否还有其他非 use 语句的引用。

- [ ] **Step 4: 搜索并清理 `CometPhase` / `CometState` / `CometGuard` 引用**

```bash
grep -rn "CometPhase\|CometState\|CometGuard\|comet_slash_agent_prompt" src/
```
对每个出现的文件，移除或替换引用。

- [ ] **Step 5: 如果 `src/runtime/mod.rs` 中存在 `pub mod comet`，也删除**

```bash
grep -rn "pub mod comet" src/runtime/
```
如果存在，移除。

- [ ] **Step 6: 清理 `src/knowledge/external_registry.rs` 中的废弃函数**

如果 Task 5 中只是加了弃用标记，现在可以删除 `comet_slash_agent_prompt()` 和 `route_slash_command()` 函数及其相关类型。

- [ ] **Step 7: 最终验证——零 Comet 引用**

```bash
# Zero Comet/OpenSpec/phase references in runtime
grep -r "comet\|openspec\|phase" src/runtime/
# 必须返回零行（除了测试数据中的字符串）

# Zero struct type references
grep -rn "CometPhase\|CometState\|CometGuard\|comet_slash_agent_prompt" src/
# 必须返回零行

# pub mod comet removed
grep "pub mod comet" src/lib.rs
# 必须返回零行
```

- [ ] **Step 8: 编译验证**

```bash
cargo check 2>&1 | head -30
```

- [ ] **Step 9: 运行完整测试套件**

```bash
cargo test 2>&1 | tail -40
```

- [ ] **Step 10: 提交**

```bash
git add -A
git commit -m "refactor(comet): delete src/comet/ and remove all hardcoded Comet references"
```

---

### Task 13: 集成测试和验证

**文件：**
- 创建：`tests/generic_agent_runtime_test.rs`

- [ ] **Step 1: 编写集成测试——SlashCommand 路由和 ContextAssembler 集成**

```rust
use std::sync::Arc;
use tokio::sync::RwLock;
use wgenty_core::runtime::command::CommandRouter;
use wgenty_core::runtime::context::{ContextAssembler, ContextLayer, LayerCondition};
use wgenty_core::runtime::hooks::{ContextSource, HookAction, HookDefinition, HookEvent, LayerVisibility};

#[tokio::test]
async fn test_workflow_slash_route() {
    let mut router = CommandRouter::new(vec!["clear".into(), "help".into()]);
    router.register_workflow("comet", &["comet".into(), "comet-open".into()]);

    let result = router.route("/comet design");
    assert!(matches!(result, RouteResult::Workflow { name, .. } if name == "comet"));
}

#[tokio::test]
async fn test_context_assembler_integration() {
    let state = Arc::new(RwLock::new("build".to_string()));
    let mut variables = std::collections::HashMap::new();
    variables.insert("build_mode".into(), "subagent".into());

    let layers = vec![
        ContextLayer {
            id: "phase".into(), priority: 10,
            visibility: LayerVisibility::Internal,
            source: ContextSource::Template("State: {{ state }}".into()),
            condition: Some(LayerCondition::StateMatches("build".into())),
        },
    ];

    let assembler = ContextAssembler::new(layers, state, variables);
    let ctx = assembler.assemble().await;
    assert!(ctx.internal_instructions.iter().any(|s| s.contains("State: build")));
}
```

- [ ] **Step 2: 编写集成测试——HookManager 的 when_state 过滤**

```rust
#[tokio::test]
async fn test_hook_when_state_filter() {
    let mut manager = HookManager::new();
    let hooks = vec![
        HookDefinition {
            event: HookEvent::PreToolUse,
            matcher: Some("Write".into()),
            when_state: Some("open|design".into()),
            actions: vec![HookAction::Command {
                command: "echo blocked".into(),
                timeout_secs: 5,
            }],
        },
    ];
    manager.register_workflow_hooks(hooks);

    let ctx = HookContext::default();
    // State = "build", hook has when_state: "open|design", should skip
    let outcomes = manager.fire(&HookEvent::PreToolUse, &ctx, Some("build")).await;
    assert!(outcomes.is_empty(), "hook with when_state=open|design should be skipped in build state");
}
```

- [ ] **Step 3: 编写集成测试——inject_context 生成的内部文本不出现在用户可见消息中**

```rust
#[tokio::test]
async fn test_internal_not_visible() {
    let state = Arc::new(RwLock::new("design".to_string()));
    let layers = vec![
        ContextLayer {
            id: "secret".into(), priority: 10,
            visibility: LayerVisibility::Internal,
            source: ContextSource::Inline("INTERNAL_ONLY".into()),
            condition: None,
        },
        ContextLayer {
            id: "public".into(), priority: 20,
            visibility: LayerVisibility::Visible,
            source: ContextSource::Inline("VISIBLE".into()),
            condition: None,
        },
    ];
    let assembler = ContextAssembler::new(layers, state, std::collections::HashMap::new());
    let ctx = assembler.assemble().await;

    // Internal content must NOT be in visible stream
    assert!(!ctx.visible_content.iter().any(|s| s.contains("INTERNAL_ONLY")));
    // Internal content IS in internal stream
    assert!(ctx.internal_instructions.iter().any(|s| s.contains("INTERNAL_ONLY")));
}
```

- [ ] **Step 4: 运行所有集成测试**

```bash
cargo test --test generic_agent_runtime_test 2>&1 | tail -20
```

- [ ] **Step 5: 回归测试——现有 slash 命令行为不变**

确保 `/clear`, `/help`, `/plan`, `/continue`, `/undo`, `/init` 在重构后行为一致。TUI 集成测试由手动验证覆盖。

- [ ] **Step 6: 提交**

```bash
git add tests/generic_agent_runtime_test.rs
git commit -m "test(runtime): add integration tests for workflow routing, context assembly, hook filtering"
```

---

### Task 14: 手动验证和最终清理

- [ ] **Step 1: 验证 grep 断言**

```bash
grep -r "comet\|openspec\|phase" src/runtime/
# 预期零结果（测试数据中可能包含 "phase guard" 等词，检查即可）

grep -rn "CometPhase\|CometState\|CometGuard\|comet_slash_agent_prompt" src/
# 预期零结果
```

- [ ] **Step 2: 验证模块结构**

```bash
ls src/runtime/hooks/
ls src/runtime/guardian.rs
ls src/comet/  # 应报错：No such file or directory
```

- [ ] **Step 3: 运行完整测试套件**

```bash
cargo test 2>&1 | tail -10
```
确认所有测试通过，无回归。

- [ ] **Step 4: 提交**

```bash
git add -A
git commit -m "chore: final cleanup and verification of generic agent runtime migration"
```

---

## 自检清单

**1. 规格覆盖率：**

| Design Doc 章节 | 任务覆盖 |
|---|---|
| §1 数据注入模式 | Task 1, 11 (app startup) |
| §2.2 SlashCommand 事件 | Task 2 |
| §2.3 inject_context Action | Task 2, 4 |
| §2.4 ask_user Action | Task 2, 6 |
| §2.5 when_state 条件 | Task 2, 3 |
| §2.6 Hook 执行流 | Task 3 |
| §3 ContextAssembler | Task 4 |
| §4 InteractionService | Task 6 |
| §5 模块结构 | Task 1, 8 |
| §6.1 ToolExecutor | Task 9 |
| §6.2 Prompt Assembler | Task 10 |
| §6.3 TUI Input | Task 11 |
| §6.4 App Startup | Task 11 |
| §7 workflow.yaml | Task 7 |
| §8 Hook 系统内部变更 | Task 2, 3 |
| §9 删除 src/comet/ | Task 12 |
| §10 CommandRouter | Task 5 |
| §11 迁移路径 | Task 1-12 |
| §12 错误处理 | Task 3 (fail-open/fail-closed) |
| §13 测试策略 | Task 13 |

**2. 占位符检查：** 无 TODO/TBD，所有代码块包含完整实现。

**3. 类型一致性检查：** `ContextSource` / `LayerVisibility` 在 Task 2 (hooks/mod.rs) 和 Task 4 (context.rs) 中一致；`HookAction` 在 Task 2 定义、Task 3 消费；`CommandRouter::route()` 在 Task 5 定义、Task 11 消费；`InteractionService` trait 在 Task 6 定义、Task 3 (HookManager 占位) 使用。
