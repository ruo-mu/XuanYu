use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;
use xuanyu_extensions::{load_extensions, ExtensionLoadConfig};

#[test]
fn high_risk_plugins_are_blocked_until_trusted() {
    let project = temp_project("high_risk_plugins_are_blocked_until_trusted");
    let plugin_dir = project.join(".xuanyu").join("plugins").join("risky");
    fs::create_dir_all(&plugin_dir).expect("create plugin dir");
    fs::write(
        plugin_dir.join("plugin.json"),
        r#"{
  "id": "risky",
  "name": "Risky",
  "version": "1.0.0",
  "permissions": {
    "fs": { "read": true, "write": ["src/generated.ts"] },
    "network": true,
    "process": true,
    "tools": ["file_write", "mcp__github"],
    "env": ["*"]
  },
  "contributes": {
    "tools": ["risky.tool"],
    "skills": [{ "id": "risky-skill", "name": "Risky Skill" }]
  }
}"#,
    )
    .expect("write plugin");

    let blocked = load_extensions(&ExtensionLoadConfig::from_app_config(
        project.clone(),
        &json!({ "plugins": { "directories": [".xuanyu/plugins"] } }),
    ));
    assert!(!blocked.plugins[0].enabled);
    assert_eq!(blocked.plugins[0].isolation.mode, "blocked");
    assert!(blocked.plugins[0].isolation.blocked);
    assert!(blocked.plugins[0]
        .isolation
        .denied_capabilities
        .contains(&"process".to_string()));
    assert!(blocked.plugins[0]
        .isolation
        .denied_capabilities
        .contains(&"network:*".to_string()));
    assert!(!blocked.skills.iter().any(|skill| skill.id == "risky-skill"));
    assert_eq!(blocked.totals.enabled_plugins, 0);
    assert_eq!(blocked.totals.tools, 0);

    let trusted = load_extensions(&ExtensionLoadConfig::from_app_config(
        project.clone(),
        &json!({
            "plugins": {
                "directories": [".xuanyu/plugins"],
                "trustedIds": ["risky"]
            }
        }),
    ));
    assert!(trusted.plugins[0].enabled);
    assert_eq!(trusted.plugins[0].isolation.mode, "trusted");
    assert!(trusted.skills.iter().any(|skill| skill.id == "risky-skill"));
    assert_eq!(trusted.totals.enabled_plugins, 1);
    assert_eq!(trusted.totals.tools, 1);

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
