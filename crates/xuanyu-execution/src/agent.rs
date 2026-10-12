use serde_json::Value;

use crate::context_assembler::PrepareTurnPlan;
use crate::prepare_turn_context::{PrepareTurnContextInput, PrepareTurnContextState};
use crate::tool_call_loop::ToolCallLoop;
use crate::turn_memory_cache::TurnMemoryCache;
use crate::context_surface::ContextSurface;

/// Turn-scoped agent state container.
pub struct Agent {
    pub turn_context: PrepareTurnContextState,
    pub tool_loop: ToolCallLoop,
    pub orchestrator_only: bool,
}

impl Agent {
    pub fn new() -> Self {
        Self {
            turn_context: PrepareTurnContextState::default(),
            tool_loop: ToolCallLoop::new(),
            orchestrator_only: false,
        }
    }

    pub fn prepare_turn_context(
        &mut self,
        plan: &PrepareTurnPlan,
        policy: &Value,
        user_message: &str,
        intent: &str,
        orchestrator_only: bool,
    ) {
        self.orchestrator_only = orchestrator_only;
        self.turn_context.prepare_turn_context(PrepareTurnContextInput {
            plan,
            policy,
            user_message,
            intent,
            orchestrator_only,
        });
        self.tool_loop.reset();
    }

    pub fn context_surface(&self) -> &ContextSurface {
        &self.turn_context.context_surface
    }

    pub fn context_surface_mut(&mut self) -> &mut ContextSurface {
        &mut self.turn_context.context_surface
    }

    pub fn memory_cache(&self) -> &TurnMemoryCache {
        &self.turn_context.memory_cache
    }

    pub fn memory_cache_mut(&mut self) -> &mut TurnMemoryCache {
        &mut self.turn_context.memory_cache
    }

    pub fn tool_loop_mut(&mut self) -> &mut ToolCallLoop {
        &mut self.tool_loop
    }

    pub fn extract_memories_on_compact(&self) -> bool {
        self.turn_context.extract_memories_on_compact
    }

    pub fn seed_key_facts_from_static(&self) -> bool {
        self.turn_context.seed_key_facts_from_static
    }
}

impl Default for Agent {
    fn default() -> Self {
        Self::new()
    }
}
