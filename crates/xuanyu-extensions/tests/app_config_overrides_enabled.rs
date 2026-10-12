use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;
use xuanyu_extensions::{load_extensions, ExtensionLoadConfig};
use xuanyu_storage::project_data_root;

#[test]
fn app_config_can_override_plugin_enabled_state() {
    let project = temp_project("app_config_can_override_plugin_enabled_state");
    std::env::set_var("XUANYU_DATA_DIR", &project);
    let plugin_dir = project_data_root(&project).join("plugins").join("demo");
    fs::create_dir_all(&plugin_dir).expect("create plugin dir");
    fs::write(
        plugin_dir.join("plugin.json"),
        r#"{
  "id": "demo",
  "name": "Demo",
  "version": "1.0.0",
  "contributes": { "skills": [{ "id": "demo-skill", "name": "Demo Skill" }] }
}"#,
    )
    .expect("write plugin");

    let config = ExtensionLoadConfig::from_app_config(
        project.clone(),
        &json!({ "plugins": { "enabledById": { "demo": false } } }),
    );
    let payload = load_extensions(&config);

    assert_eq!(payload.plugins[0].id, "demo");
    assert!(!payload.plugins[0].enabled);
    assert_eq!(payload.totals.enabled_plugins, 0);
    assert!(!payload.skills.iter().any(|skill| skill.id == "demo-skill"));

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
