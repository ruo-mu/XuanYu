//! 渐进工具面：每轮只暴露 core + meta 的完整 schema，其余经 load_tools 激活。

use crate::ToolCatalogEntry;
use std::collections::BTreeSet;

/** 主 Agent 每轮默认预挂：高频编码工具。 */
pub const AGENT_CORE_PRELOAD_TOOLS: &[&str] = &[
    "file_read",
    "file_write",
    "file_edit",
    "list_files",
    "glob",
    "search",
    "exec",
    "submit_task", // product canonical; tool catalog alias: agent
];

/** 元工具：始终在表面，无需 load_tools。 */
pub const META_TOOL_NAMES: &[&str] = &[
    "load_tools",
    "search_memory",
    "load_memories",
    "activate_skill",
    "ask_user",
];

const DEFAULT_CATALOG_DESC_CHARS: usize = 100;

pub fn is_meta_tool_name(name: &str) -> bool {
    META_TOOL_NAMES.contains(&name)
}

/** 每轮开始的激活集 = core + meta + 可选额外预挂（如目标活跃时的 update_goal）。 */
pub fn initial_active_tool_names(extra_preload: &[&str]) -> BTreeSet<String> {
    let mut active = BTreeSet::new();
    for name in AGENT_CORE_PRELOAD_TOOLS {
        active.insert((*name).to_string());
    }
    for name in META_TOOL_NAMES {
        active.insert((*name).to_string());
    }
    for name in extra_preload {
        let trimmed = name.trim();
        if !trimmed.is_empty() {
            active.insert(trimmed.to_string());
        }
    }
    active
}

/** 把未激活工具编成 prompt 目录，提示先 load_tools。 */
pub fn format_deferred_tool_catalog_for_prompt(
    catalog: &[ToolCatalogEntry],
    options: DeferredCatalogOptions<'_>,
) -> String {
    let max_desc = options.max_description_chars.unwrap_or(DEFAULT_CATALOG_DESC_CHARS);
    let core = options.core_active_names;
    let meta = options.enabled_meta_names;
    let meta_line = if meta.is_empty() {
        "No meta tools are currently enabled.".to_string()
    } else {
        format!(
            "Enabled meta tools (no load_tools needed): {}.",
            meta.join(", ")
        )
    };

    let exclude: BTreeSet<&str> = core
        .iter()
        .copied()
        .chain(META_TOOL_NAMES.iter().copied())
        .collect();

    let mut deferred = catalog
        .iter()
        .filter(|entry| !exclude.contains(entry.name.as_str()) && !is_meta_tool_name(&entry.name))
        .collect::<Vec<_>>();
    deferred.sort_by(|a, b| {
        let ca = a.category.as_deref().unwrap_or("other");
        let cb = b.category.as_deref().unwrap_or("other");
        ca.cmp(cb).then_with(|| a.name.cmp(&b.name))
    });

    let mut lines = vec![
        "## Tool catalog (deferred schemas)".to_string(),
        format!(
            "Already active with full schemas each turn: {}.",
            if core.is_empty() {
                "(none)".to_string()
            } else {
                core.join(", ")
            }
        ),
        meta_line,
        "For any other tool below: call load_tools([\"exact_name\"]) first, then invoke it on the next model iteration."
            .to_string(),
    ];

    if deferred.is_empty() {
        lines.push(String::new());
        lines.push("(No deferred tools in the current enabled set.)".to_string());
        return lines.join("\n");
    }

    let mut last_category = String::new();
    for entry in deferred {
        let category = entry.category.as_deref().unwrap_or("other");
        if category != last_category {
            lines.push(String::new());
            lines.push(format!("### {category}"));
            last_category = category.to_string();
        }
        let flags = [
            entry.is_write.then_some("write"),
            entry.is_execute.then_some("execute"),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(", ");
        let desc = truncate_one_line(&entry.description, max_desc);
        if flags.is_empty() {
            lines.push(format!("- {}: {desc}", entry.name));
        } else {
            lines.push(format!("- {}: {desc} ({flags})", entry.name));
        }
    }
    lines.join("\n")
}

pub struct DeferredCatalogOptions<'a> {
    pub core_active_names: &'a [&'a str],
    pub enabled_meta_names: &'a [&'a str],
    pub max_description_chars: Option<usize>,
}

pub fn should_report_meta_load_as_error(
    loaded: &[String],
    already_active: &[String],
    rejected: &[serde_json::Value],
) -> bool {
    if !loaded.is_empty() || !already_active.is_empty() {
        return false;
    }
    !rejected.is_empty()
}

fn truncate_one_line(text: &str, max_chars: usize) -> String {
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= max_chars {
        return one_line;
    }
    let keep = max_chars.saturating_sub(1).max(1);
    let truncated: String = one_line.chars().take(keep).collect();
    format!("{truncated}…")
}
