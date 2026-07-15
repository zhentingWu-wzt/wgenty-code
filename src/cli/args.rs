//! CLI Arguments

use super::CliArgs;
use std::sync::Arc;
use tokio::sync::RwLock;

pub type Cli = CliArgs;

impl Cli {
    pub async fn run_async(&self, state: crate::state::AppState) -> anyhow::Result<()> {
        if self.version {
            println!("wgenty code {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }

        if self.info {
            self.print_system_info();
            return Ok(());
        }

        match &self.command {
            Some(super::Commands::Repl { prompt }) => {
                self.run_repl(state, prompt.clone()).await?;
            }
            Some(super::Commands::Query { prompt }) => {
                self.run_query(state, prompt.clone()).await?;
            }
            Some(super::Commands::Config { action }) => {
                self.run_config(action)?;
            }
            Some(super::Commands::Mcp { action }) => {
                self.run_mcp(action).await?;
            }
            Some(super::Commands::Plugin { action }) => {
                self.run_plugin(action).await?;
            }
            Some(super::Commands::Memory { action }) => {
                self.run_memory(action).await?;
            }
            Some(super::Commands::Voice { push_to_talk }) => {
                self.run_voice(state, *push_to_talk).await?;
            }
            Some(super::Commands::Init { name }) => {
                self.run_init(name.clone())?;
            }
            Some(super::Commands::Update) => {
                self.run_update()?;
            }
            Some(super::Commands::Help { topic }) => {
                self.run_help(topic.clone())?;
            }
            Some(super::Commands::Services { action }) => {
                self.run_services(state, action).await?;
            }
            Some(super::Commands::Agent { agent_type, prompt }) => {
                self.run_agent(state, agent_type, prompt).await?;
            }
            Some(super::Commands::MagicDocs { action }) => {
                self.run_magic_docs(state, action).await?;
            }
            Some(super::Commands::TeamSync { action }) => {
                self.run_team_sync(state, action).await?;
            }
            Some(super::Commands::StressTest {
                concurrency,
                iterations,
            }) => {
                self.run_stress_test(*concurrency, *iterations).await?;
            }
            Some(super::Commands::Sandbox { action }) => {
                self.run_sandbox(action).await?;
            }
            Some(super::Commands::Skills { action }) => {
                self.run_skills(action).await?;
            }
            #[cfg(feature = "daemon")]
            Some(super::Commands::Daemon { port }) => {
                crate::daemon::run(state, *port).await?;
            }
            #[cfg(not(feature = "daemon"))]
            Some(super::Commands::Daemon { .. }) => {
                return Err(anyhow::anyhow!(
                    "Daemon feature is not enabled. Rebuild with: cargo build --features daemon"
                ));
            }
            None => {
                self.run_repl(state, None).await?;
            }
        }

        Ok(())
    }

    fn print_system_info(&self) {
        println!();
        println!("  System Information");
        println!();
        println!("  {:20} {}", "Version:", env!("CARGO_PKG_VERSION"));
        println!("  {:20} {}", "OS:", std::env::consts::OS);
        println!("  {:20} {}", "Architecture:", std::env::consts::ARCH);
        println!(
            "  {:20} {}",
            "Working Directory:",
            std::env::current_dir()
                .map(|d| d.display().to_string())
                .unwrap_or_else(|_| "<unknown>".to_string())
        );
        println!();
    }

    #[cfg(feature = "daemon")]
    async fn run_repl(
        &self,
        state: crate::state::AppState,
        prompt: Option<String>,
    ) -> anyhow::Result<()> {
        // Auto-install bundled skills on first run. Skip the disk write entirely
        // when already installed (the common case after first run); otherwise
        // run the install in a background blocking task so it never delays
        // terminal entry or the first rendered frame.
        if !crate::knowledge::embedded::is_auto_installed() {
            tokio::task::spawn_blocking(|| {
                let installed = crate::knowledge::embedded::auto_install();
                if !installed.is_empty() {
                    tracing::info!(count = installed.len(), "auto-installed bundled skills");
                }
            });
        }

        use crate::tui::app::{self, App};
        use crate::tui::client::DaemonClient;
        use crossterm::{
            event::{
                DisableMouseCapture, EnableMouseCapture, KeyboardEnhancementFlags,
                PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
            },
            execute,
            terminal::{
                disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
            },
        };
        use ratatui::{backend::CrosstermBackend, Terminal};
        use std::io;

        // Clone settings before state is consumed by start_daemon
        let settings_handle: crate::config::watcher::SettingsHandle =
            std::sync::Arc::new(std::sync::RwLock::new(state.settings.clone()));

        // CodeGraph availability notice. Printed before entering the alternate
        // screen so it's visible; silent when installed+initialized or dismissed.
        if let Some(msg) = crate::mcp::codegraph::install_state_notice(
            crate::mcp::codegraph::probe_install_state(&state.settings),
        ) {
            eprintln!("{msg}");
        }

        // ── Terminal setup FIRST (render-first) ──────────────────────────
        // Enter the alternate screen and enable raw mode *before* the blocking
        // daemon startup so the user sees immediate visual feedback (splash)
        // instead of a blank terminal during MCP connect / session load.
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        execute!(stdout, EnableMouseCapture)?;

        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = disable_raw_mode();
            let _ = execute!(io::stdout(), PopKeyboardEnhancementFlags);
            let _ = execute!(io::stdout(), DisableMouseCapture);
            let _ = execute!(io::stdout(), LeaveAlternateScreen);
            default_hook(info);
        }));

        enable_raw_mode()?;
        execute!(
            io::stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )?;

        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        // ── Splash screen ────────────────────────────────────────────────
        terminal.draw(|f| {
            use ratatui::layout::Alignment;
            use ratatui::widgets::Paragraph;
            let area = f.area();
            f.render_widget(
                Paragraph::new("Starting wgenty-code…").alignment(Alignment::Center),
                area,
            );
        })?;
        crate::utils::startup_timing::mark("splash rendered (first paint)");

        // ── Start daemon (session load deferred to background) ───────────
        let (base_url, shutdown_tx, daemon_handle) = match app::start_daemon(state).await {
            Ok(result) => result,
            Err(e) => {
                let _ = disable_raw_mode();
                let _ = execute!(io::stdout(), PopKeyboardEnhancementFlags);
                let _ = execute!(io::stdout(), DisableMouseCapture);
                let _ = execute!(io::stdout(), LeaveAlternateScreen);
                let _ = std::panic::take_hook();
                return Err(e);
            }
        };
        crate::utils::startup_timing::mark("daemon started");
        // Create client and app
        let client = DaemonClient::new(base_url);
        crate::utils::startup_timing::mark("daemon client created");
        let session_id = uuid::Uuid::new_v4().to_string();
        let mut app = App::new(client, session_id, settings_handle);
        crate::utils::startup_timing::mark("app new returned");

        // Load plugin commands into completion engine
        {
            let plugin_manager = crate::plugins::PluginManager::new();
            if plugin_manager.load_all().await.is_ok() {
                let commands = plugin_manager.command_registry().list().await;
                let entries: Vec<crate::tui::completion::CommandEntry> = commands
                    .into_iter()
                    .map(|c| crate::tui::completion::CommandEntry {
                        name: c.definition.name,
                        description: c.definition.description,
                        args_hint: c.definition.usage,
                        category: "Plugin".to_string(),
                    })
                    .collect();
                if !entries.is_empty() {
                    if let Some(ref mut engine) = app.completion_engine {
                        // Merge plugin commands with existing builtins
                        let mut all = engine.commands.clone();
                        // Add plugin commands, skipping duplicates
                        for entry in entries {
                            if !all.iter().any(|e| e.name == entry.name) {
                                all.push(entry);
                            }
                        }
                        engine.update_commands(all);
                    }
                }
            }
        }

        // Send initial prompt if given
        if let Some(p) = prompt {
            let tx = app.event_sender();
            let _ = tx.send(crate::tui::app::AppEvent::Submit(p));
        }
        crate::utils::startup_timing::mark("app initialized (entering run loop)");

        // Run the TUI - the terminal was created above (before daemon start)
        // so the splash was visible during daemon initialization.
        let result = app.run(&mut terminal).await;

        // Restore terminal
        disable_raw_mode()?;
        execute!(io::stdout(), PopKeyboardEnhancementFlags)?;
        execute!(io::stdout(), DisableMouseCapture)?;
        execute!(io::stdout(), LeaveAlternateScreen)?;
        // Restore default panic hook
        let _ = std::panic::take_hook();

        // Shutdown daemon and wait for it to fully stop
        let _ = shutdown_tx.send(());
        let _ = daemon_handle.await;

        result
    }

    #[cfg(not(feature = "daemon"))]
    async fn run_repl(
        &self,
        _state: crate::state::AppState,
        _prompt: Option<String>,
    ) -> anyhow::Result<()> {
        println!();
        println!("  The TUI frontend requires the daemon feature.");
        println!();
        println!("  Rebuild with:");
        println!("    cargo build --features daemon");
        println!();
        Ok(())
    }

    async fn run_query(&self, state: crate::state::AppState, prompt: String) -> anyhow::Result<()> {
        // Full agent loop (tools + micro-compaction + shared stream engine).
        // Replaces the previous one-shot non-streaming HTTP call.
        crate::cli::headless_runtime::run_oneshot(state.settings, prompt).await
    }

    fn run_config(&self, action: &super::ConfigCommands) -> anyhow::Result<()> {
        match action {
            super::ConfigCommands::Show => {
                let settings = crate::config::Settings::load()?;
                println!("{}", serde_json::to_string_pretty(&settings)?);
            }
            super::ConfigCommands::Set { key, value } => {
                crate::config::Settings::set(key, value)?;
                println!("Set {} = {}", key, value);
            }
            super::ConfigCommands::Reset => {
                crate::config::Settings::reset()?;
                println!("Configuration reset to defaults");
            }
        }
        Ok(())
    }

    async fn run_mcp(&self, action: &super::McpCommands) -> anyhow::Result<()> {
        let manager = crate::mcp::McpManager::new();
        match action {
            super::McpCommands::List => {
                let servers = manager.list_servers().await?;
                for server in servers {
                    println!("  - {} ({})", server.name, server.status);
                }
            }
            super::McpCommands::Add {
                name,
                command,
                path,
            } => {
                // 确定 command 值：优先 path，其次 command，最后空字符串
                let cmd = path
                    .as_ref()
                    .or(command.as_ref())
                    .map(|s| s.as_str())
                    .unwrap_or("");

                let mut config = crate::mcp::McpConfig::new(name, cmd);

                // 特殊处理 filesystem → 变成真正内置可用工具
                if name == "filesystem" {
                    let fs_path = path.as_deref().or(command.as_deref()).unwrap_or("");
                    config.command = fs_path.to_string();
                    config.status = crate::config::McpServerStatus::Running;
                    config.capabilities = vec![
                        "read_file".to_string(),
                        "write_file".to_string(),
                        "list_directory".to_string(),
                        "search_files".to_string(),
                        "edit_file".to_string(),
                    ];
                    config.auto_start = true;
                    config.filesystem_path = Some(std::path::PathBuf::from(fs_path));
                    println!(
                        "✅ Filesystem MCP 已作为内置工具添加（路径: {}）",
                        config.command
                    );
                }

                manager.add_server(config).await?;
                println!("Added MCP server: {}", name);
            }
            super::McpCommands::Remove { name } => {
                manager.remove_server(name).await?;
                println!("Removed MCP server: {}", name);
            }
            super::McpCommands::Restart { name } => {
                manager.restart_server(name).await?;
                println!("Restarted MCP server: {}", name);
            }
        }
        Ok(())
    }

    async fn run_plugin(&self, action: &super::PluginCommands) -> anyhow::Result<()> {
        let state = Arc::new(RwLock::new(crate::state::AppState::default()));
        let service = crate::services::PluginMarketplaceService::new(state, None);

        match action {
            super::PluginCommands::List => {
                let plugins = service.list_installed().await;
                if plugins.is_empty() {
                    println!("No plugins installed");
                } else {
                    for plugin in plugins {
                        let status = if plugin.enabled {
                            "enabled"
                        } else {
                            "disabled"
                        };
                        println!("  - {} v{} [{}]", plugin.name, plugin.version, status);
                    }
                }
            }
            super::PluginCommands::Install { plugin } => {
                let installed = service.install(plugin).await?;
                println!("Installed: {} v{}", installed.name, installed.version);
            }
            super::PluginCommands::Remove { name } => {
                service.remove(name).await?;
                println!("Removed plugin: {}", name);
            }
            super::PluginCommands::Update => {
                let updated = service.update_all().await?;
                println!("Updated {} plugins", updated.len());
            }
            super::PluginCommands::Search { query } => {
                let results = service.search(query).await;
                if results.is_empty() {
                    println!("No plugins found for: {}", query);
                } else {
                    for plugin in results {
                        println!(
                            "  - {} v{} by {} (⭐ {})",
                            plugin.name, plugin.version, plugin.author, plugin.rating
                        );
                        println!("    {}", plugin.description);
                    }
                }
            }
            super::PluginCommands::Enable { name } => {
                service.enable(name).await?;
                println!("Enabled plugin: {}", name);
            }
            super::PluginCommands::Disable { name } => {
                service.disable(name).await?;
                println!("Disabled plugin: {}", name);
            }
        }
        Ok(())
    }

    async fn run_memory(&self, action: &super::MemoryCommands) -> anyhow::Result<()> {
        let manager = crate::context::MemoryManager::new();
        manager.load().await?;

        match action {
            super::MemoryCommands::Status => {
                let status = manager.status().await?;
                println!("Memory Status:");
                println!("  Sessions: {}", status.session_count);
                println!("  Memories: {}", status.total_memories);
                println!("  Last Consolidation: {:?}", status.last_consolidation);
            }
            super::MemoryCommands::Clear => {
                manager.clear().await?;
                println!("All memories cleared");
            }
            super::MemoryCommands::Export { output } => {
                manager.export(output).await?;
                println!("Memories exported to: {}", output.display());
            }
            super::MemoryCommands::Import { input } => {
                manager.import(input).await?;
                println!("Memories imported from: {}", input.display());
            }
            super::MemoryCommands::Dream => {
                println!("Running memory consolidation (dream)...");
                manager.consolidate().await?;
                println!("Memory consolidation completed");
            }
            super::MemoryCommands::AutoDream => {
                let state = Arc::new(RwLock::new(crate::state::AppState::default()));
                let service =
                    crate::services::AutoDreamService::new(state, None, Some(Arc::new(manager)));
                println!("Forcing AutoDream consolidation...");
                service.force_consolidation().await?;
                println!("AutoDream consolidation completed");
            }
        }
        Ok(())
    }

    async fn run_voice(
        &self,
        state: crate::state::AppState,
        push_to_talk: bool,
    ) -> anyhow::Result<()> {
        let state = Arc::new(RwLock::new(state));
        let service = crate::voice::VoiceService::new(state, None);

        let status = service.get_status().await;
        if !status.available {
            println!("Voice input is not available on this system");
            println!("Backend: {:?}", status.backend);
            return Ok(());
        }

        if push_to_talk {
            println!("🎤 Push-to-talk mode enabled");
            println!("Press Enter to start recording, press Enter again to stop.");

            service.push_to_talk_start().await?;

            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;

            let text = service.push_to_talk_stop().await?;
            println!("\n📝 Transcribed: {}", text);
        } else {
            println!("🎤 Continuous voice input mode");
            println!("Voice input starting...");
            service.start_recording().await?;
        }

        Ok(())
    }

    fn run_init(&self, name: Option<String>) -> anyhow::Result<()> {
        let project_name = name.unwrap_or_else(|| "wgenty-code-project".to_string());
        crate::utils::project::init_project(&project_name)?;
        println!("Initialized project: {}", project_name);
        Ok(())
    }

    fn run_update(&self) -> anyhow::Result<()> {
        println!("Checking for updates...");
        println!("Already at latest version");
        Ok(())
    }

    fn run_help(&self, topic: Option<String>) -> anyhow::Result<()> {
        match topic {
            Some(t) => println!("Help for topic: {}", t),
            None => println!("Use --help for detailed usage information"),
        }
        Ok(())
    }

    async fn run_services(
        &self,
        state: crate::state::AppState,
        action: &super::ServiceCommands,
    ) -> anyhow::Result<()> {
        let state = Arc::new(RwLock::new(state));
        let mut manager = crate::services::ServiceManager::new(state.clone());
        manager.initialize().await?;

        match action {
            super::ServiceCommands::Status => {
                let status = manager.get_status().await;
                println!("Service Status:");
                println!("{}", serde_json::to_string_pretty(&status)?);
            }
            super::ServiceCommands::Start => {
                manager.start_all().await?;
            }
            super::ServiceCommands::Stop => {
                manager.stop_all().await?;
            }
            super::ServiceCommands::AutoDream => {
                if let Some(auto_dream) = manager.auto_dream() {
                    let status = auto_dream.get_status().await;
                    println!("AutoDream Status:");
                    println!("  Enabled: {}", status.enabled);
                    println!("  Consolidating: {}", status.is_consolidating);
                    println!("  Last consolidation: {}h ago", status.hours_since_last);
                    println!("  Sessions accumulated: {}", status.sessions_accumulated);
                    println!("  Next consolidation in: {}h", status.next_consolidation_in);
                }
            }
            super::ServiceCommands::Voice => {
                if let Some(voice) = manager.voice() {
                    let status = voice.get_status().await;
                    println!("Voice Status:");
                    println!("  Available: {}", status.available);
                    println!("  Backend: {:?}", status.backend);
                    println!("  State: {:?}", status.state);
                }
            }
            super::ServiceCommands::MagicDocs => {
                if let Some(magic_docs) = manager.magic_docs() {
                    let status = magic_docs.get_status().await;
                    println!("Magic Docs Status:");
                    println!("  Enabled: {}", status.enabled);
                    println!("  Auto-update: {}", status.auto_update);
                    println!("  Tracked docs: {}", status.tracked_count);
                }
            }
            super::ServiceCommands::TeamSync => {
                if let Some(team_sync) = manager.team_memory_sync() {
                    let status = team_sync.get_status().await;
                    println!("Team Sync Status:");
                    println!("  Enabled: {}", status.enabled);
                    println!("  Authenticated: {}", status.is_authenticated);
                    println!("  Local memories: {}", status.local_memories);
                    println!("  Remote memories: {}", status.remote_memories);
                }
            }
            super::ServiceCommands::Plugins => {
                if let Some(plugins) = manager.plugin_marketplace() {
                    let status = plugins.get_status().await;
                    println!("Plugins Status:");
                    println!("  Enabled: {}", status.enabled);
                    println!("  Installed: {}", status.installed_count);
                    println!("  Updates available: {}", status.updates_available);
                }
            }
            super::ServiceCommands::Agents => {
                if let Some(agents) = manager.agents() {
                    let status = agents.get_status().await;
                    println!("Agents Status:");
                    println!("  Available agents: {}", status.available_agents.len());
                    println!("  Active sessions: {}", status.active_sessions);
                    for agent in &status.available_agents {
                        println!("    - {} ({})", agent.name, agent.agent_type);
                    }
                }
            }
        }
        Ok(())
    }

    async fn run_agent(
        &self,
        state: crate::state::AppState,
        agent_type: &str,
        prompt: &str,
    ) -> anyhow::Result<()> {
        let state = Arc::new(RwLock::new(state));
        let service = crate::teams::AgentsService::new(state);

        let agent_type = match agent_type.to_lowercase().as_str() {
            "guide" | "wgenty-code-guide" => crate::teams::AgentType::WgentyCodeGuide,
            "explore" => crate::teams::AgentType::Explore,
            "plan" => crate::teams::AgentType::Plan,
            "verify" | "verification" => crate::teams::AgentType::Verification,
            "general" | "general-purpose" => crate::teams::AgentType::GeneralPurpose,
            _ => {
                println!("Unknown agent type: {}", agent_type);
                println!("Available types: guide, explore, plan, verify, general");
                return Ok(());
            }
        };

        println!("🤖 Running {} agent...", agent_type);
        println!("Prompt: {}", prompt);
        println!();

        let session = service.run_agent(&agent_type, prompt).await?;

        if let Some(result) = &session.result {
            println!("{}", result);
        }

        Ok(())
    }

    async fn run_magic_docs(
        &self,
        state: crate::state::AppState,
        action: &super::MagicDocsCommands,
    ) -> anyhow::Result<()> {
        let state = Arc::new(RwLock::new(state));
        let service = crate::knowledge::MagicDocsService::new(state, None);

        match action {
            super::MagicDocsCommands::List => {
                let docs = service.get_tracked_docs().await;
                if docs.is_empty() {
                    println!("No Magic Docs tracked");
                } else {
                    for doc in docs {
                        println!("  - {} ({})", doc.title, doc.path);
                        println!(
                            "    Updated: {} ({} times)",
                            doc.last_updated, doc.update_count
                        );
                    }
                }
            }
            super::MagicDocsCommands::Check { file } => {
                if let Some(header) = service.check_file(file).await {
                    println!("Magic Doc detected:");
                    println!("  Title: {}", header.title);
                    if let Some(instructions) = header.instructions {
                        println!("  Instructions: {}", instructions);
                    }
                } else {
                    println!("Not a Magic Doc: {}", file);
                }
            }
            super::MagicDocsCommands::Update { file, context } => {
                let ctx = context
                    .clone()
                    .unwrap_or_else(|| "Manual update".to_string());
                service.update_magic_doc(file, &ctx).await?;
                println!("Updated Magic Doc: {}", file);
            }
            super::MagicDocsCommands::Clear => {
                service.clear_all().await;
                println!("All Magic Docs cleared");
            }
        }
        Ok(())
    }

    async fn run_team_sync(
        &self,
        state: crate::state::AppState,
        action: &super::TeamSyncCommands,
    ) -> anyhow::Result<()> {
        use crate::services::{ConflictResolution, TeamMemoryConfig, TeamMemorySyncService};

        let state = Arc::new(RwLock::new(state));
        let service = TeamMemorySyncService::new(
            state,
            Some(TeamMemoryConfig {
                enabled: true,
                team_id: Some("test-team".to_string()),
                sync_interval_secs: 3600,
                auto_sync: false,
                conflict_resolution: ConflictResolution::PreferNewer,
            }),
        );

        match action {
            super::TeamSyncCommands::Status => {
                let status = service.get_status().await;
                println!("Team Sync Status:");
                println!("  Enabled: {}", status.enabled);
                println!("  Team ID: {:?}", status.team_id);
                println!("  Authenticated: {}", status.is_authenticated);
                println!("  Local memories: {}", status.local_memories);
                println!("  Remote memories: {}", status.remote_memories);
                if let Some(last) = status.sync_status.last_sync {
                    println!("  Last sync: {}", last);
                }
            }
            super::TeamSyncCommands::Sync => {
                println!("Starting sync...");
                let result = service.sync().await?;
                println!("Sync completed:");
                println!("  Uploaded: {}", result.uploaded);
                println!("  Downloaded: {}", result.downloaded);
                println!("  Conflicts: {}", result.conflicts);
                if !result.errors.is_empty() {
                    println!("  Errors: {:?}", result.errors);
                }
            }
            super::TeamSyncCommands::Auth { team_id } => {
                println!("Authenticating with team: {}", team_id);
                if service.authenticate(team_id).await.is_ok() {
                    println!("✅ Authentication successful");
                } else {
                    println!("❌ Authentication failed");
                }
            }
            super::TeamSyncCommands::Create { title, content, .. } => {
                let memory = service.create_memory(title, content, vec![]).await;
                if memory.is_ok() {
                    println!("✅ Memory created: {}", title);
                } else {
                    println!("❌ Failed to create memory");
                }
            }
            super::TeamSyncCommands::List => {
                let memories = service.list_memories().await;
                if memories.is_empty() {
                    println!("No local memories");
                } else {
                    for memory in memories {
                        println!("  - {} ({})", memory.title, memory.author);
                    }
                }
            }
            super::TeamSyncCommands::Delete { .. } => {
                println!("Delete not implemented");
            }
        }
        Ok(())
    }

    async fn run_stress_test(&self, concurrency: usize, iterations: usize) -> anyhow::Result<()> {
        use crate::utils::stress_tests::run_stress_test;
        run_stress_test(concurrency, iterations).await;
        Ok(())
    }

    async fn run_skills(&self, action: &super::SkillsCommands) -> anyhow::Result<()> {
        match action {
            super::SkillsCommands::List => {
                let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
                let project_root =
                    std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));

                // load_from_dirs appends "skills" to each base dir,
                // so pass parent directories (resolver returns .../skills paths).
                let roots = crate::knowledge::SkillRootResolver::roots_with(&home, &project_root);
                let skills_dirs: Vec<std::path::PathBuf> = roots
                    .iter()
                    .map(|r| {
                        r.skills_root
                            .parent()
                            .unwrap_or(&r.skills_root)
                            .to_path_buf()
                    })
                    .collect();

                let skill_loader =
                    crate::knowledge::loader::SkillLoader::load_from_dirs(&skills_dirs);

                let names = skill_loader.skill_names();
                if names.is_empty() {
                    println!("No skills found.");
                    println!(
                        "Place SKILL.md files under ~/.wgenty-code/skills/<name>/ or project/.wgenty-code/skills/<name>/"
                    );
                } else {
                    println!("Available skills:");
                    for name in &names {
                        if let Some(skill) = skill_loader.load_skill(name) {
                            let desc = if skill.description.is_empty() {
                                "(no description)"
                            } else {
                                &skill.description
                            };
                            println!("  - {}: {}", name, desc);
                        } else {
                            println!("  - {}", name);
                        }
                    }
                }
            }
            super::SkillsCommands::Execute { skill, args } => {
                println!("Executing skill: {} with args: {:?}", skill, args);
            }
            super::SkillsCommands::Help { skill } => {
                println!("Help for skill: {}", skill);
            }
            super::SkillsCommands::Search { query } => {
                println!("Searching skills for: {}", query);
            }
            super::SkillsCommands::Install => {
                use crate::knowledge::embedded;

                let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
                let base = home.join(".wgenty-code");

                #[cfg(feature = "bundled-skills")]
                let bundled_count = embedded::BundledSkills::count();
                #[cfg(not(feature = "bundled-skills"))]
                let bundled_count: usize = 0;

                if bundled_count == 0 {
                    println!("No bundled skills available.");
                    println!("This binary was built without the 'bundled-skills' feature.");
                    return Ok(());
                }

                println!(
                    "Installing {} bundled skill(s) to {} ...",
                    bundled_count,
                    base.join("skills").display()
                );

                match embedded::BundledSkills::install_to(&base) {
                    Ok(installed) => {
                        if installed.is_empty() {
                            println!("All bundled skills are already installed (no overwrites).");
                        } else {
                            println!("Installed {} skill(s):", installed.len());
                            // Show name + description for each newly installed skill
                            let all = embedded::BundledSkills::list_bundled();
                            for name in &installed {
                                let desc = all
                                    .iter()
                                    .find(|(n, _)| n == name)
                                    .map(|(_, d)| d.as_str())
                                    .unwrap_or("");
                                println!("  - {}: {}", name, desc);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to install bundled skills: {}", e);
                        return Err(anyhow::anyhow!("install failed: {}", e));
                    }
                }
            }
        }
        Ok(())
    }

    async fn run_sandbox(&self, action: &super::SandboxCommands) -> anyhow::Result<()> {
        let sandbox = crate::sandbox::SandboxManager::new();
        let status = sandbox.status();
        match action {
            super::SandboxCommands::Status => {
                println!("Sandbox Status:");
                println!("  Backend: {}", status.backend_name);
                println!("  Hardware-enforced: {}", status.is_hardware_enforced);
                println!("  Capabilities: {:?}", status.capabilities);
            }
            super::SandboxCommands::Disable => {
                println!("Sandbox disabled for this session.");
            }
            super::SandboxCommands::Enable => {
                if status.is_hardware_enforced {
                    println!("Sandbox enabled ({}).", status.backend_name);
                } else {
                    println!("Sandbox enabled (policy-only, {}).", status.backend_name);
                }
            }
        }
        Ok(())
    }
}
