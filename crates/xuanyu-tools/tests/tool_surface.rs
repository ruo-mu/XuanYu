use xuanyu_tools::{
    format_deferred_tool_catalog_for_prompt, initial_active_tool_names, DeferredCatalogOptions,
    ToolCatalogEntry, ToolSource, AGENT_CORE_PRELOAD_TOOLS, META_TOOL_NAMES,
};

#[test]
fn initial_surface_includes_core_and_meta() {
    let active = initial_active_tool_names(&["update_goal"]);
    assert!(active.contains("file_read"));
    assert!(active.contains("exec"));
    assert!(active.contains("submit_task"));
    assert!(active.contains("load_tools"));
    assert!(active.contains("ask_user"));
    assert!(active.contains("update_goal"));
    assert!(!active.contains("git_status"));
    assert!(!active.contains("search_tools"));
}

#[test]
fn deferred_catalog_excludes_core_and_meta() {
    let catalog = vec![
        entry("file_read", "filesystem", "Read a file"),
        entry("load_tools", "meta", "Load tools"),
        entry("git_status", "git", "Show git status"),
        entry("search_tools", "discovery", "Search tools"),
    ];
    let text = format_deferred_tool_catalog_for_prompt(
        &catalog,
        DeferredCatalogOptions {
            core_active_names: AGENT_CORE_PRELOAD_TOOLS,
            enabled_meta_names: META_TOOL_NAMES,
            max_description_chars: None,
        },
    );
    assert!(text.contains("git_status"));
    assert!(text.contains("search_tools"));
    assert!(!text.contains("- file_read:"));
    assert!(!text.contains("- load_tools:"));
    assert!(text.contains("Already active with full schemas each turn:"));
}

fn entry(name: &str, category: &str, description: &str) -> ToolCatalogEntry {
    ToolCatalogEntry {
        name: name.to_string(),
        description: description.to_string(),
        category: Some(category.to_string()),
        source: ToolSource::Builtin,
        is_write: false,
        is_execute: false,
        is_concurrent: true,
        aliases: Vec::new(),
    }
}
