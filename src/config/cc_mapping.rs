//! CC config key mapping — maps CC-standard settings keys to internal fields.
//!
//! Applied after Settings::load(), this ensures backward compatibility
//! while accepting Claude Code-compatible configuration keys.

use super::Settings;

/// Applies CC config key mappings to Settings.
///
/// Priority rules:
/// - `enabledPlugins` values override `plugins.enabled_map` entries with the same key
/// - `pluginMarketplaces` is merged into the existing marketplace configuration
/// - CC keys take priority over wgenty-code native keys
pub struct CcConfigMapper;

impl CcConfigMapper {
    /// Apply all CC config key mappings to the given settings.
    pub fn apply_mappings(settings: &mut Settings) {
        // 1. enabledPlugins → plugins.enabled_map (CC keys win on conflict)
        if let Some(ref cc_enabled) = settings.enabled_plugins {
            for (key, val) in cc_enabled {
                settings.plugins.enabled_map.insert(key.clone(), *val);
            }
        }
        // 2. pluginMarketplaces are stored in settings and resolved by
        //    PluginMarketplaceService at runtime — no further mapping needed here.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_enabled_plugins_mapping() {
        let mut settings = Settings::default();
        let mut cc_enabled = std::collections::HashMap::new();
        cc_enabled.insert("superpowers@claude-plugins-official".to_string(), true);

        settings.enabled_plugins = Some(cc_enabled);
        CcConfigMapper::apply_mappings(&mut settings);

        assert!(
            settings
                .plugins
                .enabled_map
                .get("superpowers@claude-plugins-official")
                == Some(&true)
        );
    }

    #[test]
    fn test_enabled_plugins_cc_wins_on_conflict() {
        let mut settings = Settings::default();
        // Set native value first
        settings
            .plugins
            .enabled_map
            .insert("plugin@pub".to_string(), false);

        let mut cc_enabled = std::collections::HashMap::new();
        cc_enabled.insert("plugin@pub".to_string(), true);
        settings.enabled_plugins = Some(cc_enabled);

        CcConfigMapper::apply_mappings(&mut settings);

        // CC value should win
        assert_eq!(settings.plugins.enabled_map.get("plugin@pub"), Some(&true));
    }

    #[test]
    fn test_no_cc_keys_preserves_existing() {
        let mut settings = Settings::default();
        settings
            .plugins
            .enabled_map
            .insert("legacy".to_string(), true);

        // No CC keys set
        CcConfigMapper::apply_mappings(&mut settings);

        assert_eq!(settings.plugins.enabled_map.get("legacy"), Some(&true));
    }
}
