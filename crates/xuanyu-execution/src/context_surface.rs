use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

/// Mirrors foundation `ContextSectionName` / Node `contextSurface.ts`.
pub const CONTEXT_SECTION_NAMES: [&str; 3] =
    ["memory", "memory_profile", "project_instructions"];

pub fn is_context_section_name(value: &str) -> bool {
    CONTEXT_SECTION_NAMES.contains(&value)
}

pub fn section_label(section: &str) -> &'static str {
    match section {
        "memory" => "Retrieved long-term memories",
        "memory_profile" => "Explicit memory profiles (soul/user/project/...)",
        "project_instructions" => "Project instruction files (AGENTS.md / XUANYU.md)",
        _ => "Unknown context section",
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextLoadResult {
    pub loaded: Vec<String>,
    pub already_active: Vec<String>,
    pub rejected: Vec<ContextSectionRejected>,
    pub active_sections: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextSectionRejected {
    pub section: String,
    pub reason: String,
}

#[derive(Debug, Default)]
pub struct ContextSurface {
    active: BTreeSet<String>,
}

impl ContextSurface {
    pub fn new() -> Self {
        Self::default()
    }

    /// `ContextSurface.resetForTurn` — intent resolved upstream in policy.
    pub fn reset_for_turn(&mut self, policy: &Value, _intent: &str) {
        self.active.clear();
        for section in policy_auto_sections(policy) {
            if is_context_section_name(&section) {
                self.active.insert(section);
            }
        }
    }

    /// Soft lazy: explicit `load_memories` promise; does not re-check policy gates.
    pub fn load(&mut self, sections: &[String]) -> ContextLoadResult {
        let mut loaded = Vec::new();
        let mut already_active = Vec::new();
        let mut rejected = Vec::new();
        for raw in sections {
            if !is_context_section_name(raw) {
                rejected.push(ContextSectionRejected {
                    section: raw.clone(),
                    reason: "unknown section".to_string(),
                });
                continue;
            }
            if self.active.contains(raw) {
                already_active.push(raw.clone());
                continue;
            }
            self.active.insert(raw.clone());
            loaded.push(raw.clone());
        }
        ContextLoadResult {
            loaded,
            already_active,
            rejected,
            active_sections: self.get_active_sections(),
        }
    }

    pub fn restore_active_sections(&mut self, sections: &[String]) {
        self.active.clear();
        for section in sections {
            if is_context_section_name(section) {
                self.active.insert(section.clone());
            }
        }
    }

    pub fn is_active(&self, section: &str) -> bool {
        self.active.contains(section)
    }

    pub fn get_active_sections(&self) -> Vec<String> {
        let mut sections = self.active.iter().cloned().collect::<Vec<_>>();
        sections.sort();
        sections
    }
}

pub fn policy_auto_sections(policy: &Value) -> Vec<String> {
    policy
        .get("autoActivateSections")
        .and_then(Value::as_array)
        .map(|sections| {
            sections
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

pub fn sections_need_memory_cache_invalidate(sections: &[String]) -> bool {
    sections.iter().any(|section| {
        section == "memory"
            || section == "memory_profile"
            || section == "project_instructions"
    })
}
