use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use xuanyu_protocol::{AppEvent, Envelope};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventLane {
    Global,
    Session,
    Turn,
    Worker,
    Audio,
}

impl EventLane {
    pub fn classify(event: &AppEvent) -> Self {
        match event {
            AppEvent::TurnStarted { .. }
            | AppEvent::TurnDelta { .. }
            | AppEvent::TurnFinished { .. }
            | AppEvent::TurnFailed { .. }
            | AppEvent::TurnUsage { .. }
            | AppEvent::TurnReasoning { .. }
            | AppEvent::AgentToolRequested { .. }
            | AppEvent::AgentToolProgress { .. }
            | AppEvent::AgentToolResult { .. }
            | AppEvent::TaskEvent { .. }
            | AppEvent::TaskResult { .. }
            | AppEvent::RunPlan { .. } => EventLane::Turn,
            AppEvent::AsrPartial { .. } | AppEvent::AudioStateUpdated { .. } => EventLane::Audio,
            AppEvent::WorkerHealth { .. } | AppEvent::RuntimeModuleUpdated { .. } => {
                EventLane::Worker
            }
            AppEvent::SessionCreated { .. }
            | AppEvent::SessionUpdated { .. }
            | AppEvent::SessionHistory { .. }
            | AppEvent::SessionTitleStatus { .. }
            | AppEvent::ModelRouteSelected { .. }
            | AppEvent::TeamAgentStarted { .. }
            | AppEvent::TeamAgentProgress { .. }
            | AppEvent::TeamAgentCompleted { .. }
            | AppEvent::TeamAgentFailed { .. }
            | AppEvent::TaskSnapshot { .. } => EventLane::Session,
            _ => EventLane::Global,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            EventLane::Global => "global",
            EventLane::Session => "session",
            EventLane::Turn => "turn",
            EventLane::Worker => "worker",
            EventLane::Audio => "audio",
        }
    }

    pub fn buffer_size(self) -> usize {
        match self {
            EventLane::Turn => 2048,
            EventLane::Audio => 512,
            EventLane::Session => 512,
            EventLane::Worker => 128,
            EventLane::Global => 256,
        }
    }
}

#[derive(Clone)]
pub struct EventLaneHub {
    lanes: Arc<HashMap<EventLane, tokio::sync::broadcast::Sender<Envelope<AppEvent>>>>,
}

impl EventLaneHub {
    pub fn new() -> Self {
        let mut lanes = HashMap::new();
        for lane in [
            EventLane::Global,
            EventLane::Session,
            EventLane::Turn,
            EventLane::Worker,
            EventLane::Audio,
        ] {
            let (sender, _) = tokio::sync::broadcast::channel(lane.buffer_size());
            lanes.insert(lane, sender);
        }
        Self {
            lanes: Arc::new(lanes),
        }
    }

    pub fn publish(&self, lane: EventLane, envelope: Envelope<AppEvent>) {
        if let Some(sender) = self.lanes.get(&lane) {
            let _ = sender.send(envelope);
        }
    }

    pub fn subscribe(&self, lane: EventLane) -> tokio::sync::broadcast::Receiver<Envelope<AppEvent>> {
        self.lanes
            .get(&lane)
            .map(|sender| sender.subscribe())
            .unwrap_or_else(|| {
                let (sender, receiver) = tokio::sync::broadcast::channel(16);
                drop(sender);
                receiver
            })
    }
}

impl Default for EventLaneHub {
    fn default() -> Self {
        Self::new()
    }
}
