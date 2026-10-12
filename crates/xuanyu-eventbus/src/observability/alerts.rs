use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AlertSeverity {
    Info,
    Warning,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlertEvent {
    pub id: String,
    pub severity: AlertSeverity,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

const MAX_ALERTS: usize = 5000;

#[derive(Debug, Default)]
pub struct AlertBuffer {
    events: Vec<AlertEvent>,
}

impl AlertBuffer {
    pub fn push(
        &mut self,
        severity: AlertSeverity,
        title: impl Into<String>,
        payload: Option<serde_json::Value>,
        trace_id: Option<String>,
    ) -> AlertEvent {
        let event = AlertEvent {
            id: format!("alert_{}", Uuid::new_v4()),
            severity,
            title: title.into(),
            payload,
            trace_id,
            created_at: Utc::now(),
        };
        self.events.push(event.clone());
        if self.events.len() > MAX_ALERTS {
            let drain = self.events.len().saturating_sub(MAX_ALERTS);
            self.events.drain(0..drain);
        }
        event
    }

    pub fn list(&self, severity: Option<AlertSeverity>) -> Vec<AlertEvent> {
        match severity {
            Some(filter) => self
                .events
                .iter()
                .filter(|event| event.severity == filter)
                .cloned()
                .collect(),
            None => self.events.clone(),
        }
    }

    pub fn recent(&self, limit: usize) -> Vec<AlertEvent> {
        self.events
            .iter()
            .rev()
            .take(limit.max(1))
            .cloned()
            .collect()
    }
}
