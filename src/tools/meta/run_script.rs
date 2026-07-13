//! Run Script Tool — execute Rhai scripts for programmatic agent orchestration.
//!
//! Gives the model control-flow capabilities (loops, conditions, variables)
//! through a sandboxed Rhai scripting engine. Exposes key agent APIs as Rhai
//! functions: `task`, `grep`, `read`, `exec`, `log`.

use crate::agent::{AgentCoordinator, ToolContext};
use crate::api::ApiClient;
use crate::config::Settings;
use crate::teams::subagent_loop::run_subagent_loop;
use crate::tools::{Tool, ToolError, ToolOutput, ToolRegistry};
use async_trait::async_trait;
use rhai::Engine;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

pub struct RunScriptTool {
    settings: Settings,
    tool_registry: std::sync::Weak<ToolRegistry>,
    coordinator: Arc<AgentCoordinator>,
}

impl RunScriptTool {
    pub fn new(
        settings: Settings,
        tool_registry: std::sync::Weak<ToolRegistry>,
        coordinator: Arc<AgentCoordinator>,
    ) -> Self {
        Self {
            settings,
            tool_registry,
            coordinator,
        }
    }
}

#[async_trait]
impl Tool for RunScriptTool {
    fn name(&self) -> &str {
        "run_script"
    }
    fn description(&self) -> &str {
        "Execute a Rhai script to orchestrate multiple agent operations with loops, conditions, and variables. Exposed functions: task(prompt), grep(pattern), read(path), exec(cmd), log(msg). Use for complex multi-step workflows. Rhai syntax is similar to Rust/JS."
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "script": {
                    "type": "string",
                    "description": "Rhai script to execute. Available functions: task(prompt), grep(pattern), read(path), exec(cmd), log(msg). Script must return a string."
                }
            },
            "required": ["script"]
        })
    }

    async fn execute(&self, _input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        Err(ToolError {
            message: "run_script requires trusted agent context".to_string(),
            code: Some("missing_agent_context".to_string()),
        })
    }

    async fn execute_with_context(
        &self,
        context: &ToolContext<'_>,
        input: serde_json::Value,
    ) -> Result<ToolOutput, ToolError> {
        let script = input["script"].as_str().unwrap_or("");
        let tool_registry = self.tool_registry.upgrade().ok_or_else(|| ToolError {
            message: "Tool registry unavailable".to_string(),
            code: Some("registry_dropped".to_string()),
        })?;

        // ── Build Rhai engine with sandboxed functions ──────────────────
        let mut engine = Engine::new_raw();
        engine.set_max_operations(5000);

        let allowed_tools: Vec<String> = tool_registry
            .list()
            .iter()
            .map(|t| t.name().to_string())
            .filter(|n| n != "task" && n != "delegate" && n != "run_script")
            .collect();

        let call_count = Arc::new(AtomicU32::new(0));
        let output = Arc::new(Mutex::new(String::new()));

        // register task(prompt) -> String
        {
            let reg = tool_registry.clone();
            let settings = self.settings.clone();
            let tools = allowed_tools.clone();
            let cnt = call_count.clone();
            let rt = tokio::runtime::Handle::current();
            let caller_context = context.agent.clone();
            let coordinator = self.coordinator.clone();
            engine.register_fn("task", move |prompt: String| -> String {
                let n = cnt.fetch_add(1, Ordering::Relaxed);
                if n >= 10 { return "[ERROR] Max 10 subagent calls per script".to_string(); }
                let client = ApiClient::new(settings.clone());
                let coordinator = coordinator.clone();
                let caller = caller_context.clone();
                rt.block_on(async {
                    // Reserve a coordinator-owned child derived from the trusted
                    // caller context (never synthesized from tool arguments).
                    let reservation = match coordinator
                        .reserve_child(&caller, crate::agent::SpawnChildRequest::new(&prompt))
                        .await
                    {
                        Ok(r) => r,
                        Err(e) => return format!("[ERROR] coordinator reserve failed: {}", e),
                    };
                    let child_context = reservation.context.clone();
                    let result = run_subagent_loop(
                        &client, &reg, &child_context, coordinator.clone(),
                        "You are a sub-agent in a Rhai script. Execute the task precisely and return a concise result.",
                        &prompt, &tools, 10, 120, None, None,
                    ).await;
                    let (terminal, content) = match result {
                        Ok(r) => (
                            crate::agent::ChildTerminal::Completed {
                                summary: r.chars().take(500).collect(),
                            },
                            r,
                        ),
                        Err(e) => (
                            crate::agent::ChildTerminal::Failed {
                                code: "subagent_failed".to_string(),
                                partial_result: None,
                            },
                            format!("[ERROR] {}", e),
                        ),
                    };
                    let _ = coordinator.finish_child(&child_context, terminal).await;
                    content
                })
            });
        }

        // register grep(pattern) -> String
        engine.register_fn("grep", move |pattern: String| -> String {
            let output = std::process::Command::new("rg")
                .args(["-l", &pattern])
                .output();
            match output {
                Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
                Err(e) => format!("[ERROR] grep failed: {}", e),
            }
        });

        // register read(path) -> String
        engine.register_fn("read", move |path: String| -> String {
            std::fs::read_to_string(&path).unwrap_or_else(|e| format!("[ERROR] read failed: {}", e))
        });

        // register exec(cmd) -> String
        engine.register_fn("exec", move |cmd: String| -> String {
            let output = std::process::Command::new("sh")
                .arg("-c")
                .arg(&cmd)
                .output();
            match output {
                Ok(o) => {
                    let stdout = String::from_utf8_lossy(&o.stdout).to_string();
                    let stderr = String::from_utf8_lossy(&o.stderr).to_string();
                    let stderr_part = if stderr.is_empty() {
                        String::new()
                    } else {
                        format!("\n[stderr]\n{}", stderr)
                    };
                    format!("{}{}", stdout, stderr_part)
                }
                Err(e) => format!("[ERROR] exec failed: {}", e),
            }
        });

        // register log(msg) -> appends to output
        {
            let out = output.clone();
            engine.register_fn("log", move |msg: String| {
                if let Ok(mut o) = out.lock() {
                    o.push_str(&msg);
                    o.push('\n');
                }
            });
        }

        // ── Compile and run ────────────────────────────────────────────
        let ast = engine.compile(script).map_err(|e| ToolError {
            message: format!("Rhai compile error: {}", e),
            code: Some("rhai_compile_error".to_string()),
        })?;

        let result: String = engine.eval_ast(&ast).map_err(|e| ToolError {
            message: format!("Rhai runtime error: {}", e),
            code: Some("rhai_runtime_error".to_string()),
        })?;

        let log_output = output.lock().unwrap().clone();
        let final_output = if log_output.is_empty() {
            result
        } else {
            format!("{}\n[Log]\n{}", result, log_output)
        };

        Ok(ToolOutput {
            output_type: "text".to_string(),
            content: final_output,
            metadata: {
                let mut m = HashMap::new();
                m.insert(
                    "subagent_calls".to_string(),
                    serde_json::json!(call_count.load(Ordering::Relaxed)),
                );
                m
            },
        })
    }
}
