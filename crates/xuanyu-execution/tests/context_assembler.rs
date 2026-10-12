use serde_json::json;
use xuanyu_execution::{policy_bool, prepare_turn, PrepareTurnInput};

#[test]
fn prepare_turn_lazy_work_intent_activates_memory_sections() {
    let plan = prepare_turn(PrepareTurnInput {
        config: json!({}),
        intent: "task".to_string(),
        lazy_override: None,
    });
    assert!(plan.auto_retrieve_memory);
    assert!(plan.inject_key_facts);
    assert!(policy_bool(&plan.context_policy, "injectProjectInstructions"));
    assert!(plan
        .active_sections
        .iter()
        .any(|section| section == "project_instructions"));
    assert!(plan.active_sections.iter().any(|section| section == "memory"));
}

#[test]
fn prepare_turn_coding_is_work_intent_like_node_gateway() {
    let plan = prepare_turn(PrepareTurnInput {
        config: json!({}),
        intent: "coding".to_string(),
        lazy_override: None,
    });
    assert!(plan.auto_retrieve_memory);
    assert!(policy_bool(&plan.context_policy, "injectMemoryProfile"));
    assert!(policy_bool(&plan.context_policy, "injectProjectInstructions"));
    assert!(!policy_bool(&plan.context_policy, "injectDelegation"));
    assert!(plan
        .active_sections
        .iter()
        .any(|section| section == "project_instructions"));
}

#[test]
fn prepare_turn_chat_stays_light_under_lazy() {
    let plan = prepare_turn(PrepareTurnInput {
        config: json!({}),
        intent: "chat".to_string(),
        lazy_override: None,
    });
    assert!(!plan.auto_retrieve_memory);
    assert!(!policy_bool(&plan.context_policy, "injectProjectInstructions"));
    assert!(!plan
        .active_sections
        .iter()
        .any(|section| section == "project_instructions" || section == "memory"));
}
