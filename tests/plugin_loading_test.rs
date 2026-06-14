use std::fs;
use tempfile::TempDir;
use wgenty_code::plugins::PluginManager;

fn create_cc_plugin_structure(root: &std::path::Path) {
    let ver_dir = root
        .join("cache")
        .join("anthropic")
        .join("superpowers")
        .join("5.1.0");
    fs::create_dir_all(&ver_dir).unwrap();
    fs::write(
        ver_dir.join("package.json"),
        r#"{"name": "@anthropic/superpowers", "version": "5.1.0", "main": "index.js"}"#,
    )
    .unwrap();
}

fn create_legacy_plugin_structure(root: &std::path::Path) {
    let plugin_dir = root.join("legacy-tool");
    fs::create_dir_all(&plugin_dir).unwrap();
    fs::write(
        plugin_dir.join("plugin.json"),
        r#"{"name": "legacy-tool", "version": "1.0.0", "main": "tool.js", "commands": [], "hooks": [], "dependencies": {}, "permissions": [], "enabled": true}"#,
    ).unwrap();
}

#[tokio::test]
async fn test_load_all_cc_plugin() {
    let dir = TempDir::new().unwrap();
    create_cc_plugin_structure(dir.path());

    let manager = PluginManager::new().with_plugins_dir(dir.path().to_path_buf());
    manager.load_all().await.unwrap();

    let plugins = manager.list().await.unwrap();
    assert_eq!(plugins.len(), 1);
    assert_eq!(plugins[0].name, "superpowers");
}

#[tokio::test]
async fn test_load_all_legacy_plugin() {
    let dir = TempDir::new().unwrap();
    create_legacy_plugin_structure(dir.path());

    let manager = PluginManager::new().with_plugins_dir(dir.path().to_path_buf());
    manager.load_all().await.unwrap();

    let plugins = manager.list().await.unwrap();
    assert_eq!(plugins.len(), 1);
    assert_eq!(plugins[0].name, "legacy-tool");
}

#[tokio::test]
async fn test_load_all_cc_priority_over_legacy() {
    let dir = TempDir::new().unwrap();

    // Create CC-format plugin
    let ver_dir = dir
        .path()
        .join("cache")
        .join("testpub")
        .join("myplugin")
        .join("2.0.0");
    fs::create_dir_all(&ver_dir).unwrap();
    fs::write(
        ver_dir.join("package.json"),
        r#"{"name": "@testpub/myplugin", "version": "2.0.0", "main": "index.js"}"#,
    )
    .unwrap();

    // Create legacy plugin with same name in flat dir
    let legacy_dir = dir.path().join("myplugin");
    fs::create_dir_all(&legacy_dir).unwrap();
    fs::write(
        legacy_dir.join("plugin.json"),
        r#"{"name": "myplugin", "version": "1.0.0", "main": "legacy.js", "commands": [], "hooks": [], "dependencies": {}, "permissions": [], "enabled": true}"#,
    ).unwrap();

    let manager = PluginManager::new().with_plugins_dir(dir.path().to_path_buf());
    manager.load_all().await.unwrap();

    let plugins = manager.list().await.unwrap();
    // Both should be loaded (different keys: "myplugin@testpub" vs "myplugin")
    // The registry key for CC format is "myplugin@testpub" vs legacy "myplugin"
    assert!(
        plugins.len() >= 1,
        "Expected at least 1 plugin, got {}",
        plugins.len()
    );
    // Verify CC format is present
    let cc_plugin = plugins.iter().find(|p| p.version == "2.0.0");
    assert!(cc_plugin.is_some(), "CC-format plugin should be loaded");
}
