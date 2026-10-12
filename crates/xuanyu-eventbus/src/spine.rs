use crate::StoredEvent;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use xuanyu_protocol::{AppEvent, Envelope, SessionId, TurnId};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventPublishContext {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
}

impl EventPublishContext {
    pub fn from_event(event: &AppEvent) -> Self {
        match event {
            AppEvent::SessionCreated { session } | AppEvent::SessionUpdated { session } => Self {
                session_id: Some(session.id.0.clone()),
                turn_id: None,
                trace_id: None,
            },
            AppEvent::SessionHistory { session_id, .. } => Self {
                session_id: Some(session_id.0.clone()),
                turn_id: None,
                trace_id: None,
            },
            AppEvent::SessionTitleStatus { session_id, .. } => Self {
                session_id: Some(session_id.0.clone()),
                turn_id: None,
                trace_id: None,
            },
            AppEvent::TurnStarted { session_id, turn_id }
            | AppEvent::TurnDelta { session_id, turn_id, .. }
            | AppEvent::TurnFinished { session_id, turn_id }
            | AppEvent::TurnFailed { session_id, turn_id, .. }
            | AppEvent::TurnUsage { session_id, turn_id, .. }
            | AppEvent::TurnReasoning { session_id, turn_id, .. }
            | AppEvent::AgentToolRequested { session_id, turn_id, .. }
            | AppEvent::AgentToolProgress { session_id, turn_id, .. }
            | AppEvent::AgentToolResult { session_id, turn_id, .. } => Self {
                session_id: Some(session_id.0.clone()),
                turn_id: Some(turn_id.0.clone()),
                trace_id: None,
            },
            AppEvent::AudioStateUpdated { session_id, .. }
            | AppEvent::AsrPartial { session_id, .. }
            | AppEvent::ModelRouteSelected { session_id, .. }
            | AppEvent::RunPlan { session_id, .. } => Self {
                session_id: Some(session_id.0.clone()),
                turn_id: None,
                trace_id: None,
            },
            AppEvent::TaskEvent { session_id, task_id, .. } => Self {
                session_id: Some(session_id.clone()),
                turn_id: Some(task_id.clone()),
                trace_id: None,
            },
            AppEvent::TaskResult { session_id, task_id, .. } => Self {
                session_id: Some(session_id.clone()),
                turn_id: Some(task_id.clone()),
                trace_id: None,
            },
            AppEvent::TeamAgentStarted { session_id, .. }
            | AppEvent::TeamAgentProgress { session_id, .. }
            | AppEvent::TeamAgentCompleted { session_id, .. }
            | AppEvent::TeamAgentFailed { session_id, .. }
            | AppEvent::TaskSnapshot { session_id, .. } => Self {
                session_id: session_id.clone(),
                turn_id: None,
                trace_id: None,
            },
            AppEvent::WorkerHealth { .. } => Self::default(),
            AppEvent::RuntimeModuleUpdated { .. } => Self::default(),
            _ => Self::default(),
        }
    }

    pub fn for_session_turn(session_id: &SessionId, turn_id: &TurnId) -> Self {
        Self {
            session_id: Some(session_id.0.clone()),
            turn_id: Some(turn_id.0.clone()),
            trace_id: None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lane: Option<String>,
}

impl EventFilter {
    pub fn matches_envelope(&self, envelope: &Envelope<AppEvent>) -> bool {
        let session_id = envelope.meta.get("sessionId").map(String::as_str);
        let turn_id = envelope.meta.get("turnId").map(String::as_str);
        let lane = envelope.meta.get("lane").map(String::as_str);
        self.matches_scope(session_id, turn_id, lane, &envelope.kind)
    }

    pub fn matches_stored(&self, stored: &StoredEvent) -> bool {
        self.matches_scope(
            stored.session_id.as_deref(),
            stored.turn_id.as_deref(),
            stored.lane.as_deref(),
            &stored.kind,
        )
    }

    fn matches_scope(
        &self,
        session_id: Option<&str>,
        turn_id: Option<&str>,
        lane: Option<&str>,
        kind: &str,
    ) -> bool {
        if let Some(expected) = self.session_id.as_deref() {
            if session_id != Some(expected) {
                return false;
            }
        }
        if let Some(expected) = self.turn_id.as_deref() {
            if turn_id != Some(expected) {
                return false;
            }
        }
        if !self.kinds.is_empty() && !self.kinds.iter().any(|item| item == kind) {
            return false;
        }
        if let Some(expected_lane) = self.lane.as_deref() {
            if lane != Some(expected_lane) {
                return false;
            }
        }
        true
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeadLetterReason {
    SubscriberLagged,
    FilterMismatch,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeadLetterRecord {
    pub reason: DeadLetterReason,
    pub detail: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

pub struct FilteredReceiver {
    inner: broadcast::Receiver<Envelope<AppEvent>>,
    filter: EventFilter,
    dead_letters: Arc<RwLock<VecDeque<DeadLetterRecord>>>,
}

impl FilteredReceiver {
    pub fn new(
        inner: broadcast::Receiver<Envelope<AppEvent>>,
        filter: EventFilter,
        dead_letters: Arc<RwLock<VecDeque<DeadLetterRecord>>>,
    ) -> Self {
        Self {
            inner,
            filter,
            dead_letters,
        }
    }

    pub async fn recv(&mut self) -> Result<Envelope<AppEvent>, broadcast::error::RecvError> {
        loop {
            match self.inner.recv().await {
                Ok(envelope) => {
                    if self.filter.matches_envelope(&envelope) {
                        return Ok(envelope);
                    }
                }
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    self.record_dead_letter(
                        DeadLetterReason::SubscriberLagged,
                        format!("subscriber lagged behind by {skipped} events"),
                    )
                    .await;
                    return Err(broadcast::error::RecvError::Lagged(skipped));
                }
                Err(err) => return Err(err),
            }
        }
    }

    async fn record_dead_letter(&self, reason: DeadLetterReason, detail: String) {
        let mut letters = self.dead_letters.write().await;
        if letters.len() >= 256 {
            letters.pop_front();
        }
        letters.push_back(DeadLetterRecord {
            reason,
            detail,
            created_at: chrono::Utc::now(),
        });
    }
}
