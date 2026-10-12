use crate::context_assembler::{policy_bool, policy_auto_sections, PrepareTurnPlan};
use crate::context_surface::ContextSurface;
use crate::turn_memory_cache::TurnMemoryCache;
use serde_json::Value;

/// Turn preparation state for assembling runtime context.
#[derive(Debug, Default)]
pub struct PrepareTurnContextState {
    pub runtime_system_prompt: Option<String>,
    pub turn_user_message: String,
    pub turn_intent: String,
    pub orchestrator_only: bool,
    pub extract_memories_on_compact: bool,
    pub seed_key_facts_from_static: bool,
    pub memory_cache: TurnMemoryCache,
    pub context_surface: ContextSurface,
}

pub struct PrepareTurnContextInput<'a> {
    pub plan: &'a PrepareTurnPlan,
    pub policy: &'a Value,
    pub user_message: &'a str,
    pub intent: &'a str,
    pub orchestrator_only: bool,
}

impl PrepareTurnContextState {
    pub fn prepare_turn_context(&mut self, input: PrepareTurnContextInput<'_>) {
        self.runtime_system_prompt = None;
        self.turn_user_message = input.user_message.to_string();
        self.turn_intent = input.intent.to_string();
        self.orchestrator_only = input.orchestrator_only;
        self.context_surface
            .reset_for_turn(input.policy, input.intent);
        self.memory_cache.invalidate();
        self.extract_memories_on_compact = policy_bool(input.policy, "injectRetrievedMemory")
            || policy_bool(input.policy, "injectMemoryProfile")
            || policy_auto_sections(input.policy)
                .iter()
                .any(|section| section == "memory");
        self.seed_key_facts_from_static = input
            .policy
            .get("seedKeyFactsFromStaticContext")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let _ = input.plan;
    }

    pub fn refresh_after_context_sections_loaded(&mut self, sections: &[String]) {
        self.memory_cache
            .refresh_after_context_sections_loaded(sections);
    }
}
