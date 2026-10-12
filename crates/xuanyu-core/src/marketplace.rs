use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{LazyLock, RwLock},
    time::{SystemTime, UNIX_EPOCH},
};
use xuanyu_storage::project_data_root;
use xuanyu_extensions::{
    load_extensions, load_marketplace, ExtensionLoadConfig, PluginManifest, SkillDef,
};

static MARKETPLACE_PLANS: LazyLock<RwLock<BTreeMap<String, Value>>> =
    LazyLock::new(|| RwLock::new(BTreeMap::new()));
static MARKETPLACE_OPERATIONS: LazyLock<RwLock<Vec<Value>>> =
    LazyLock::new(|| RwLock::new(Vec::new()));

pub struct MarketplaceSearchParams<'a> {
    pub project_dir: PathBuf,
    pub config: &'a Value,
    pub query: Option<&'a str>,
    pub kind: Option<&'a str>,
    pub kinds: Option<Vec<String>>,
    pub request_id: Option<String>,
    pub page: Option<usize>,
    pub page_size: Option<usize>,
    pub category: Option<&'a str>,
    pub source_id: Option<&'a str>,
    pub summary_only: bool,
}

pub async fn marketplace_search_async(input: MarketplaceSearchParams<'_>) -> Value {
    let MarketplaceSearchParams {
        project_dir,
        config,
        query,
        kind,
        kinds,
        request_id,
        page,
        page_size,
        category,
        source_id,
        summary_only,
    } = input;
    let load_config = ensure_marketplace_path(
        ExtensionLoadConfig::from_app_config(project_dir.clone(), config),
        &project_dir,
    );
    let payload = load_extensions(&load_config);
    let marketplace_manifests = load_config
        .marketplace_path
        .as_ref()
        .map(|path| load_marketplace(path))
        .unwrap_or_default();

    let mut entries = Vec::new();
    let mut seen = BTreeSet::new();

    // Prefer raw marketplace.json entries (preserve install specs).
    for path in marketplace_catalog_candidates(&project_dir, load_config.marketplace_path.as_deref())
    {
        for entry in load_raw_marketplace_entries(&path) {
            push_market_entry(&mut entries, &mut seen, entry);
        }
    }
    // Embedded fallback catalog shipped with the engine binary.
    for entry in load_bundled_marketplace_entries() {
        push_market_entry(&mut entries, &mut seen, entry);
    }

    for plugin in payload.plugins.iter().chain(marketplace_manifests.iter()) {
        push_market_entry(&mut entries, &mut seen, plugin_entry(plugin, "plugin"));
    }
    for skill in &payload.skills {
        push_market_entry(&mut entries, &mut seen, skill_entry(skill));
    }

    // Channel adapters are built into xuanyu-channels; never listed as installable plugins.
    entries.retain(|entry| !is_channel_market_entry(entry));

    let filter = crate::marketplace_remote::RemoteKindFilter::from_kinds(
        kind,
        kinds.as_deref(),
    );
    let remote = crate::marketplace_remote::fetch_remote_marketplace(filter).await;
    for entry in remote.entries {
        push_market_entry(&mut entries, &mut seen, entry);
    }

    let filter_kinds = normalize_kinds(kind, kinds);
    if !filter_kinds.is_empty() {
        entries.retain(|entry| {
            entry
                .get("kind")
                .and_then(Value::as_str)
                .is_some_and(|value| filter_kinds.iter().any(|kind| kind == value))
        });
    }

    if let Some(category) = category.map(str::trim).filter(|value| !value.is_empty() && *value != "all")
    {
        entries.retain(|entry| {
            entry
                .get("category")
                .and_then(Value::as_str)
                .is_some_and(|value| value == category)
        });
    }

    if let Some(source_id) =
        source_id.map(str::trim).filter(|value| !value.is_empty() && *value != "all")
    {
        entries.retain(|entry| {
            entry
                .get("sourceId")
                .and_then(Value::as_str)
                .is_some_and(|value| value == source_id)
        });
    }

    if let Some(query) = query.map(str::trim).filter(|value| !value.is_empty()) {
        let needle = query.to_ascii_lowercase();
        entries.retain(|entry| marketplace_entry_matches(entry, &needle));
    }

    let mut categories: Vec<String> = entries
        .iter()
        .filter_map(|entry| {
            entry
                .get("category")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
        .collect();
    categories.sort();
    categories.dedup();

    let total = entries.len();
    let catalog = crate::marketplace_remote::remote_catalog_counts();

    let mut sources = vec![json!({
        "id": "native",
        "name": "Native",
        "type": "native",
        "location": path_to_string(&project_dir),
        "enabled": true,
        "official": true,
        "status": "ready",
        "lastSyncedAt": now_ms(),
    })];
    if let Some(path) = &load_config.marketplace_path {
        sources.push(json!({
            "id": "marketplace-file",
            "name": "Marketplace cache",
            "type": "native",
            "location": path_to_string(path),
            "enabled": true,
            "official": true,
            "status": if path.is_file() { "ready" } else { "unavailable" },
            "lastSyncedAt": now_ms(),
        }));
    }
    sources.extend(remote.sources);

    if summary_only {
        return json!({
            "entries": [],
            "total": total,
            "categories": categories,
            "sources": sources,
            "kinds": filter_kinds,
            "servedFromCache": remote.served_from_cache,
            "incomplete": remote.incomplete,
            "catalogCounts": {
                "mcp": catalog.mcp,
                "skills": catalog.skills,
                "plugins": catalog.plugins,
                "mcpComplete": catalog.mcp_complete,
                "skillComplete": catalog.skill_complete,
                "pluginComplete": catalog.plugin_complete,
            },
            "requestId": request_id,
        });
    }

    let page_entries = if let Some(page_size) = page_size {
        let page = page.unwrap_or(1).max(1);
        let page_size = page_size.clamp(1, 100);
        let start = (page - 1).saturating_mul(page_size);
        entries.into_iter().skip(start).take(page_size).collect::<Vec<_>>()
    } else {
        entries
    };

    json!({
        "entries": page_entries,
        "total": total,
        "categories": categories,
        "sources": sources,
        "kinds": filter_kinds,
        "servedFromCache": remote.served_from_cache,
        "incomplete": remote.incomplete,
        "catalogCounts": {
            "mcp": catalog.mcp,
            "skills": catalog.skills,
            "plugins": catalog.plugins,
            "mcpComplete": catalog.mcp_complete,
            "skillComplete": catalog.skill_complete,
            "pluginComplete": catalog.plugin_complete,
        },
        "requestId": request_id,
    })
}

pub async fn marketplace_install_plan(
    project_dir: PathBuf,
    config: &Value,
    entry_id: &str,
    scope: Option<&str>,
) -> anyhow::Result<Value> {
    let entry = find_market_entry_async(project_dir, config, entry_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Marketplace entry not found: {entry_id}"))?;
    if entry
        .get("installable")
        .and_then(Value::as_bool)
        == Some(false)
    {
        let reason = entry
            .get("installReason")
            .and_then(Value::as_str)
            .unwrap_or("Marketplace entry is not installable")
            .to_string();
        anyhow::bail!(reason);
    }
    let scope = normalize_scope(scope);
    let created_at = now_ms();
    let plan_id = format!("plan_{created_at}_{}", entry_id.replace('.', "_"));
    let digest = entry
        .pointer("/provenance/digest")
        .and_then(Value::as_str)
        .unwrap_or(entry_id);
    let approval_digest = digest_text(&format!("{digest}:{scope}"));
    let requested_effects = entry
        .get("requestedEffects")
        .cloned()
        .unwrap_or_else(|| json!([]));
    let provenance = entry.get("provenance").cloned().unwrap_or_else(|| json!({
        "registry": "native",
        "digest": digest,
    }));
    let plan = json!({
        "planId": plan_id,
        "entry": entry,
        "scope": scope,
        "approvalDigest": approval_digest,
        "requestedEffects": requested_effects,
        "provenance": provenance,
        "risks": [
            "Desktop install applies files into the local extensions directory.",
            "Review requestedEffects before confirming.",
        ],
        "changes": [
            format!("Install {}@{}", entry.get("name").and_then(Value::as_str).unwrap_or(entry_id), entry.get("version").and_then(Value::as_str).unwrap_or("0.0.0")),
            format!("Scope: {scope}"),
        ],
        "rollback": [
            "Disable the capability from Installed list.",
            "Remove copied plugin files from the runtime data directory if needed.",
        ],
        "createdAt": created_at,
        "expiresAt": created_at + 15 * 60_000,
    });
    MARKETPLACE_PLANS
        .write()
        .expect("marketplace plans lock")
        .insert(plan_id.clone(), plan.clone());
    Ok(plan)
}

pub fn marketplace_install_confirm_plan(
    plan_id: &str,
    approval_digest: &str,
) -> Result<Value, String> {
    let plan = MARKETPLACE_PLANS
        .read()
        .expect("marketplace plans lock")
        .get(plan_id)
        .cloned()
        .ok_or_else(|| "Install plan expired or missing".to_string())?;
    if plan
        .get("expiresAt")
        .and_then(Value::as_i64)
        .is_some_and(|expires| expires < now_ms())
    {
        return Err("Install plan expired or missing".to_string());
    }
    if plan
        .get("approvalDigest")
        .and_then(Value::as_str)
        .unwrap_or_default()
        != approval_digest
    {
        return Err(
            "Install approval does not match entry digest, version, scope, and effects".to_string(),
        );
    }
    Ok(plan)
}

pub fn marketplace_install_record(
    plan: &Value,
    install_result: &Value,
) -> Value {
    let entry = plan.get("entry").cloned().unwrap_or_else(|| json!({}));
    let entry_id = entry
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let ok = install_result.get("ok").and_then(Value::as_bool) == Some(true);
    let operation = json!({
        "operationId": format!("op_{}", now_ms()),
        "entryId": entry_id,
        "kind": entry.get("kind").and_then(Value::as_str).unwrap_or("plugin"),
        "action": "install",
        "status": if ok { "completed" } else { "failed" },
        "stages": [
            { "name": "verify", "status": "completed", "at": now_ms() },
            { "name": "install", "status": if ok { "completed" } else { "failed" }, "at": now_ms() },
            { "name": "lock", "status": if ok { "completed" } else { "failed" }, "at": now_ms() },
        ],
        "error": install_result.get("error").cloned(),
        "createdAt": now_ms(),
        "updatedAt": now_ms(),
    });
    MARKETPLACE_OPERATIONS
        .write()
        .expect("marketplace operations lock")
        .push(operation.clone());
    operation
}

pub fn marketplace_install_status() -> Value {
    let operations = MARKETPLACE_OPERATIONS
        .read()
        .expect("marketplace operations lock")
        .clone();
    json!({ "operations": operations })
}

pub fn capability_installed_list(extensions: &Value) -> Value {
    let mut capabilities = Vec::new();
    if let Some(plugins) = extensions.get("plugins").and_then(Value::as_array) {
        for plugin in plugins {
            if let Some(record) = installed_from_plugin(plugin) {
                capabilities.push(record);
            }
        }
    }
    if let Some(skills) = extensions.get("skills").and_then(Value::as_array) {
        for skill in skills {
            if let Some(record) = installed_from_skill(skill) {
                capabilities.push(record);
            }
        }
    }
    json!({ "capabilities": capabilities })
}

pub fn capability_installed_mutate(
    extensions: &Value,
    entry_id: &str,
    action: &str,
    ok: bool,
    error: Option<&str>,
) -> Value {
    let capabilities = capability_installed_list(extensions);
    let Some(record) = capabilities
        .get("capabilities")
        .and_then(Value::as_array)
        .and_then(|items| {
            items
                .iter()
                .find(|item| item.get("id").and_then(Value::as_str) == Some(entry_id))
                .cloned()
        })
    else {
        return failed_operation(action, entry_id, &format!("Installed capability not found: {entry_id}"));
    };
    if record.get("builtin").and_then(Value::as_bool) == Some(true) {
        return failed_operation(action, entry_id, "Built-in capabilities cannot be mutated");
    }
    match action {
        "enable" | "disable" | "uninstall" | "rollback" => {
            record_operation(entry_id, record, action, ok, error)
        }
        _ => failed_operation(action, entry_id, "Unknown action"),
    }
}

async fn find_market_entry_async(
    project_dir: PathBuf,
    config: &Value,
    entry_id: &str,
) -> anyhow::Result<Option<Value>> {
    if let Some(entry) = crate::marketplace_remote::find_cached_remote_entry(entry_id) {
        return Ok(Some(entry));
    }
    let result = marketplace_search_async(MarketplaceSearchParams {
        project_dir,
        config,
        query: None,
        kind: None,
        kinds: None,
        request_id: None,
        page: None,
        page_size: None,
        category: None,
        source_id: None,
        summary_only: false,
    })
    .await;
    Ok(result
        .get("entries")
        .and_then(Value::as_array)
        .and_then(|entries| {
            entries
                .iter()
                .find(|entry| entry.get("id").and_then(Value::as_str) == Some(entry_id))
                .cloned()
        }))
}

fn plugin_entry(plugin: &PluginManifest, kind: &str) -> Value {
    let digest = digest_text(&format!("{}:{}", plugin.id, plugin.version));
    let install = if plugin.source == "marketplace" {
        json!({
            "type": "url",
            "url": plugin.marketplace_url,
            "path": plugin.directory,
        })
    } else {
        json!({
            "type": "local",
            "path": plugin.directory,
        })
    };
    json!({
        "id": plugin.id,
        "kind": kind,
        "name": plugin.name,
        "version": plugin.version,
        "description": plugin.description,
        "author": "",
        "category": if plugin.source == "marketplace" { "Marketplace" } else { "Plugins" },
        "tags": [],
        "requestedEffects": requested_effects(plugin),
        "install": install,
        "sourceId": "native",
        "installable": plugin.directory.is_some() || plugin.marketplace_url.is_some(),
        "installReason": if plugin.directory.is_some() || plugin.marketplace_url.is_some() { Value::Null } else { Value::String("Missing install source".to_string()) },
        "provenance": {
            "registry": plugin.source,
            "digest": digest,
        }
    })
}

fn skill_entry(skill: &SkillDef) -> Value {
    let digest = digest_text(&format!("{}:{}", skill.id, skill.name));
    json!({
        "id": skill.id,
        "kind": "skill",
        "name": skill.name,
        "version": "0.0.0",
        "description": skill.description,
        "author": "",
        "category": "Skills",
        "tags": skill.tools.clone(),
        "requestedEffects": ["read"],
        "install": {
            "type": "skill",
            "path": skill.file_path,
        },
        "sourceId": skill.source,
        "installable": false,
        "installReason": "Skills are loaded from workspace directories",
        "provenance": {
            "registry": skill.source,
            "digest": digest,
        }
    })
}

fn installed_from_plugin(plugin: &Value) -> Option<Value> {
    let id = plugin.get("id").and_then(Value::as_str)?;
    Some(json!({
        "id": id,
        "name": plugin.get("name").and_then(Value::as_str).unwrap_or(id),
        "version": plugin.get("version").and_then(Value::as_str).unwrap_or("0.0.0"),
        "enabled": plugin.get("enabled").and_then(Value::as_bool).unwrap_or(true),
        "kind": "plugin",
        "builtin": plugin.get("source").and_then(Value::as_str) == Some("builtin"),
        "entry": plugin,
    }))
}

fn installed_from_skill(skill: &Value) -> Option<Value> {
    let id = skill.get("id").and_then(Value::as_str)?;
    Some(json!({
        "id": id,
        "name": skill.get("name").and_then(Value::as_str).unwrap_or(id),
        "version": "0.0.0",
        "enabled": true,
        "kind": "skill",
        "builtin": skill.get("source").and_then(Value::as_str) == Some("system"),
        "entry": skill,
    }))
}

fn requested_effects(plugin: &PluginManifest) -> Value {
    let mut effects = Vec::new();
    if plugin
        .permissions
        .as_ref()
        .is_some_and(|permissions| permissions.network.enabled)
    {
        effects.push("network");
    }
    if plugin
        .permissions
        .as_ref()
        .is_some_and(|permissions| permissions.process)
    {
        effects.push("process");
    }
    if plugin
        .permissions
        .as_ref()
        .is_some_and(|permissions| !permissions.fs.write.is_empty())
    {
        effects.push("write");
    }
    if effects.is_empty() {
        effects.push("read");
    }
    json!(effects)
}

fn push_market_entry(entries: &mut Vec<Value>, seen: &mut BTreeSet<String>, entry: Value) {
    let Some(id) = entry.get("id").and_then(Value::as_str) else {
        return;
    };
    if seen.insert(id.to_string()) {
        entries.push(entry);
    }
}

fn ensure_marketplace_path(mut config: ExtensionLoadConfig, project_dir: &Path) -> ExtensionLoadConfig {
    if config.marketplace_path.as_ref().is_some_and(|path| path.is_file()) {
        return config;
    }
    for candidate in marketplace_catalog_candidates(project_dir, None) {
        if candidate.is_file() {
            config.marketplace_path = Some(candidate);
            break;
        }
    }
    config
}

fn marketplace_catalog_candidates(
    project_dir: &Path,
    configured: Option<&Path>,
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(path) = configured {
        out.push(path.to_path_buf());
    }
    if let Ok(raw) = std::env::var("XUANYU_MARKETPLACE_JSON") {
        let path = PathBuf::from(raw.trim());
        if !path.as_os_str().is_empty() {
            out.push(path);
        }
    }
    out.push(project_dir.join("marketplace.json"));
    out.push(project_dir.join("plugins").join("marketplace.json"));
    out.push(project_data_root(project_dir).join("marketplace.json"));
    // Dev fallback
    out.push(PathBuf::from("apps/desktop-ui/public/plugins/marketplace.json"));
    out
}

fn load_raw_marketplace_entries(path: &Path) -> Vec<Value> {
    let Ok(raw) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(parsed) = serde_json::from_str::<Value>(&raw) else {
        return Vec::new();
    };
    let entries = if let Some(array) = parsed.as_array() {
        array.clone()
    } else {
        parsed
            .get("plugins")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    entries
        .into_iter()
        .filter_map(|entry| normalize_raw_market_entry(entry, path))
        .collect()
}

fn load_bundled_marketplace_entries() -> Vec<Value> {
    const BUNDLED: &str = include_str!("../assets/marketplace.json");
    let Ok(parsed) = serde_json::from_str::<Value>(BUNDLED) else {
        return Vec::new();
    };
    let entries = parsed
        .get("plugins")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let fake_path = PathBuf::from("bundled://marketplace.json");
    entries
        .into_iter()
        .filter_map(|entry| normalize_raw_market_entry(entry, &fake_path))
        .collect()
}

fn normalize_raw_market_entry(mut entry: Value, source_path: &Path) -> Option<Value> {
    let object = entry.as_object_mut()?;
    let id = object
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .to_string();
    let kind = {
        let install_type = object
            .get("install")
            .and_then(|install| install.get("type"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_lowercase();
        let raw_kind = object
            .get("kind")
            .and_then(Value::as_str)
            .or_else(|| object.get("category").and_then(Value::as_str))
            .unwrap_or("plugin")
            .to_ascii_lowercase();
        if matches!(install_type.as_str(), "mcp") || matches!(raw_kind.as_str(), "mcp" | "search")
        {
            "mcp"
        } else if matches!(install_type.as_str(), "skill")
            || matches!(raw_kind.as_str(), "skill" | "skills")
        {
            "skill"
        } else if matches!(raw_kind.as_str(), "channels" | "channel") {
            "channel"
        } else if matches!(
            raw_kind.as_str(),
            "tools" | "tool" | "database" | "memory" | "devops" | "development"
        ) {
            // Local sample catalog used mixed categories; map by install type above first.
            if install_type == "mcp" {
                "mcp"
            } else if install_type == "skill" {
                "skill"
            } else if matches!(raw_kind.as_str(), "tools" | "tool" | "database" | "memory" | "search")
            {
                "mcp"
            } else if matches!(raw_kind.as_str(), "devops" | "development") {
                "skill"
            } else {
                "plugin"
            }
        } else {
            "plugin"
        }
    };
    let version = object
        .get("version")
        .and_then(Value::as_str)
        .unwrap_or("0.0.0")
        .to_string();
    let digest = digest_text(&format!("{id}:{version}:{}", path_to_string(source_path)));
    object.insert("kind".to_string(), json!(kind));
    object
        .entry("author")
        .or_insert_with(|| json!(""));
    object
        .entry("category")
        .or_insert_with(|| json!(kind));
    object
        .entry("tags")
        .or_insert_with(|| json!([]));
    object
        .entry("requestedEffects")
        .or_insert_with(|| json!(["read"]));
    object
        .entry("sourceId")
        .or_insert_with(|| json!("marketplace-file"));
    object.insert(
        "installable".to_string(),
        json!(object.get("install").is_some()),
    );
    if object.get("install").is_none() {
        object.insert(
            "installReason".to_string(),
            json!("Missing install source"),
        );
    }
    object.insert(
        "provenance".to_string(),
        json!({
            "registry": "marketplace-file",
            "digest": digest,
        }),
    );
    Some(entry)
}

fn is_channel_market_entry(entry: &Value) -> bool {
    if entry.get("kind").and_then(Value::as_str) == Some("channel") {
        return true;
    }
    let category = entry
        .get("category")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    category == "channels" || category == "channel"
}

fn normalize_kinds(kind: Option<&str>, kinds: Option<Vec<String>>) -> Vec<String> {
    let mut out = BTreeSet::new();
    if let Some(kind) = kind.map(str::trim).filter(|value| !value.is_empty()) {
        out.insert(kind.to_string());
    }
    if let Some(kinds) = kinds {
        for kind in kinds {
            let kind = kind.trim();
            if !kind.is_empty() {
                out.insert(kind.to_string());
            }
        }
    }
    out.into_iter().collect()
}

fn normalize_scope(scope: Option<&str>) -> &'static str {
    match scope.unwrap_or("user") {
        "project" => "project",
        "org" => "org",
        _ => "user",
    }
}

fn marketplace_entry_matches(entry: &Value, needle: &str) -> bool {
    [
        entry.get("id").and_then(Value::as_str),
        entry.get("name").and_then(Value::as_str),
        entry.get("description").and_then(Value::as_str),
        entry.get("category").and_then(Value::as_str),
    ]
    .into_iter()
    .flatten()
    .any(|value| value.to_ascii_lowercase().contains(needle))
        || entry
            .get("tags")
            .and_then(Value::as_array)
            .is_some_and(|tags| {
                tags.iter().filter_map(Value::as_str).any(|tag| {
                    tag.to_ascii_lowercase().contains(needle)
                })
            })
}

fn record_operation(
    entry_id: &str,
    record: Value,
    action: &str,
    ok: bool,
    error: Option<&str>,
) -> Value {
    let operation = json!({
        "operationId": format!("op_{}", now_ms()),
        "entryId": entry_id,
        "kind": record.get("kind").and_then(Value::as_str).unwrap_or("plugin"),
        "action": action,
        "status": if ok { "completed" } else { "failed" },
        "stages": [{ "name": action, "status": if ok { "completed" } else { "failed" }, "at": now_ms() }],
        "error": error,
        "createdAt": now_ms(),
        "updatedAt": now_ms(),
    });
    MARKETPLACE_OPERATIONS
        .write()
        .expect("marketplace operations lock")
        .push(operation.clone());
    operation
}

pub fn capabilities_lock_path(project_dir: &Path) -> PathBuf {
    project_data_root(project_dir).join("capabilities.lock.json")
}

pub fn read_capabilities_lock(project_dir: &Path) -> Value {
    let path = capabilities_lock_path(project_dir);
    let Ok(raw) = fs::read_to_string(&path) else {
        return json!({ "version": 1, "capabilities": [] });
    };
    serde_json::from_str(&raw).unwrap_or_else(|_| json!({ "version": 1, "capabilities": [] }))
}

pub fn write_capabilities_lock(project_dir: &Path, lock: &Value) -> anyhow::Result<()> {
    let path = capabilities_lock_path(project_dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    write_json_atomic(&path, lock)
}

pub struct UpsertInstalledCapabilityInput<'a> {
    pub project_dir: &'a Path,
    pub id: &'a str,
    pub kind: &'a str,
    pub name: &'a str,
    pub version: &'a str,
    pub digest: &'a str,
    pub enabled: bool,
    pub scope: &'a str,
    pub path: &'a str,
    pub previous_artifact_path: Option<&'a str>,
}

pub fn upsert_installed_capability(
    input: UpsertInstalledCapabilityInput<'_>,
) -> anyhow::Result<()> {
    let UpsertInstalledCapabilityInput {
        project_dir,
        id,
        kind,
        name,
        version,
        digest,
        enabled,
        scope,
        path,
        previous_artifact_path,
    } = input;
    let mut lock = read_capabilities_lock(project_dir);
    let capabilities = lock
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("capabilities lock root must be an object"))?
        .entry("capabilities")
        .or_insert_with(|| json!([]));
    let items = capabilities
        .as_array_mut()
        .ok_or_else(|| anyhow::anyhow!("capabilities lock capabilities must be an array"))?;
    if let Some(existing) = items.iter_mut().find(|item| {
        item.get("id").and_then(Value::as_str) == Some(id)
    }) {
        if let Some(previous_path) = previous_artifact_path.filter(|value| !value.is_empty()) {
            let previous = json!({
                "version": existing.get("version").cloned().unwrap_or_else(|| json!(version)),
                "digest": existing.get("digest").cloned().unwrap_or_else(|| json!(digest)),
                "path": previous_path,
                "enabled": existing.get("enabled").and_then(Value::as_bool).unwrap_or(enabled),
            });
            match existing.get_mut("previous") {
                Some(Value::Array(stack)) => stack.push(previous),
                _ => {
                    if let Some(object) = existing.as_object_mut() { object.insert("previous".to_string(), json!([previous])); }
                }
            }
        }
        if let Some(object) = existing.as_object_mut() {
            object.insert("kind".to_string(), json!(kind));
            object.insert("name".to_string(), json!(name));
            object.insert("version".to_string(), json!(version));
            object.insert("digest".to_string(), json!(digest));
            object.insert("enabled".to_string(), json!(enabled));
            object.insert("scope".to_string(), json!(scope));
            object.insert("path".to_string(), json!(path));
        }
    } else {
        items.push(json!({
            "id": id,
            "kind": kind,
            "name": name,
            "version": version,
            "digest": digest,
            "enabled": enabled,
            "scope": scope,
            "installedAt": now_ms(),
            "path": path,
        }));
    }
    write_capabilities_lock(project_dir, &lock)
}

pub fn remove_lock_record(project_dir: &Path, entry_id: &str) -> anyhow::Result<Option<Value>> {
    let mut lock = read_capabilities_lock(project_dir);
    let Some(items) = lock.get_mut("capabilities").and_then(Value::as_array_mut) else {
        return Ok(None);
    };
    let index = items
        .iter()
        .position(|item| item.get("id").and_then(Value::as_str) == Some(entry_id));
    let Some(index) = index else {
        return Ok(None);
    };
    let removed = items.remove(index);
    write_capabilities_lock(project_dir, &lock)?;
    Ok(Some(removed))
}

pub fn extract_previous_from_lock(
    lock: &mut Value,
    entry_id: &str,
) -> anyhow::Result<(Value, Value)> {
    let items = lock
        .get_mut("capabilities")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| anyhow::anyhow!("Installed capability not found: {entry_id}"))?;
    let record = items
        .iter_mut()
        .find(|item| item.get("id").and_then(Value::as_str) == Some(entry_id))
        .ok_or_else(|| anyhow::anyhow!("Installed capability not found: {entry_id}"))?;
    let previous = record
        .get_mut("previous")
        .and_then(|value| value.as_array_mut())
        .and_then(|stack| stack.pop())
        .ok_or_else(|| anyhow::anyhow!("No rollback version is available"))?;
    Ok((record.clone(), previous))
}

pub fn apply_lock_rollback_record_in_memory(
    lock: &mut Value,
    entry_id: &str,
    version: &str,
    digest: &str,
    enabled: bool,
) -> anyhow::Result<()> {
    let record = lock
        .get_mut("capabilities")
        .and_then(Value::as_array_mut)
        .and_then(|items| {
            items
                .iter_mut()
                .find(|item| item.get("id").and_then(Value::as_str) == Some(entry_id))
        })
        .ok_or_else(|| anyhow::anyhow!("Installed capability not found: {entry_id}"))?;
    if let Some(object) = record.as_object_mut() {
        object.insert("version".to_string(), json!(version));
        object.insert("digest".to_string(), json!(digest));
        object.insert("enabled".to_string(), json!(enabled));
    }
    Ok(())
}

pub fn marketplace_backups_root(project_dir: &Path) -> PathBuf {
    project_data_root(project_dir).join("marketplace-backups")
}

pub fn backup_installed_directory(
    project_dir: &Path,
    entry_id: &str,
    version: &str,
    active_path: &Path,
) -> anyhow::Result<Option<PathBuf>> {
    if !active_path.is_dir() {
        return Ok(None);
    }
    let state_dir = project_data_root(project_dir);
    if !is_path_inside(active_path, &state_dir) {
        return Ok(None);
    }
    let backup_path = marketplace_backups_root(project_dir)
        .join(safe_path_segment(entry_id))
        .join(format!("{}-{}", safe_path_segment(version), now_ms()));
    if let Some(parent) = backup_path.parent() {
        fs::create_dir_all(parent)?;
    }
    copy_directory(active_path, &backup_path)?;
    Ok(Some(backup_path))
}

pub fn restore_installed_directory(active_path: &Path, backup_path: &Path) -> anyhow::Result<()> {
    if active_path == backup_path || !backup_path.is_dir() {
        anyhow::bail!("Rollback artifact path is invalid or missing");
    }
    if active_path.exists() {
        fs::remove_dir_all(active_path)?;
    }
    if let Some(parent) = active_path.parent() {
        fs::create_dir_all(parent)?;
    }
    copy_directory(backup_path, active_path)
}

pub fn is_path_inside(child: &Path, parent: &Path) -> bool {
    let Ok(child) = fs::canonicalize(child) else {
        return false;
    };
    let Ok(parent) = fs::canonicalize(parent) else {
        return false;
    };
    child.starts_with(parent)
}

pub fn failed_operation(action: &str, entry_id: &str, error: &str) -> Value {
    let operation = json!({
        "operationId": format!("op_{}", now_ms()),
        "entryId": entry_id,
        "kind": "plugin",
        "action": action,
        "status": "failed",
        "stages": [{ "name": action, "status": "failed", "message": error, "at": now_ms() }],
        "error": error,
        "createdAt": now_ms(),
        "updatedAt": now_ms(),
    });
    MARKETPLACE_OPERATIONS
        .write()
        .expect("marketplace operations lock")
        .push(operation.clone());
    operation
}

fn digest_text(text: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn write_json_atomic(path: &Path, value: &Value) -> anyhow::Result<()> {
    let serialized = serde_json::to_string_pretty(value)? + "\n";
    let temp_path = path.with_extension("tmp");
    {
        let mut file = fs::File::create(&temp_path)?;
        file.write_all(serialized.as_bytes())?;
        file.sync_all()?;
    }
    fs::rename(temp_path, path)?;
    Ok(())
}

fn safe_path_segment(value: &str) -> String {
    let mut output = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            output.push(ch);
        } else {
            output.push('_');
        }
    }
    if output.is_empty() {
        "entry".to_string()
    } else {
        output
    }
}

fn copy_directory(source: &Path, target: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        let metadata = fs::symlink_metadata(&source_path)?;
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            anyhow::bail!(
                "Refusing to copy symlink '{}'.",
                path_to_string(&source_path)
            );
        }
        if file_type.is_dir() {
            copy_directory(&source_path, &target_path)?;
        } else if file_type.is_file() {
            fs::copy(&source_path, &target_path)?;
        }
    }
    Ok(())
}
