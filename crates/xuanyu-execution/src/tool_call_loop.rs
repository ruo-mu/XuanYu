use serde_json::Value;

pub use crate::tool_loop::{LoopIntervention, ToolLoopGuard, TurnFingerprint};

/// Tool-call iteration loop guard — legacy `Agent.run()` stagnation layer.
pub struct ToolCallLoop {
    guard: ToolLoopGuard,
}

impl ToolCallLoop {
    pub fn new() -> Self {
        Self {
            guard: ToolLoopGuard::new(),
        }
    }

    pub fn reset(&mut self) {
        self.guard = ToolLoopGuard::new();
    }

    pub fn guard(&self) -> &ToolLoopGuard {
        &self.guard
    }

    pub fn guard_mut(&mut self) -> &mut ToolLoopGuard {
        &mut self.guard
    }

    pub fn record_iteration(
        &mut self,
        tool_calls: &[(&str, &Value)],
        output_text: &str,
        context_pressure: f64,
    ) -> LoopIntervention {
        self.guard
            .record_tool_iteration(tool_calls, output_text, context_pressure)
    }

    pub fn intervention_message(
        intervention: LoopIntervention,
        fingerprint: &TurnFingerprint,
        context_pressure: f64,
    ) -> Option<String> {
        ToolLoopGuard::intervention_message(intervention, fingerprint, context_pressure)
    }
}

impl Default for ToolCallLoop {
    fn default() -> Self {
        Self::new()
    }
}
