//! Task Tool — subagent spawning for complex, multi-step tasks.
//!
//! The `task` tool allows the parent agent to delegate work to an isolated
//! subagent with its own message context, filtered tool set (no recursive
//! `task` calls to prevent explosion), and a complete agent loop.
//!
//! Available subagent types:
//! - `general-purpose` (default) — general tool-use tasks
//! - `explore`                   — codebase search and analysis
//! - `plan`                      — architecture planning and breakdown

use crate::api::ApiClient;
use crate::config::Settings;
use crate::teams::subagent_loop::run_subagent_loop;
use crate::tools::{Tool, ToolError, ToolOutput};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;


/// Detect whether a prompt is complex enough to warrant RLM delegation.
fn is_complex_task(prompt: &str, use_small_model: bool) -> bool {
    if use_small_model {
        return false; // User explicitly asked for cheap model
    }

    let prompt = prompt.trim();
    let len = prompt.len();

    // Long prompts are likely complex
    if len > 500 {
        return true;
    }

    // Multi-step indicators
    let keywords = [
        "refactor", "implement", "migrate", "restructure", "redesign",
        "rewrite", "create", "build", "develop", "analyze", "research",
        "first", "second", "third", "then", "after", "before",
        "step", "phase", "stage", "component", "module", "system",
    ];

    let lower = prompt.to_lowercase();
    let hits = keywords.iter().filter(|kw| lower.contains(*kw)).count();

    // Multiple complexity keywords
    if hits >= 4 {
        return true;
    }

    // Multiple sections (separated by blank lines or numbered lists)
    let sections = prompt.split("\n\n").count();
    let digits = prompt.matches(|c: char| c.is_ascii_digit()).count();

    if sections >= 3 || digits > 5 {
        return true;
    }

    false
}


pub struct TaskTool {
    settings: Settings,
    tool_registry: std::sync::Weak<crate::tools::ToolRegistry>,
    background_manager: std::sync::Arc<crate::tools::execution::background::BackgroundManager>,
    /// Tracks currently running subagents to enforce max_concurrent limit.
    active_count: Arc<AtomicUsize>,
}

impl TaskTool {
    pub fn new(
        settings: Settings,
        tool_registry: std::sync::Weak<crate::tools::ToolRegistry>,
        background_manager: std::sync::Arc<crate::tools::execution::background::BackgroundManager>,
    ) -> Self {
        Self {
            settings,
            tool_registry,
            background_manager,
            active_count: Arc::new(AtomicUsize::new(0)),
        }
    }
}

#[async_trait]
impl Tool for TaskTool {
    fn name(&self) -> &str {
        "task"
    }

    fn is_read_only(&self) -> bool {
        false
    }

    fn description(&self) -> &str {
        "Launch a subagent to handle complex, multi-step tasks. \
         Available types: general-purpose (default), explore (codebase search), \
         plan (architecture). Subagents have isolated context and filtered tools \
         (no recursive task spawning). Use for: parallel work, context-heavy \
         research, complex multi-step tasks."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "subagent_type": {
                    "type": "string",
                    "description": "Type of subagent: general-purpose, explore, or plan",
                    "enum": ["general-purpose", "explore", "plan"]
                },
                "description": {
                    "type": "string",
                    "description": "Short (3-5 word) description of the task"
                },
                "background": {
                    "type": "boolean",
                    "description": "Run subagent in background. Returns task_id immediately; result delivered later. Default: false"
                },
                "use_small_model": {
                    "type": "boolean",
                    "description": "When true and a small model is configured, run the subagent with a smaller/cheaper model. Use for simple, self-contained tasks (e.g., reading files, searching, running a single command). Default: false"
                },
                "prompt": {
                    "type": "string",
                    "description": "The detailed task for the subagent to perform"
                }
            },
            "required": ["description", "prompt"]
        })
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let _subagent_type = input["subagent_type"].as_str().unwrap_or("general-purpose");
        let description = input["description"].as_str().unwrap_or("Subagent task");
        let prompt = input["prompt"].as_str().unwrap_or("");
        let background = input["background"].as_bool().unwrap_or(false);

        tracing::info!(
            subagent_type = _subagent_type,
            description = description,
            prompt_len = prompt.len(),
            background = background,
            "TaskTool: executing subagent"
        );

        // Upgrade the Weak reference to the tool registry.
        let tool_registry = self.tool_registry.upgrade().ok_or_else(|| ToolError {
            message: "Tool registry is no longer available".to_string(),
            code: Some("registry_dropped".to_string()),
        })?;

        // Filter tools: exclude "task" when depth exceeds limit.
        let depth = input["_subagent_depth"].as_u64().unwrap_or(0) as usize;
        let allowed_tools: Vec<String> = tool_registry
            .list()
            .iter()
            .map(|t| t.name().to_string())
            .filter(|name| { if name == "task" { depth < self.settings.max_subagent_depth } else { true } })
            .collect();

        // Build system prompt based on subagent type.
        let system_prompt = match _subagent_type {
            "explore" => {
                "You are a subagent spawned by a coordinator. The coordinator is waiting for your result. Do not attempt to coordinate other agents yourself — focus solely on your assigned task. Return a complete, self-contained result so the coordinator can proceed without follow-up questions.\n\nYou are a code exploration subagent. Your role is to search and \
                 analyze codebases thoroughly.\n\nKey responsibilities:\n\
                 1. Search for relevant files and code patterns\n\
                 2. Read and understand code structure\n\
                 3. Analyze dependencies and relationships\n\
                 4. Report findings clearly and concisely\n\n\
                 Use search, grep, glob, and file_read tools to explore the \
                 codebase. Be thorough but efficient — focus on answering the \
                 specific question."
            }
            "plan" => {
                "You are a subagent spawned by a coordinator. The coordinator is waiting for your result. Do not attempt to coordinate other agents yourself — focus solely on your assigned task. Return a complete, self-contained result so the coordinator can proceed without follow-up questions.\n\nYou are a planning subagent. Your role is to break down complex \
                 tasks into actionable steps.\n\nKey responsibilities:\n\
                 1. Analyze task requirements\n\
                 2. Identify key files and components\n\
                 3. Break down the work into logical steps\n\
                 4. Consider dependencies, risks, and trade-offs\n\n\
                 Use file_read and search tools to understand the codebase before \
                 planning. Be thorough and structured in your analysis."
            }
            _ => {
                "You are a subagent spawned by a coordinator. The coordinator is waiting for your result. Do not attempt to coordinate other agents yourself — focus solely on your assigned task. Return a complete, self-contained result so the coordinator can proceed without follow-up questions.\n\nYou are a general-purpose subagent. Complete the assigned task \
                 efficiently using the available tools.\n\nKey responsibilities:\n\
                 1. Understand the task requirements\n\
                 2. Use appropriate tools to accomplish the task\n\
                 3. Provide clear and complete results\n\
                 4. Handle edge cases gracefully\n\n\
                 If you need to read files, search, or execute commands, use the \
                 appropriate tools. Return a complete summary of what was accomplished."
            }
        };

        // Build the full user prompt with context.
        let full_prompt = format!(
            "## Task Description\n{}\n\n## Task Details\n{}",
            description, prompt
        );

        // ── Guard: depth limit ──────────────────────────────────────────
        let depth = input["_subagent_depth"].as_u64().unwrap_or(0) as usize;
        if depth >= self.settings.max_subagent_depth {
            return Ok(ToolOutput {
                output_type: "text".to_string(),
                content: format!(
                    "Maximum subagent depth ({}) reached. Refusing to spawn deeper subagent.",
                    self.settings.max_subagent_depth
                ),
                metadata: HashMap::new(),
            });
        }

        // ── Guard: concurrency limit ────────────────────────────────────
        let current = self.active_count.load(Ordering::SeqCst);
        if current >= self.settings.max_concurrent_subagents {
            return Ok(ToolOutput {
                output_type: "text".to_string(),
                content: format!(
                    "Maximum concurrent subagents ({}) reached ({} running). Try again later.",
                    self.settings.max_concurrent_subagents, current
                ),
                metadata: HashMap::new(),
            });
        }
        self.active_count.fetch_add(1, Ordering::SeqCst);

        // Use small model when requested and configured.
        // Falls back to main model's base_url/api_key when small_model fields are absent.
        let use_small = input["use_small_model"].as_bool().unwrap_or(false);
        let api_client = if use_small {
            if let Some(ref small_model) = self.settings.small_model {
                let mut small_settings = self.settings.clone();
                small_settings.model = small_model.clone();
                small_settings.api.max_tokens = 2048;
                if let Some(ref url) = self.settings.small_model_base_url {
                    small_settings.api.base_url = url.clone();
                }
                if let Some(ref key) = self.settings.small_model_api_key {
                    small_settings.api.api_key = Some(key.clone());
                }
                if let Some(ref appkey) = self.settings.small_model_appkey {
                    small_settings.api.api_key = Some(appkey.clone());
                }
                ApiClient::new(small_settings)
            } else {
                ApiClient::new(self.settings.clone())
            }
        } else {
            ApiClient::new(self.settings.clone())
        };

        // Run the subagent loop (capped at 30 rounds).
        if background {
            // ── Background mode: spawn and return immediately ──────────────
            let desc = description.to_string();
            let prompt_owned = full_prompt;
            let sys_prompt = system_prompt.to_string();
            let bg = self.background_manager.clone();
            let active = self.active_count.clone();
            let reg = tool_registry.clone();
            let tools = allowed_tools.clone();
            let api_client_bg = ApiClient::new(self.settings.clone());
            let timeout_secs = self.settings.subagent_timeout_secs;

            tokio::spawn(async move {
                let result = run_subagent_loop(
                    &api_client_bg,
                    &reg,
                    &sys_prompt,
                    &prompt_owned,
                    &tools,
                    30,
                    timeout_secs,
                )
                .await;

                active.fetch_sub(1, Ordering::SeqCst);

                let (success, content) = match result {
                    Ok(r) => (true, r),
                    Err(e) => (false, format!("Subagent error: {}", e)),
                };

                bg.push_subagent_result(&desc, &content, success).await;
            });

            Ok(ToolOutput {
                output_type: "text".to_string(),
                content: format!(
                    "[Subagent launched in background]\ntype: {}\ndescription: {}\nstatus: running\n\nThe subagent result will be delivered when it completes.",
                    _subagent_type, description
                ),
                metadata: {
                    let mut m = HashMap::new();
                    m.insert("subagent_type".to_string(), serde_json::json!(_subagent_type));
                    m.insert("description".to_string(), serde_json::json!(description));
                    m.insert("background".to_string(), serde_json::json!(true));
                    m.insert("execution_mode".to_string(), serde_json::json!("background"));
                    m
                },
            })
        } else {
            // ── Synchronous mode: block until complete ─────────────────────
            let result = if is_complex_task(&full_prompt, use_small) {
                tracing::info!(
                    target: "rlm",
                    phase = "auto_route",
                    "Complex task detected, routing to RLM pipeline"
                );
                crate::tools::meta::rlm::run_rlm_pipeline(
                    &self.settings,
                    tool_registry.clone(),
                    description,
                    prompt,
                    depth,
                )
                .await
            } else {
                run_subagent_loop(
                    &api_client,
                    &tool_registry,
                    system_prompt,
                    &full_prompt,
                    &allowed_tools,
                    30,
                    self.settings.subagent_timeout_secs,
                )
                .await
            };
            self.active_count.fetch_sub(1, Ordering::SeqCst);

            match result {
                Ok(result) => {
                    let mut metadata = HashMap::new();
                    metadata.insert(
                        "subagent_type".to_string(),
                        serde_json::json!(_subagent_type),
                    );
                    metadata.insert("description".to_string(), serde_json::json!(description));

                    Ok(ToolOutput {
                        output_type: "text".to_string(),
                        content: result,
                        metadata,
                    })
                }
                Err(e) => Err(ToolError {
                    message: e,
                    code: Some("subagent_error".to_string()),
                }),
            }
        }
    }
}
