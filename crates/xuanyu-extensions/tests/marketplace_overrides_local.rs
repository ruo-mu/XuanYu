use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;
use xuanyu_extensions::{load_extensions, ExtensionLoadConfig};
use xuanyu_storage::project_data_root;

#[test]
fn marketplace_can_override_local_manifest_like_legacy_loader() {
    let project = temp_project("marketplace_can_override_local_manifest_like_legacy_loader");
    std::env::set_var("XUANYU_DATA_DIR", &project);
    let plugin_dir = project_data_root(&project).join("plugins").join("demo");
    fs::create_dir_all(&plugin_dir).expect("create plugin dir");
    fs::write(
        plugin_dir.join("plugin.json"),
        r#"{ "id": "same", "name": "Local Same", "version": "1.0.0" }"#,
    )
    .expect("write local plugin");
    fs::write(
        project.join("marketplace.json"),
        r#"{ "plugins": [{ "id": "same", "name": "Marketplace Same", "version": "2.0.0", "url": "https://example.com/plugin.zip" }] }"#,
    )
    .expect("write marketplace");

    let config = ExtensionLoadConfig::from_app_config(
        project.clone(),
        &json!({ "plugins": { "marketplacePath": "marketplace.json" } }),
    );
    let payload = load_extensions(&config);

    assert_eq!(payload.plugins.len(), 1);
    assert_eq!(payload.plugins[0].name, "Marketplace Same");
    assert_eq!(payload.plugins[0].source, "marketplace");
    assert_eq!(
        payload.plugins[0].marketplace_url.as_deref(),
        Some("https://example.com/plugin.zip")
    );

    let _ = fs::remove_dir_all(project);
}

fn temp_project(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("xuanyu_extensions_{name}_{nonce}"));
    fs::create_dir_all(&path).expect("create temp project");
    path
}
