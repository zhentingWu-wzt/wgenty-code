//! Services Module - Background services for Claude Code
//!
//! This module provides various background services including:
//! - AutoDream: Automatic memory consolidation
//! - Voice: Voice input and transcription
//! - MagicDocs: Automatic documentation maintenance
//! - TeamMemorySync: Team memory synchronization
//! - PluginMarketplace: Plugin management
//! - Agents: Built-in agent system

use crate::state::AppState;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

pub mod agents;
pub mod auto_dream;
pub mod magic_docs;
pub mod plugin_marketplace;
pub mod stress_tests;
pub mod team_memory_sync;
pub mod voice;

pub use agents::{AgentDefinition, AgentSession, AgentStatus, AgentType, AgentsService};
pub use auto_dream::{AutoDreamConfig, AutoDreamService, AutoDreamStatus};
pub use magic_docs::{MagicDocHeader, MagicDocInfo, MagicDocsConfig, MagicDocsService};
pub use plugin_marketplace::{MarketplacePlugin, Plugin, PluginConfig, PluginMarketplaceService};
pub use stress_tests::{run_stress_test, StressTestResult, StressTestRunner};
pub use team_memory_sync::{
    ConflictResolution, TeamMemory, TeamMemoryConfig, TeamMemorySyncService, TeamMemorySyncStatus,
};
pub use voice::{RecordingState, VoiceBackend, VoiceConfig, VoiceService, VoiceStatus};

/// Background service manager
pub struct ServiceManager {
    state: Arc<RwLock<AppState>>,
    auto_dream: Option<Arc<AutoDreamService>>,
    voice: Option<Arc<VoiceService>>,
    magic_docs: Option<Arc<MagicDocsService>>,
    team_memory_sync: Option<Arc<TeamMemorySyncService>>,
    plugin_marketplace: Option<Arc<PluginMarketplaceService>>,
    agents: Option<Arc<AgentsService>>,
}

impl ServiceManager {
    /// Create a new service manager
    pub fn new(state: Arc<RwLock<AppState>>) -> Self {
        Self {
            state,
            auto_dream: None,
            voice: None,
            magic_docs: None,
            team_memory_sync: None,
            plugin_marketplace: None,
            agents: None,
        }
    }

    /// Initialize all services
    pub async fn initialize(&mut self) -> anyhow::Result<()> {
        info!("initializing background services");

        self.auto_dream = Some(Arc::new(AutoDreamService::new(self.state.clone(), None)));
        self.voice = Some(Arc::new(VoiceService::new(self.state.clone(), None)));
        self.magic_docs = Some(Arc::new(MagicDocsService::new(self.state.clone(), None)));
        self.team_memory_sync = Some(Arc::new(TeamMemorySyncService::new(
            self.state.clone(),
            None,
        )));
        self.plugin_marketplace = Some(Arc::new(PluginMarketplaceService::new(
            self.state.clone(),
            None,
        )));
        self.agents = Some(Arc::new(AgentsService::new(self.state.clone())));

        if let Some(magic_docs) = &self.magic_docs {
            magic_docs.load_state().await?;
        }

        info!("background services initialized");
        Ok(())
    }

    /// Start all background services
    pub async fn start_all(&self) -> anyhow::Result<()> {
        info!("starting background services");

        if let Some(auto_dream) = &self.auto_dream {
            let status = auto_dream.get_status().await;
            info!(
                enabled = status.enabled,
                hours_since_last = status.hours_since_last,
                "autodream service status"
            );
        }

        if let Some(voice) = &self.voice {
            let status = voice.get_status().await;
            info!(
                available = status.available,
                backend = ?status.backend,
                "voice service status"
            );
        }

        if let Some(magic_docs) = &self.magic_docs {
            let status = magic_docs.get_status().await;
            info!(
                tracked_docs = status.tracked_count,
                "magic docs service status"
            );
        }

        if let Some(team_sync) = &self.team_memory_sync {
            let status = team_sync.get_status().await;
            info!(
                local_memories = status.local_memories,
                remote_memories = status.remote_memories,
                authenticated = status.is_authenticated,
                "team sync service status"
            );
        }

        if let Some(plugins) = &self.plugin_marketplace {
            let status = plugins.get_status().await;
            info!(
                installed_plugins = status.installed_count,
                "plugin marketplace status"
            );
        }

        if let Some(agents) = &self.agents {
            let status = agents.get_status().await;
            info!(
                available_agents = status.available_agents.len(),
                active_sessions = status.active_sessions,
                "agents service status"
            );
        }

        info!("background services started");
        Ok(())
    }

    /// Stop all background services
    pub async fn stop_all(&self) -> anyhow::Result<()> {
        info!("stopping background services");

        if let Some(magic_docs) = &self.magic_docs {
            magic_docs.save_state().await?;
        }

        info!("background services stopped");
        Ok(())
    }

    pub fn auto_dream(&self) -> Option<Arc<AutoDreamService>> {
        self.auto_dream.clone()
    }

    pub fn voice(&self) -> Option<Arc<VoiceService>> {
        self.voice.clone()
    }

    pub fn magic_docs(&self) -> Option<Arc<MagicDocsService>> {
        self.magic_docs.clone()
    }

    pub fn team_memory_sync(&self) -> Option<Arc<TeamMemorySyncService>> {
        self.team_memory_sync.clone()
    }

    pub fn plugin_marketplace(&self) -> Option<Arc<PluginMarketplaceService>> {
        self.plugin_marketplace.clone()
    }

    pub fn agents(&self) -> Option<Arc<AgentsService>> {
        self.agents.clone()
    }

    pub async fn get_status(&self) -> ServiceStatus {
        ServiceStatus {
            auto_dream: self
                .auto_dream
                .as_ref()
                .map(|s| futures::executor::block_on(s.get_status())),
            voice: self
                .voice
                .as_ref()
                .map(|s| futures::executor::block_on(s.get_status())),
            magic_docs: self
                .magic_docs
                .as_ref()
                .map(|s| futures::executor::block_on(s.get_status())),
            team_sync: self
                .team_memory_sync
                .as_ref()
                .map(|s| futures::executor::block_on(s.get_status())),
            plugins: self
                .plugin_marketplace
                .as_ref()
                .map(|s| futures::executor::block_on(s.get_status())),
            agents: self
                .agents
                .as_ref()
                .map(|s| futures::executor::block_on(s.get_status())),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ServiceStatus {
    pub auto_dream: Option<AutoDreamStatus>,
    pub voice: Option<VoiceStatus>,
    pub magic_docs: Option<magic_docs::MagicDocsStatus>,
    pub team_sync: Option<TeamMemorySyncStatus>,
    pub plugins: Option<plugin_marketplace::PluginStatus>,
    pub agents: Option<agents::AgentStatusReport>,
}
