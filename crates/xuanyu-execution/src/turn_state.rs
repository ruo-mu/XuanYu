use serde::{Deserialize, Serialize};

/// REFACTOR_PLAN §11.2 — main turn lifecycle phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnPhase {
    Idle,
    Planning,
    AssemblingContext,
    ModelCall,
    ToolExecution,
    Finalizing,
    Complete,
    Failed,
}

impl TurnPhase {
    pub fn as_event_name(self) -> &'static str {
        match self {
            Self::Idle => "turn.idle",
            Self::Planning => "turn.planning",
            Self::AssemblingContext => "turn.assembling_context",
            Self::ModelCall => "turn.model_call",
            Self::ToolExecution => "turn.tool_execution",
            Self::Finalizing => "turn.finalizing",
            Self::Complete => "turn.complete",
            Self::Failed => "turn.failed",
        }
    }

    pub fn allows_tool_calls(self) -> bool {
        matches!(self, Self::ModelCall | Self::ToolExecution)
    }
}
