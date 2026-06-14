use wgenty_code::plugins::package_json::PackageJsonManifest;

#[test]
fn test_parse_basic_package_json() {
    let json = r#"{
        "name": "my-plugin",
        "version": "1.0.0",
        "description": "A test plugin",
        "author": "Test Author",
        "main": "index.js"
    }"#;

    let manifest: PackageJsonManifest = serde_json::from_str(json).unwrap();
    assert_eq!(manifest.name, "my-plugin");
    assert_eq!(manifest.version, "1.0.0");
    assert_eq!(manifest.description.as_deref(), Some("A test plugin"));
    assert_eq!(manifest.main.as_deref(), Some("index.js"));
}

#[test]
fn test_parse_scoped_package_json() {
    let json = r#"{
        "name": "@anthropic/superpowers",
        "version": "5.1.0",
        "description": "Superpowers plugin",
        "author": {"name": "Anthropic", "email": "support@anthropic.com"},
        "main": "index.js"
    }"#;

    let manifest: PackageJsonManifest = serde_json::from_str(json).unwrap();
    assert_eq!(manifest.name, "@anthropic/superpowers");
}

#[test]
fn test_parse_minimal_package_json() {
    let json = r#"{
        "name": "minimal",
        "version": "0.1.0"
    }"#;

    let manifest: PackageJsonManifest = serde_json::from_str(json).unwrap();
    assert_eq!(manifest.name, "minimal");
    assert_eq!(manifest.version, "0.1.0");
    assert_eq!(manifest.description, None);
    assert_eq!(manifest.main, None);
}
