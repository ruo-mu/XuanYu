use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LazyContextOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_load_policy: Option<Value>,
    #[serde(default)]
    pub active_sections: Vec<String>,
    #[serde(default)]
    pub active_tool_names: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_intent: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refresh_sections: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct PrepareTurnInput {
    pub config: Value,
    pub intent: String,
    pub lazy_override: Option<LazyContextOverride>,
}

#[derive(Debug, Clone)]
pub struct PrepareTurnPlan {
    pub load_mode: &'static str,
    pub context_policy: Value,
    pub auto_retrieve_memory: bool,
    pub inject_key_facts: bool,
    pub context_surface: Value,
    pub active_sections: Vec<String>,
}

/// REFACTOR_PLAN §11.6 — lazy context policy + section surface before model call.
pub fn prepare_turn(input: PrepareTurnInput) -> PrepareTurnPlan {
    let load_mode = context_load_mode_from_config(&input.config);
    let mut context_policy = context_load_policy(&input.config, load_mode, &input.intent);
    if let Some(override_state) = input.lazy_override.as_ref() {
        context_policy = merge_lazy_context_override(&context_policy, override_state);
    }
    PrepareTurnPlan {
        load_mode,
        auto_retrieve_memory: should_auto_retrieve_memory_for_policy(&context_policy),
        inject_key_facts: should_inject_key_facts(&context_policy),
        context_surface: context_surface_snapshot(&context_policy),
        active_sections: active_context_sections(&context_policy),
        context_policy,
    }
}

pub fn context_load_mode_from_config(config: &Value) -> &'static str {
    match config
        .get("contextManagement")
        .and_then(Value::as_object)
        .and_then(|context| context.get("loadMode"))
        .and_then(Value::as_str)
        .unwrap_or("lazy")
    {
        "balanced" => "balanced",
        "eager" => "eager",
        _ => "lazy",
    }
}

pub fn context_load_policy(config: &Value, load_mode: &str, intent: &str) -> Value {
    let mode = match load_mode {
        "balanced" | "eager" | "lazy" => load_mode,
        _ => "lazy",
    };
    // Match Node gateway: workIntent = intent !== 'chat' (coding/task/review/debug all work).
    let normalized = intent
        .strip_prefix("desktop-")
        .unwrap_or(intent)
        .trim()
        .to_ascii_lowercase();
    let work_intent = !normalized.is_empty() && normalized != "chat";
    let eager = mode == "eager";
    let balanced = mode == "balanced";
    // Reference resolveContextLoadPolicy: work turns inject memory/project/keyfacts.
    // loadMode eager/balanced can still force richer injects for chat.
    let inject_memory_profile = work_intent || eager || balanced;
    let inject_retrieved_memory =
        (work_intent || eager || balanced) && memory_auto_inject_enabled(config);
    let inject_project_instructions = work_intent || eager || balanced;
    let inject_project_context = work_intent || eager || balanced;
    let inject_custom_instructions = work_intent || eager || balanced;
    let inject_delegation = matches!(normalized.as_str(), "task" | "review" | "debug");
    let inject_key_facts = work_intent || eager || balanced;
    let seed_key_facts = work_intent || eager || balanced;
    // Skills live in the base prompt in the Node gateway; keep flag for override/compat only.
    let inject_skills = false;
    let mut auto_activate = Vec::new();

    if work_intent || inject_retrieved_memory {
        push_unique_section(&mut auto_activate, "memory");
    }
    if inject_memory_profile && (eager || balanced) {
        push_unique_section(&mut auto_activate, "memory_profile");
    }
    if inject_project_instructions {
        push_unique_section(&mut auto_activate, "project_instructions");
    }

    json!({
        "source": "rust-core",
        "estimated": true,
        "mode": mode,
        "intent": intent,
        "orchestratorOnly": false,
        "injectMemoryProfile": inject_memory_profile,
        "injectRetrievedMemory": inject_retrieved_memory,
        "injectProjectInstructions": inject_project_instructions,
        "injectSkills": inject_skills,
        "injectDelegation": inject_delegation,
        "injectProjectContext": inject_project_context,
        "injectCustomInstructions": inject_custom_instructions,
        "seedKeyFactsFromStaticContext": seed_key_facts,
        "injectKeyFacts": inject_key_facts,
        "memoryAutoInjectMinScore": memory_auto_inject_min_score(config),
        "memoryAutoInjectLimit": memory_auto_inject_limit(config),
        "memoryIncludeCrossSession": memory_auto_inject_include_cross_session(config),
        "autoActivateSections": auto_activate,
        "availableSections": context_available_sections()
    })
}

pub fn context_available_sections() -> Vec<Value> {
    // Align with foundation ContextSectionName: memory | memory_profile | project_instructions.
    vec![
        json!({
            "id": "memory",
            "label": "Retrieved long-term memories",
            "tool": "load_memories"
        }),
        json!({
            "id": "memory_profile",
            "label": "Explicit memory profiles",
            "tool": "load_memories"
        }),
        json!({
            "id": "project_instructions",
            "label": "Project instruction files",
            "tool": "load_memories"
        }),
    ]
}

pub fn policy_bool(policy: &Value, key: &str) -> bool {
    policy.get(key).and_then(Value::as_bool).unwrap_or(false)
}

pub fn should_auto_retrieve_memory_for_policy(policy: &Value) -> bool {
    policy_bool(policy, "injectRetrievedMemory")
        || policy_auto_sections(policy)
            .iter()
            .any(|section| section == "memory")
}

pub fn should_inject_key_facts(policy: &Value) -> bool {
    policy_bool(policy, "injectKeyFacts")
}

pub fn should_refresh_memory_profile(policy: &Value) -> bool {
    policy_bool(policy, "injectMemoryProfile")
        || policy_auto_sections(policy)
            .iter()
            .any(|section| section == "memory_profile")
}

pub fn should_inject_context_section(policy: &Value, section: &str) -> bool {
    if policy_refresh_sections(policy)
        .iter()
        .any(|item| item == section)
    {
        return true;
    }
    context_section_injection_gate(policy, section) != "none"
}

pub fn context_section_activation_reason(policy: &Value, section: &str) -> &'static str {
    match context_section_injection_gate(policy, section) {
        "both" => "active by policy and context surface",
        "policy" => "active by context load policy",
        "surface" => "active by lazy context surface",
        _ => "available through load_memories",
    }
}

pub fn context_surface_snapshot(policy: &Value) -> Value {
    let active_sections = policy_auto_sections(policy);
    let injection_gates = [
        "memory",
        "memory_profile",
        "project_instructions",
        "key_facts",
        "delegation",
    ]
    .into_iter()
    .map(|section| {
        json!({
            "section": section,
            "gate": context_section_injection_gate(policy, section)
        })
    })
    .collect::<Vec<_>>();
    json!({
        "activeSections": active_sections,
        "availableSections": context_available_sections(),
        "injectionGates": injection_gates
    })
}

pub fn context_section_injection_gate(policy: &Value, section: &str) -> &'static str {
    let active_sections = policy_auto_sections(policy);
    let on_surface = active_sections.iter().any(|item| item == section);
    let from_policy = match section {
        "memory" => policy_bool(policy, "injectRetrievedMemory"),
        "memory_profile" => policy_bool(policy, "injectMemoryProfile"),
        "project_instructions" => policy_bool(policy, "injectProjectInstructions"),
        "key_facts" => policy_bool(policy, "injectKeyFacts"),
        "delegation" => policy_bool(policy, "injectDelegation"),
        _ => false,
    };
    match (from_policy, on_surface) {
        (true, true) => "both",
        (true, false) => "policy",
        (false, true) => "surface",
        (false, false) => "none",
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

pub fn active_context_sections(policy: &Value) -> Vec<String> {
    let mut sections = vec!["conversation".to_string(), "toolCatalog".to_string()];
    for section in policy_auto_sections(policy) {
        if !sections.iter().any(|item| item == &section) {
            sections.push(section);
        }
    }
    for (flag, section) in [
        ("injectKeyFacts", "key_facts"),
        ("injectDelegation", "delegation"),
    ] {
        if policy_bool(policy, flag) && !sections.iter().any(|item| item == section) {
            sections.push(section.to_string());
        }
    }
    sections
}

pub fn policy_refresh_sections(policy: &Value) -> Vec<String> {
    policy
        .get("refreshSections")
        .or_else(|| policy.get("refresh_sections"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

pub fn mark_lazy_sections_for_refresh(override_state: &mut LazyContextOverride) {
    for section in ["memory_profile", "project_instructions"] {
        if !override_state
            .active_sections
            .iter()
            .any(|item| item == section)
        {
            override_state.active_sections.push(section.to_string());
        }
        if !override_state
            .refresh_sections
            .iter()
            .any(|item| item == section)
        {
            override_state.refresh_sections.push(section.to_string());
        }
    }
}

pub fn strip_refresh_sections(policy: &Value) -> Value {
    let mut cleaned = policy.clone();
    if let Some(object) = cleaned.as_object_mut() {
        object.remove("refreshSections");
        object.remove("refresh_sections");
    }
    cleaned
}

pub fn merge_lazy_context_override(policy: &Value, override_state: &LazyContextOverride) -> Value {
    let mut merged = policy.clone();
    let Some(object) = merged.as_object_mut() else {
        return merged;
    };
    if let Some(extra) = override_state.context_load_policy.as_ref() {
        for key in [
            "injectRetrievedMemory",
            "injectMemoryProfile",
            "injectProjectInstructions",
            "injectDelegation",
            "injectProjectContext",
            "injectCustomInstructions",
            "injectKeyFacts",
            "mode",
            "intent",
            "memoryAutoInjectMinScore",
            "memoryAutoInjectLimit",
        ] {
            if let Some(value) = extra.get(key) {
                object.insert(key.to_string(), value.clone());
            }
        }
    }
    if !override_state.active_sections.is_empty() {
        object.insert(
            "autoActivateSections".to_string(),
            json!(override_state.active_sections),
        );
    }
    if !override_state.refresh_sections.is_empty() {
        object.insert(
            "refreshSections".to_string(),
            json!(override_state.refresh_sections),
        );
    }
    if let Some(intent) = override_state.turn_intent.as_deref() {
        object.insert("intent".to_string(), json!(intent));
    }
    merged
}

fn push_unique_section(sections: &mut Vec<&'static str>, section: &'static str) {
    if !sections.contains(&section) {
        sections.push(section);
    }
}

pub fn memory_auto_inject_enabled(config: &Value) -> bool {
    let memory_disabled = config
        .get("memory")
        .and_then(Value::as_object)
        .and_then(|memory| memory.get("autoInject"))
        .and_then(Value::as_bool)
        .is_some_and(|enabled| !enabled);
    let context_disabled = config
        .get("contextManagement")
        .and_then(Value::as_object)
        .and_then(|context| context.get("injectRetrievedMemory"))
        .and_then(Value::as_bool)
        .is_some_and(|enabled| !enabled);
    !memory_disabled && !context_disabled
}

pub fn memory_auto_inject_min_score(config: &Value) -> f32 {
    config
        .get("contextManagement")
        .and_then(Value::as_object)
        .and_then(|context| context.get("memoryAutoInjectMinScore"))
        .and_then(Value::as_f64)
        .map(|value| value.clamp(0.0, 1.0) as f32)
        .unwrap_or(0.35)
}

pub fn memory_auto_inject_limit(config: &Value) -> usize {
    config
        .get("contextManagement")
        .and_then(Value::as_object)
        .and_then(|context| {
            context
                .get("memoryAutoInjectLimit")
                .or_else(|| context.get("memoryInjectLimit"))
        })
        .and_then(Value::as_u64)
        .map(|value| value.clamp(1, 20) as usize)
        .unwrap_or(8)
}

pub fn memory_auto_inject_include_cross_session(config: &Value) -> bool {
    config
        .get("contextManagement")
        .and_then(Value::as_object)
        .and_then(|context| {
            context
                .get("memoryIncludeCrossSession")
                .or_else(|| context.get("memoryAutoInjectCrossSession"))
        })
        .and_then(Value::as_bool)
        .unwrap_or(false)
}
