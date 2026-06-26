use std::collections::HashMap;

/// A parsed slash-command invocation.
#[derive(Debug, Clone)]
pub struct CommandInvocation {
    pub name: String,
    pub args: String,
    pub raw_input: String,
}

/// The result of routing a user input.
#[derive(Debug, Clone)]
pub enum RouteResult {
    /// Matched a built-in command (e.g. /clear, /help).
    BuiltIn,
    /// Matched a registered workflow entry command.
    Workflow {
        name: String,
        command: String,
        args: String,
    },
    /// Unrecognised slash command.
    Unknown {
        command: String,
        suggestions: Vec<String>,
    },
    /// Input does not start with `/`.
    NotSlash,
}

/// Pure-data router that maps slash-command input to a route result.
///
/// Built-ins are resolved first. Afterwards the router checks its
/// workflow-command registry. The workflow name stored in `RouteResult::Workflow`
/// is never hard-coded — it comes from `register_workflow`.
pub struct CommandRouter {
    builtins: Vec<String>,
    workflow_commands: HashMap<String, String>,
}

impl CommandRouter {
    pub fn new(builtins: Vec<String>) -> Self {
        CommandRouter {
            builtins,
            workflow_commands: HashMap::new(),
        }
    }

    /// Register a workflow's entry commands. Every command in `entry_commands`
    /// maps to `name`.
    pub fn register_workflow(&mut self, name: &str, entry_commands: &[String]) {
        for cmd in entry_commands {
            self.workflow_commands.insert(cmd.clone(), name.to_string());
        }
    }

    /// Route a raw user input. Returns `NotSlash` when the input does not start
    /// with `/`, `BuiltIn` when it matches a built-in, `Workflow` when a
    /// registered workflow entry command is found, or `Unknown` otherwise.
    pub fn route(&self, input: &str) -> RouteResult {
        if !input.starts_with('/') {
            return RouteResult::NotSlash;
        }
        let text = &input[1..];
        let parts: Vec<&str> = text.splitn(2, ' ').collect();
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
        RouteResult::Unknown {
            command,
            suggestions: vec![],
        }
    }

    /// Returns all known entry commands (built-ins + registered workflow
    /// commands).
    pub fn entry_commands(&self) -> Vec<String> {
        let mut cmds: Vec<String> = self.builtins.clone();
        for cmd in self.workflow_commands.keys() {
            cmds.push(cmd.clone());
        }
        cmds
    }
}

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
        router.register_workflow("example-workflow", &["workflow".into(), "wf".into()]);
        match router.route("/workflow fix bug") {
            RouteResult::Workflow { name, command, args } => {
                assert_eq!(name, "example-workflow");
                assert_eq!(command, "workflow");
                assert_eq!(args, "fix bug");
            }
            _ => panic!("expected Workflow route"),
        }
    }

    #[test]
    fn test_workflow_alias() {
        let mut router = CommandRouter::new(vec![]);
        router.register_workflow("example-workflow", &["workflow".into(), "wf".into()]);
        match router.route("/wf some args") {
            RouteResult::Workflow { name, command, args } => {
                assert_eq!(name, "example-workflow");
                assert_eq!(command, "wf");
                assert_eq!(args, "some args");
            }
            _ => panic!("expected Workflow route for alias"),
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
        assert!(matches!(
            router.route("/unknown"),
            RouteResult::Unknown { .. }
        ));
    }

    #[test]
    fn test_slash_with_no_args() {
        let mut router = CommandRouter::new(vec![]);
        router.register_workflow("example-workflow", &["workflow".into()]);
        match router.route("/workflow") {
            RouteResult::Workflow { name, command, args } => {
                assert_eq!(name, "example-workflow");
                assert_eq!(command, "workflow");
                assert_eq!(args, "");
            }
            _ => panic!("expected Workflow route with empty args"),
        }
    }

    #[test]
    fn test_entry_commands_includes_builtins_and_workflows() {
        let mut router = CommandRouter::new(vec!["help".into(), "clear".into()]);
        router.register_workflow("example-workflow", &["workflow".into(), "wf".into()]);
        let cmds = router.entry_commands();
        assert!(cmds.contains(&"help".to_string()));
        assert!(cmds.contains(&"clear".to_string()));
        assert!(cmds.contains(&"workflow".to_string()));
        assert!(cmds.contains(&"wf".to_string()));
        assert_eq!(cmds.len(), 4);
    }
}
