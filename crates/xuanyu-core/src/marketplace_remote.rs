//! Official remote marketplace catalogs (MCP registry, skill hub, plugin directory).

use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use xuanyu_storage::runtime_data_root;

const MCP_REGISTRY: &str = "https://registry.modelcontextprotocol.io/v0.1/servers";
const CLAWHUB_SKILLS: &str = "https://clawhub.ai/api/v1/skills";
const PLUGIN_OFFICIAL_MARKETPLACE: &str =
    "https://raw.githubusercontent.com/anthropics/claude-plugins-official/main/.claude-plugin/marketplace.json";
const PLUGIN_COMMUNITY_MARKETPLACE: &str =
    "https://raw.githubusercontent.com/anthropics/claude-plugins-community/main/.claude-plugin/marketplace.json";

const CACHE_TTL: Duration = Duration::from_secs(15 * 60);
const DISK_CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const MCP_PAGE_LIMIT: u32 = 100;
/// Soft safety cap (~25k servers ≈ 250 pages). Cursor exhaustion ends earlier.
const MCP_MAX_PAGES: u32 = 400;
const MCP_WARM_PAGES: u32 = 2;
const SKILL_PAGE_LIMIT: u32 = 200;
/// Soft safety cap for ClawHub skill pages (API max page size is 200).
const SKILL_MAX_PAGES: u32 = 200;
const SKILL_WARM_PAGES: u32 = 1;

#[derive(Clone, Default)]
struct RemoteCache {
    fetched_at: Option<Instant>,
    saved_at_ms: u64,
    mcp: Vec<Value>,
    skills: Vec<Value>,
    plugins: Vec<Value>,
    sources: Vec<Value>,
    mcp_complete: bool,
    skill_complete: bool,
    plugin_complete: bool,
    mcp_cursor: Option<String>,
    skill_cursor: Option<String>,
    disk_loaded: bool,
}

static REMOTE_CACHE: LazyLock<Mutex<RemoteCache>> =
    LazyLock::new(|| Mutex::new(RemoteCache::default()));
static MCP_FILLING: AtomicBool = AtomicBool::new(false);
static SKILL_FILLING: AtomicBool = AtomicBool::new(false);
static PLUGIN_FILLING: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteKindFilter {
    All,
    Mcp,
    Skill,
    Plugin,
}

impl RemoteKindFilter {
    pub fn from_kinds(kind: Option<&str>, kinds: Option<&[String]>) -> Self {
        let mut set = std::collections::BTreeSet::new();
        if let Some(kind) = kind.map(str::trim).filter(|v| !v.is_empty()) {
            set.insert(kind.to_ascii_lowercase());
        }
        if let Some(kinds) = kinds {
            for item in kinds {
                let item = item.trim();
                if !item.is_empty() {
                    set.insert(item.to_ascii_lowercase());
                }
            }
        }
        if set.is_empty() {
            return Self::All;
        }
        let wants_mcp = set.iter().any(|k| k == "mcp");
        let wants_skill = set.iter().any(|k| k == "skill" || k == "skills");
        let wants_plugin = set.iter().any(|k| k == "plugin" || k == "plugins");
        match (wants_mcp, wants_skill, wants_plugin) {
            (true, false, false) => Self::Mcp,
            (false, true, false) => Self::Skill,
            (false, false, true) => Self::Plugin,
            _ => Self::All,
        }
    }
}

pub struct RemoteMarketplaceResult {
    pub entries: Vec<Value>,
    pub sources: Vec<Value>,
    pub incomplete: bool,
    pub served_from_cache: bool,
}

pub struct RemoteCatalogCounts {
    pub mcp: usize,
    pub skills: usize,
    pub plugins: usize,
    pub mcp_complete: bool,
    pub skill_complete: bool,
    pub plugin_complete: bool,
}

pub fn remote_catalog_counts() -> RemoteCatalogCounts {
    hydrate_disk_cache();
    let cache = REMOTE_CACHE
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    RemoteCatalogCounts {
        mcp: cache.mcp.len(),
        skills: cache.skills.len(),
        plugins: cache.plugins.len(),
        mcp_complete: cache.mcp_complete,
        skill_complete: cache.skill_complete,
        plugin_complete: cache.plugin_complete,
    }
}

pub fn find_cached_remote_entry(entry_id: &str) -> Option<Value> {
    hydrate_disk_cache();
    let cache = REMOTE_CACHE.lock().ok()?;
    cache
        .mcp
        .iter()
        .chain(cache.skills.iter())
        .chain(cache.plugins.iter())
        .find(|entry| entry.get("id").and_then(Value::as_str) == Some(entry_id))
        .cloned()
}

/// Load disk cache and resume incomplete MCP/skill/plugin fills in background threads.
pub fn warm_remote_marketplace_cache() {
    hydrate_disk_cache();
}

pub async fn fetch_remote_marketplace(filter: RemoteKindFilter) -> RemoteMarketplaceResult {
    hydrate_disk_cache();
    if let Some(cached) = snapshot_cache(filter) {
        let stale = cache_is_stale();
        if cached.incomplete || stale {
            // incomplete → resume; stale complete → re-walk without wiping.
            spawn_background_refresh(filter, stale && !cached.incomplete);
        }
        return cached;
    }

    let client = match http_client() {
        Some(client) => client,
        None => return empty_result_with_errors(&[("native", "http client unavailable")]),
    };

    let warm = warm_fetch(&client, filter).await;
    persist_disk_cache();
    if warm.incomplete {
        spawn_background_refresh(filter, false);
    }
    warm
}

fn http_client() -> Option<reqwest::Client> {
    match reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent("xuanyu-agent-desktop/0.1")
        .build()
    {
        Ok(client) => Some(client),
        Err(err) => {
            tracing::warn!("marketplace http client failed: {err:#}");
            None
        }
    }
}

fn cache_is_stale() -> bool {
    let Ok(cache) = REMOTE_CACHE.lock() else {
        return true;
    };
    if cache.saved_at_ms == 0 {
        return cache
            .fetched_at
            .map(|at| at.elapsed() > CACHE_TTL)
            .unwrap_or(true);
    }
    now_ms().saturating_sub(cache.saved_at_ms) > DISK_CACHE_TTL.as_millis() as u64
}

fn snapshot_cache(filter: RemoteKindFilter) -> Option<RemoteMarketplaceResult> {
    let cache = REMOTE_CACHE.lock().ok()?;
    if !cache_has_data(&cache, filter) {
        return None;
    }
    let incomplete = cache_incomplete(&cache, filter);
    let mut sources = cache.sources.clone();
    if incomplete {
        mark_sources_status(&mut sources, filter, "loading");
    } else if cache
        .fetched_at
        .map(|at| at.elapsed() > CACHE_TTL)
        .unwrap_or(false)
    {
        mark_sources_status(&mut sources, filter, "cached");
    }
    Some(RemoteMarketplaceResult {
        entries: select_entries(filter, &cache.mcp, &cache.skills, &cache.plugins),
        sources,
        incomplete,
        served_from_cache: true,
    })
}

fn cache_has_data(cache: &RemoteCache, filter: RemoteKindFilter) -> bool {
    match filter {
        RemoteKindFilter::All => {
            !cache.mcp.is_empty() || !cache.skills.is_empty() || !cache.plugins.is_empty()
        }
        RemoteKindFilter::Mcp => !cache.mcp.is_empty(),
        RemoteKindFilter::Skill => !cache.skills.is_empty(),
        RemoteKindFilter::Plugin => !cache.plugins.is_empty(),
    }
}

fn cache_incomplete(cache: &RemoteCache, filter: RemoteKindFilter) -> bool {
    match filter {
        RemoteKindFilter::All => {
            !cache.mcp_complete || !cache.skill_complete || !cache.plugin_complete
        }
        RemoteKindFilter::Mcp => !cache.mcp_complete,
        RemoteKindFilter::Skill => !cache.skill_complete,
        RemoteKindFilter::Plugin => !cache.plugin_complete,
    }
}

fn mark_sources_status(sources: &mut [Value], filter: RemoteKindFilter, status: &str) {
    let ids: &[&str] = match filter {
        RemoteKindFilter::All => &[
            "mcp-registry",
            "clawhub",
            "claude-plugins-official",
            "claude-plugins-community",
        ],
        RemoteKindFilter::Mcp => &["mcp-registry"],
        RemoteKindFilter::Skill => &["clawhub"],
        RemoteKindFilter::Plugin => &["claude-plugins-official", "claude-plugins-community"],
    };
    for source in sources.iter_mut() {
        let id = source.get("id").and_then(Value::as_str).unwrap_or("");
        if ids.contains(&id) {
            source["status"] = json!(status);
        }
    }
}

async fn warm_fetch(client: &reqwest::Client, filter: RemoteKindFilter) -> RemoteMarketplaceResult {
    let need_mcp = matches!(filter, RemoteKindFilter::All | RemoteKindFilter::Mcp);
    let need_skill = matches!(filter, RemoteKindFilter::All | RemoteKindFilter::Skill);
    let need_plugin = matches!(filter, RemoteKindFilter::All | RemoteKindFilter::Plugin);

    let mcp_fut = async {
    if need_mcp {
            fetch_mcp_registry_pages(client, None, MCP_WARM_PAGES).await
        } else {
            Ok(PagedFetch::default())
        }
    };
    let skill_fut = async {
        if need_skill {
            fetch_clawhub_pages(client, None, SKILL_WARM_PAGES).await
        } else {
            Ok(PagedFetch::default())
        }
    };
    let plugin_fut = async {
        if need_plugin {
            fetch_plugin_marketplaces(client).await
        } else {
            PluginMarketplaceFetch::default()
        }
    };
    let (mcp_res, skill_res, plugin_fetch) = tokio::join!(mcp_fut, skill_fut, plugin_fut);

    let mut sources = Vec::new();
    let mut mcp = Vec::new();
    let mut mcp_cursor = None;
    let mut mcp_complete = !need_mcp;
    let mut skills = Vec::new();
    let mut skill_cursor = None;
    let mut skill_complete = !need_skill;
    let mut incomplete = false;

    match mcp_res {
        Ok(page) if need_mcp => {
            incomplete |= !page.complete;
            mcp_complete = page.complete;
            mcp_cursor = page.next_cursor.clone();
                sources.push(source_status(
                    "mcp-registry",
                    "MCP Registry",
                    "mcp-registry",
                    MCP_REGISTRY,
                if page.complete { "ready" } else { "loading" },
                    None,
                ));
            mcp = page.entries;
            }
        Err(err) if need_mcp => {
                tracing::warn!("MCP registry fetch failed: {err:#}");
                sources.push(source_status(
                    "mcp-registry",
                    "MCP Registry",
                    "mcp-registry",
                    MCP_REGISTRY,
                    "error",
                    Some(&err.to_string()),
                ));
            }
        _ => {}
    }

    match skill_res {
        Ok(page) if need_skill => {
            incomplete |= !page.complete;
            skill_complete = page.complete;
            skill_cursor = page.next_cursor.clone();
                sources.push(source_status(
                    "clawhub",
                    "ClawHub Skills",
                    "clawhub",
                    CLAWHUB_SKILLS,
                if page.complete { "ready" } else { "loading" },
                    None,
                ));
            skills = page.entries;
            }
        Err(err) if need_skill => {
                tracing::warn!("ClawHub skills fetch failed: {err:#}");
                sources.push(source_status(
                    "clawhub",
                    "ClawHub Skills",
                    "clawhub",
                    CLAWHUB_SKILLS,
                    "error",
                    Some(&err.to_string()),
                ));
            }
        _ => {}
    }

    let plugins = if need_plugin {
        sources.extend(plugin_fetch.sources.clone());
        plugin_fetch.plugins
    } else {
        Vec::new()
    };
    let plugin_complete = !need_plugin || !plugins.is_empty();

    {
        let mut cache = REMOTE_CACHE.lock().expect("remote marketplace cache");
        cache.fetched_at = Some(Instant::now());
        cache.saved_at_ms = now_ms();
        // Seed only empty categories. Never replace a larger partial/full catalog with warm pages.
        if need_mcp && !mcp.is_empty() && cache.mcp.is_empty() {
            cache.mcp = mcp.clone();
            cache.mcp_cursor = mcp_cursor;
            cache.mcp_complete = mcp_complete;
        }
        if need_skill && !skills.is_empty() && cache.skills.is_empty() {
            cache.skills = skills.clone();
            cache.skill_cursor = skill_cursor;
            cache.skill_complete = skill_complete;
        }
        if need_plugin && !plugins.is_empty() && cache.plugins.is_empty() {
            cache.plugins = plugins.clone();
            cache.plugin_complete = plugin_complete;
        }
        merge_sources(&mut cache.sources, &sources);
        let kept = select_entries(filter, &cache.mcp, &cache.skills, &cache.plugins);
        let kept_incomplete = cache_incomplete(&cache, filter);
        RemoteMarketplaceResult {
            entries: kept,
            sources: {
                let mut out = cache.sources.clone();
                merge_sources(&mut out, &sources);
                out
            },
            incomplete: kept_incomplete || incomplete,
            served_from_cache: false,
        }
    }
}

fn merge_sources(dst: &mut Vec<Value>, incoming: &[Value]) {
    for source in incoming {
        let id = source.get("id").and_then(Value::as_str).unwrap_or("");
        dst.retain(|item| item.get("id").and_then(Value::as_str) != Some(id));
        dst.push(source.clone());
    }
}

fn spawn_background_refresh(filter: RemoteKindFilter, restart: bool) {
    match filter {
        RemoteKindFilter::All => {
            spawn_kind_fill(FillKind::Mcp, restart);
            spawn_kind_fill(FillKind::Skill, restart);
            spawn_kind_fill(FillKind::Plugin, restart);
        }
        RemoteKindFilter::Mcp => spawn_kind_fill(FillKind::Mcp, restart),
        RemoteKindFilter::Skill => spawn_kind_fill(FillKind::Skill, restart),
        RemoteKindFilter::Plugin => spawn_kind_fill(FillKind::Plugin, restart),
    }
}

#[derive(Clone, Copy)]
enum FillKind {
    Mcp,
    Skill,
    Plugin,
}

fn fill_flag(kind: FillKind) -> &'static AtomicBool {
    match kind {
        FillKind::Mcp => &MCP_FILLING,
        FillKind::Skill => &SKILL_FILLING,
        FillKind::Plugin => &PLUGIN_FILLING,
    }
}

fn spawn_kind_fill(kind: FillKind, restart: bool) {
    let flag = fill_flag(kind);
    if flag
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }
    let name = match kind {
        FillKind::Mcp => "xy-market-mcp",
        FillKind::Skill => "xy-market-skill",
        FillKind::Plugin => "xy-market-plugin",
    };
    let spawn_result = std::thread::Builder::new().name(name.into()).spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(err) => {
                tracing::warn!("marketplace fill runtime failed: {err:#}");
                flag.store(false, Ordering::SeqCst);
                return;
            }
        };
        runtime.block_on(async move {
            let Some(client) = http_client() else {
                flag.store(false, Ordering::SeqCst);
                return;
            };
            if restart {
                reset_cursor_for_rewalk(kind);
            }
            match kind {
                FillKind::Mcp => continue_fill_mcp(&client).await,
                FillKind::Skill => continue_fill_skill(&client).await,
                FillKind::Plugin => continue_fill_plugin(&client).await,
            }
            persist_disk_cache();
            flag.store(false, Ordering::SeqCst);
        });
    });
    if let Err(err) = spawn_result {
        tracing::warn!("marketplace fill thread failed: {err:#}");
        flag.store(false, Ordering::SeqCst);
    }
}

fn reset_cursor_for_rewalk(kind: FillKind) {
    let Ok(mut cache) = REMOTE_CACHE.lock() else {
        return;
    };
    match kind {
        FillKind::Mcp => {
            cache.mcp_cursor = None;
            cache.mcp_complete = false;
        }
        FillKind::Skill => {
            cache.skill_cursor = None;
            cache.skill_complete = false;
        }
        FillKind::Plugin => {
            cache.plugin_complete = false;
        }
    }
}

async fn continue_fill_mcp(client: &reqwest::Client) {
    let (cursor, already) = {
        let cache = REMOTE_CACHE.lock().expect("remote marketplace cache");
        (cache.mcp_cursor.clone(), cache.mcp_complete)
    };
    if already {
        return;
    }
    let mut cursor = cursor;
    let mut hops = 0u32;
    let mut consecutive_errors = 0u32;
    loop {
        hops += 1;
        if hops > MCP_MAX_PAGES {
            tracing::warn!("MCP registry background fill hit hop cap ({} entries)", {
                REMOTE_CACHE
                    .lock()
                    .map(|c| c.mcp.len())
                    .unwrap_or(0)
            });
            break;
        }
        match fetch_mcp_registry_pages(client, cursor.clone(), 10).await {
            Ok(page) => {
                consecutive_errors = 0;
                let complete = page.complete;
                let next = page.next_cursor.clone();
                let added = page.entries.len();
                {
                    let mut cache = REMOTE_CACHE.lock().expect("remote marketplace cache");
                    append_unique(&mut cache.mcp, page.entries);
                    cache.mcp_cursor = next.clone();
                    cache.mcp_complete = complete;
                    cache.fetched_at = Some(Instant::now());
                    merge_sources(
                        &mut cache.sources,
                        &[source_status(
                            "mcp-registry",
                            "MCP Registry",
                            "mcp-registry",
                            MCP_REGISTRY,
                            if complete { "ready" } else { "loading" },
                            None,
                        )],
                    );
                    tracing::info!(
                        "MCP registry fill hop={hops} added={added} total={} complete={complete}",
                        cache.mcp.len()
                    );
                }
                if hops.is_multiple_of(2) {
                    persist_disk_cache();
                }
                if complete {
                    break;
                }
                cursor = next;
            }
            Err(err) => {
                consecutive_errors += 1;
                tracing::warn!(
                    "MCP registry background fill failed (try {consecutive_errors}): {err:#}"
                );
                if consecutive_errors >= 3 {
                    break;
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}

async fn continue_fill_skill(client: &reqwest::Client) {
    let (cursor, already) = {
        let cache = REMOTE_CACHE.lock().expect("remote marketplace cache");
        (cache.skill_cursor.clone(), cache.skill_complete)
    };
    if already {
        return;
    }
    let mut cursor = cursor;
    let mut hops = 0u32;
    let mut consecutive_errors = 0u32;
    loop {
        hops += 1;
        if hops > SKILL_MAX_PAGES {
            tracing::warn!("ClawHub background fill hit hop cap ({} entries)", {
                REMOTE_CACHE
                    .lock()
                    .map(|c| c.skills.len())
                    .unwrap_or(0)
            });
            break;
        }
        match fetch_clawhub_pages(client, cursor.clone(), 6).await {
            Ok(page) => {
                consecutive_errors = 0;
                let complete = page.complete;
                let next = page.next_cursor.clone();
                let added = page.entries.len();
                {
                    let mut cache = REMOTE_CACHE.lock().expect("remote marketplace cache");
                    append_unique(&mut cache.skills, page.entries);
                    cache.skill_cursor = next.clone();
                    cache.skill_complete = complete;
                    cache.fetched_at = Some(Instant::now());
                    merge_sources(
                        &mut cache.sources,
                        &[source_status(
                            "clawhub",
                            "ClawHub Skills",
                            "clawhub",
                            CLAWHUB_SKILLS,
                            if complete { "ready" } else { "loading" },
                            None,
                        )],
                    );
                    tracing::info!(
                        "ClawHub fill hop={hops} added={added} total={} complete={complete}",
                        cache.skills.len()
                    );
                }
                if hops.is_multiple_of(2) {
                    persist_disk_cache();
                }
                if complete {
                    break;
                }
                cursor = next;
            }
            Err(err) => {
                consecutive_errors += 1;
                tracing::warn!("ClawHub background fill failed (try {consecutive_errors}): {err:#}");
                if consecutive_errors >= 3 {
                    break;
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}

async fn continue_fill_plugin(client: &reqwest::Client) {
    let already = REMOTE_CACHE
        .lock()
        .expect("remote marketplace cache")
        .plugin_complete;
    if already {
        return;
    }
    let plugin_fetch = fetch_plugin_marketplaces(client).await;
    if plugin_fetch.plugins.is_empty() {
        let mut cache = REMOTE_CACHE.lock().expect("remote marketplace cache");
        merge_sources(&mut cache.sources, &plugin_fetch.sources);
        return;
    }
    let mut cache = REMOTE_CACHE.lock().expect("remote marketplace cache");
    // Prefer larger plugin catalogs; never shrink.
    if plugin_fetch.plugins.len() >= cache.plugins.len() {
        cache.plugins = plugin_fetch.plugins;
    } else {
        append_unique(&mut cache.plugins, plugin_fetch.plugins);
    }
    cache.plugin_complete = true;
    merge_sources(&mut cache.sources, &plugin_fetch.sources);
}

fn append_unique(dst: &mut Vec<Value>, incoming: Vec<Value>) {
    let mut seen: std::collections::BTreeSet<String> = dst
        .iter()
        .filter_map(|entry| entry.get("id").and_then(Value::as_str).map(str::to_string))
        .collect();
    for entry in incoming {
        let id = entry
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if id.is_empty() || seen.insert(id) {
            dst.push(entry);
        }
    }
}

fn disk_cache_path() -> PathBuf {
    runtime_data_root().join("cache").join("marketplace-remote.json")
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn hydrate_disk_cache() {
    let mut cache = REMOTE_CACHE.lock().expect("remote marketplace cache");
    if cache.disk_loaded {
        return;
    }
    cache.disk_loaded = true;
    let Ok(raw) = fs::read(disk_cache_path()) else {
        return;
    };
    let Ok(parsed) = serde_json::from_slice::<Value>(&raw) else {
        return;
    };
    cache.saved_at_ms = parsed
        .get("savedAt")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    cache.mcp = parsed
        .get("mcp")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    cache.skills = parsed
        .get("skills")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    cache.plugins = parsed
        .get("plugins")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    cache.sources = parsed
        .get("sources")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    cache.mcp_complete = parsed
        .get("mcpComplete")
        .and_then(Value::as_bool)
        .unwrap_or(!cache.mcp.is_empty());
    cache.skill_complete = parsed
        .get("skillComplete")
        .and_then(Value::as_bool)
        .unwrap_or(!cache.skills.is_empty());
    cache.plugin_complete = parsed
        .get("pluginComplete")
        .and_then(Value::as_bool)
        .unwrap_or(!cache.plugins.is_empty());
    cache.mcp_cursor = parsed
        .get("mcpCursor")
        .and_then(Value::as_str)
        .map(str::to_string);
    cache.skill_cursor = parsed
        .get("skillCursor")
        .and_then(Value::as_str)
        .map(str::to_string);
    cache.fetched_at = Some(Instant::now());
    let resume_mcp = !cache.mcp_complete;
    let resume_skill = !cache.skill_complete;
    let resume_plugin = !cache.plugin_complete;
    drop(cache);
    // Engine boot: keep pulling toward the full catalog without waiting for UI.
    if resume_mcp {
        spawn_kind_fill(FillKind::Mcp, false);
    }
    if resume_skill {
        spawn_kind_fill(FillKind::Skill, false);
    }
    if resume_plugin {
        spawn_kind_fill(FillKind::Plugin, false);
    }
}

fn persist_disk_cache() {
    let snapshot = {
        let Ok(cache) = REMOTE_CACHE.lock() else {
            return;
        };
        json!({
            "savedAt": now_ms(),
            "mcp": cache.mcp,
            "skills": cache.skills,
            "plugins": cache.plugins,
            "sources": cache.sources,
            "mcpComplete": cache.mcp_complete,
            "skillComplete": cache.skill_complete,
            "pluginComplete": cache.plugin_complete,
            "mcpCursor": cache.mcp_cursor,
            "skillCursor": cache.skill_cursor,
        })
    };
    let path = disk_cache_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(bytes) = serde_json::to_vec(&snapshot) {
        let _ = fs::write(path, bytes);
    }
}

fn select_entries(
    filter: RemoteKindFilter,
    mcp: &[Value],
    skills: &[Value],
    plugins: &[Value],
) -> Vec<Value> {
    match filter {
        RemoteKindFilter::All => mcp
            .iter()
            .chain(skills.iter())
            .chain(plugins.iter())
            .cloned()
            .collect(),
        RemoteKindFilter::Mcp => mcp.to_vec(),
        RemoteKindFilter::Skill => skills.to_vec(),
        RemoteKindFilter::Plugin => plugins.to_vec(),
    }
}

fn empty_result_with_errors(errors: &[(&str, &str)]) -> RemoteMarketplaceResult {
    let sources = errors
        .iter()
        .map(|(id, message)| {
            source_status(id, id, "native", "", "error", Some(message))
        })
        .collect();
    RemoteMarketplaceResult {
        entries: Vec::new(),
        sources,
        incomplete: false,
        served_from_cache: false,
    }
}

fn source_status(
    id: &str,
    name: &str,
    source_type: &str,
    location: &str,
    status: &str,
    error: Option<&str>,
) -> Value {
    let mut value = json!({
        "id": id,
        "name": name,
        "type": source_type,
        "location": location,
        "enabled": true,
        "official": true,
        "status": status,
        "lastSyncedAt": chrono::Utc::now().timestamp_millis(),
    });
    if let Some(error) = error {
        value["error"] = json!(error.chars().take(240).collect::<String>());
    }
    value
}

async fn fetch_mcp_registry_pages(
    client: &reqwest::Client,
    start_cursor: Option<String>,
    max_pages: u32,
) -> anyhow::Result<PagedFetch> {
    let mut out = Vec::new();
    let mut cursor = start_cursor;
    let mut complete = false;
    for page in 0..max_pages {
        let mut url = reqwest::Url::parse(MCP_REGISTRY)?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("limit", &MCP_PAGE_LIMIT.to_string());
            query.append_pair("version", "latest");
            if let Some(cursor) = cursor.as_deref() {
                query.append_pair("cursor", cursor);
            }
        }
        let response = client
            .get(url)
            .timeout(Duration::from_secs(30))
            .send()
            .await?
            .error_for_status()?;
        let body: Value = response.json().await?;
        let servers = body
            .get("servers")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if servers.is_empty() {
            complete = true;
            cursor = None;
            break;
        }
        for item in servers {
            if let Some(entry) = normalize_mcp_server(&item) {
                out.push(entry);
            }
        }
        cursor = body
            .pointer("/metadata/nextCursor")
            .and_then(Value::as_str)
            .map(str::to_string)
            .filter(|value| !value.is_empty());
        if cursor.is_none() {
            complete = true;
            break;
        }
        if page + 1 == max_pages {
            tracing::debug!(
                "MCP registry warm/fill page cap {} reached; collected {}",
                max_pages,
                out.len()
            );
        }
    }
    Ok(PagedFetch {
        entries: out,
        next_cursor: cursor,
        complete,
    })
}

fn normalize_mcp_server(item: &Value) -> Option<Value> {
    let server = item.get("server").unwrap_or(item);
    let name = server.get("name").and_then(Value::as_str)?.trim();
    if name.is_empty() {
        return None;
    }
    let title = server
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or(name)
        .trim();
    let version = server
        .get("version")
        .and_then(Value::as_str)
        .unwrap_or("0.0.0");
    let description = server
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("");
    let install = mcp_install_from_server(server)?;
    let id = format!("mcp:{}", name.replace('/', "."));
    Some(json!({
        "id": id,
        "kind": "mcp",
        "name": title,
        "version": version,
        "description": description,
        "author": "",
        "category": "mcp",
        "tags": ["mcp", "official-registry"],
        "requestedEffects": ["network"],
        "sourceId": "mcp-registry",
        "installable": true,
        "install": install,
        "homepage": server.get("websiteUrl").cloned().unwrap_or(Value::Null),
        "provenance": {
            "registry": "mcp-registry",
            "digest": format!("mcp:{name}:{version}"),
            "serverName": name,
        }
    }))
}

fn mcp_install_from_server(server: &Value) -> Option<Value> {
    if let Some(install) = mcp_install_from_packages(server) {
        return Some(install);
    }
    mcp_install_from_remotes(server)
}

fn mcp_install_from_packages(server: &Value) -> Option<Value> {
    let packages = server.get("packages").and_then(Value::as_array)?;
    let package = packages
        .iter()
        .find(|pkg| {
        pkg.get("registryType")
            .and_then(Value::as_str)
            .map(|value| value.eq_ignore_ascii_case("npm"))
            .unwrap_or(false)
        })
        .or_else(|| packages.first())?;

    let identifier = package
        .get("identifier")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())?;
    let runtime_hint = package
        .get("runtimeHint")
        .and_then(Value::as_str)
        .unwrap_or("npx");
    let mut args = Vec::new();
    if let Some(runtime_args) = package.get("runtimeArguments").and_then(Value::as_array) {
        for arg in runtime_args {
            if let Some(value) = arg.get("value").and_then(Value::as_str) {
                args.push(json!(value));
            }
        }
    }
    if args.is_empty() && runtime_hint.eq_ignore_ascii_case("npx") {
        args.push(json!("-y"));
    }
    args.push(json!(identifier));
    if let Some(pkg_args) = package.get("packageArguments").and_then(Value::as_array) {
        for arg in pkg_args {
            if let Some(value) = arg.get("value").and_then(Value::as_str) {
                args.push(json!(value));
            }
        }
    }

    let mut env = serde_json::Map::new();
    if let Some(vars) = package
        .get("environmentVariables")
        .and_then(Value::as_array)
    {
        for var in vars {
            let key = var.get("name").and_then(Value::as_str).unwrap_or("");
            if key.is_empty() {
                continue;
            }
            if let Some(default) = var.get("default").and_then(Value::as_str) {
                env.insert(key.to_string(), json!(default));
            }
        }
    }

    let mut install = json!({
        "type": "mcp",
        "command": runtime_hint,
        "args": args,
    });
    if !env.is_empty() {
        install["env"] = Value::Object(env);
    }
    Some(install)
}

fn mcp_install_from_remotes(server: &Value) -> Option<Value> {
    let remotes = server.get("remotes").and_then(Value::as_array)?;
    let remote = remotes.iter().find(|item| {
        item.get("url")
            .and_then(Value::as_str)
            .map(|url| url.starts_with("https://") || url.starts_with("http://"))
            .unwrap_or(false)
    })?;
    let url = remote.get("url").and_then(Value::as_str)?.trim();
    if url.is_empty() {
        return None;
    }
    let transport = remote
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("streamable-http");
    Some(json!({
        "type": "mcp",
        "transport": transport,
        "url": url,
    }))
}

async fn fetch_clawhub_pages(
    client: &reqwest::Client,
    start_cursor: Option<String>,
    max_pages: u32,
) -> anyhow::Result<PagedFetch> {
    let mut out = Vec::new();
    let mut cursor = start_cursor;
    let mut complete = false;
    for page in 0..max_pages {
        let mut url = reqwest::Url::parse(CLAWHUB_SKILLS)?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("limit", &SKILL_PAGE_LIMIT.to_string());
            query.append_pair("sort", "downloads");
            query.append_pair("nonSuspiciousOnly", "true");
            if let Some(cursor) = cursor.as_deref() {
                query.append_pair("cursor", cursor);
            }
        }
        let response = client
            .get(url)
            .timeout(Duration::from_secs(30))
            .send()
            .await?
            .error_for_status()?;
    let body: Value = response.json().await?;
    let items = body
        .get("items")
        .or_else(|| body.get("skills"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
        if items.is_empty() {
            complete = true;
            cursor = None;
            break;
        }
        for item in &items {
            if let Some(entry) = normalize_clawhub_skill(item) {
                out.push(entry);
            }
        }
        cursor = body
            .get("nextCursor")
            .and_then(Value::as_str)
            .map(str::to_string)
            .filter(|value| !value.is_empty());
        if cursor.is_none() {
            complete = true;
            break;
        }
        if page + 1 == max_pages {
            tracing::debug!(
                "ClawHub warm/fill page cap {} reached; collected {}",
                max_pages,
                out.len()
            );
        }
    }
    Ok(PagedFetch {
        entries: out,
        next_cursor: cursor,
        complete,
    })
}

fn normalize_clawhub_skill(item: &Value) -> Option<Value> {
    let slug = item
        .get("slug")
        .or_else(|| item.get("id"))
        .and_then(Value::as_str)?
        .trim();
    if slug.is_empty() {
        return None;
    }
    let name = item
        .get("displayName")
        .or_else(|| item.get("name"))
        .and_then(Value::as_str)
        .unwrap_or(slug);
    let version = item
        .pointer("/latestVersion/version")
        .or_else(|| item.pointer("/tags/latest"))
        .or_else(|| item.get("version"))
        .and_then(Value::as_str)
        .unwrap_or("0.0.0");
    let description = item
        .get("summary")
        .or_else(|| item.get("description"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let id = format!("skill:{slug}");
    Some(json!({
        "id": id,
        "kind": "skill",
        "name": name,
        "version": version,
        "description": description,
        "author": item.pointer("/owner/handle").or_else(|| item.get("owner")).and_then(Value::as_str).unwrap_or(""),
        "category": "skill",
        "tags": ["skill", "clawhub"],
        "requestedEffects": ["network"],
        "sourceId": "clawhub",
        "installable": true,
        "install": {
            "type": "skill",
            "url": format!("https://clawhub.ai/api/v1/download?slug={slug}"),
            "slug": slug,
        },
        "homepage": format!("https://clawhub.ai/skills/{slug}"),
        "provenance": {
            "registry": "clawhub",
            "digest": format!("clawhub:{slug}:{version}"),
        }
    }))
}

async fn fetch_plugin_marketplaces(client: &reqwest::Client) -> PluginMarketplaceFetch {
    let mut plugins = Vec::new();
    let mut sources = Vec::new();

    match fetch_claude_marketplace_with_mirrors(
        client,
        PLUGIN_OFFICIAL_MARKETPLACE,
        "claude-plugins-official",
    )
    .await
    {
        Ok(entries) => {
            sources.push(source_status(
                "claude-plugins-official",
                "Official Plugins",
                "claude",
                PLUGIN_OFFICIAL_MARKETPLACE,
                "ready",
                None,
            ));
            plugins.extend(entries);
        }
        Err(err) => {
            tracing::warn!("official plugin marketplace unavailable: {err:#}");
            sources.push(source_status(
                "claude-plugins-official",
                "Official Plugins",
                "claude",
                PLUGIN_OFFICIAL_MARKETPLACE,
                "error",
                Some(&err.to_string()),
            ));
        }
    }

    match fetch_claude_marketplace_with_mirrors(
        client,
        PLUGIN_COMMUNITY_MARKETPLACE,
        "claude-plugins-community",
    )
    .await
    {
        Ok(entries) => {
            sources.push(source_status(
                "claude-plugins-community",
                "Community Plugins",
                "claude",
                PLUGIN_COMMUNITY_MARKETPLACE,
                "ready",
                None,
            ));
            plugins.extend(entries);
        }
        Err(err) => {
            tracing::warn!("community plugin marketplace unavailable: {err:#}");
            sources.push(source_status(
                "claude-plugins-community",
                "Community Plugins",
                "claude",
                PLUGIN_COMMUNITY_MARKETPLACE,
                "error",
                Some(&err.to_string()),
            ));
        }
    }

    PluginMarketplaceFetch { plugins, sources }
}

#[derive(Default)]
struct PagedFetch {
    entries: Vec<Value>,
    next_cursor: Option<String>,
    complete: bool,
}

#[derive(Default)]
struct PluginMarketplaceFetch {
    plugins: Vec<Value>,
    sources: Vec<Value>,
}

/// Prefer the canonical GitHub raw URL, then CDN mirrors when GitHub is blocked/slow.
fn github_raw_mirror_urls(raw_url: &str) -> Vec<String> {
    let mut urls = vec![raw_url.to_string()];
    let Some(rest) = raw_url.strip_prefix("https://raw.githubusercontent.com/") else {
        return urls;
    };
    let mut parts = rest.splitn(4, '/');
    let (Some(owner), Some(repo), Some(branch), Some(path)) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return urls;
    };
    urls.push(format!(
        "https://cdn.jsdelivr.net/gh/{owner}/{repo}@{branch}/{path}"
    ));
    urls.push(format!(
        "https://fastly.jsdelivr.net/gh/{owner}/{repo}@{branch}/{path}"
    ));
    urls.push(format!(
        "https://ghproxy.net/https://raw.githubusercontent.com/{rest}"
    ));
    urls
}

async fn fetch_claude_marketplace_with_mirrors(
    client: &reqwest::Client,
    primary_url: &str,
    source_id: &str,
) -> anyhow::Result<Vec<Value>> {
    let mut last_err = None;
    for url in github_raw_mirror_urls(primary_url) {
        match fetch_claude_marketplace(client, &url, source_id).await {
            Ok(entries) => {
                if url != primary_url {
                    tracing::info!(
                        "plugin marketplace {source_id} loaded via mirror: {url}"
                    );
                }
                return Ok(entries);
            }
            Err(err) => {
                tracing::warn!("plugin marketplace {source_id} fetch failed ({url}): {err:#}");
                last_err = Some(err);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("no marketplace mirrors configured")))
}

async fn fetch_claude_marketplace(
    client: &reqwest::Client,
    url: &str,
    source_id: &str,
) -> anyhow::Result<Vec<Value>> {
    let response = client
        .get(url)
        .timeout(Duration::from_secs(12))
        .send()
        .await?
        .error_for_status()?;
    let body: Value = response.json().await?;
    let plugins = body
        .get("plugins")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(plugins
        .iter()
        .filter_map(|plugin| normalize_claude_plugin(plugin, source_id))
        .collect())
}

fn normalize_claude_plugin(plugin: &Value, source_id: &str) -> Option<Value> {
    let name = plugin.get("name").and_then(Value::as_str)?.trim();
    if name.is_empty() {
        return None;
    }
    let description = plugin
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("");
    let author = plugin
        .pointer("/author/name")
        .and_then(Value::as_str)
        .or_else(|| plugin.get("author").and_then(Value::as_str))
        .unwrap_or("");
    let category = plugin
        .get("category")
        .and_then(Value::as_str)
        .unwrap_or("plugin");
    let source = plugin.get("source").cloned().unwrap_or(Value::Null);
    let version = source
        .get("ref")
        .or_else(|| source.get("sha"))
        .and_then(Value::as_str)
        .unwrap_or("latest");
    let install = if let Some(url) = source.get("url").and_then(Value::as_str) {
        let mut install = json!({
            "type": "git",
            "url": url,
            "sourceKind": source.get("source").and_then(Value::as_str).unwrap_or("git"),
        });
        if let Some(path) = source.get("path").and_then(Value::as_str) {
            install["path"] = json!(path);
        }
        if let Some(ref_name) = source.get("ref").and_then(Value::as_str) {
            install["ref"] = json!(ref_name);
        }
        if let Some(sha) = source.get("sha").and_then(Value::as_str) {
            install["sha"] = json!(sha);
        }
        install
    } else {
        json!({ "type": "marketplace", "id": name })
    };
    let id = format!("plugin:{source_id}:{name}");
    Some(json!({
        "id": id,
        "kind": "plugin",
        "name": plugin.get("displayName").and_then(Value::as_str).unwrap_or(name),
        "version": version,
        "description": description,
        "author": author,
        "category": category,
        "tags": ["plugin", source_id],
        "requestedEffects": ["network"],
        "sourceId": source_id,
        "installable": true,
        "install": install,
        "homepage": plugin.get("homepage").cloned().unwrap_or(Value::Null),
        "provenance": {
            "registry": source_id,
            "digest": format!("{source_id}:{name}:{version}"),
        }
    }))
}
